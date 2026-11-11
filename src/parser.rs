//! Recursive-descent parser: tokens -> AST. See GRAMMAR.md.

use crate::ast::*;
use crate::lexer::{self, Error, Span, StrPart as LexPart, Tok, Token};

type PResult<T> = Result<T, Error>;

#[allow(dead_code)]
pub fn parse_file(src: &str) -> PResult<File> {
    parse_file_at(src, Span { line: 1, col: 1 })
}

/// Parses a file whose first line has the given position.
pub fn parse_file_at(src: &str, origin: Span) -> PResult<File> {
    let toks = lexer::lex_at(src, origin)?;
    Parser { toks, pos: 0 }.file()
}

#[allow(dead_code)]
pub fn parse_expr(src: &str) -> PResult<Expr> {
    parse_expr_at(src, Span { line: 1, col: 1 })
}

fn parse_expr_at(src: &str, origin: Span) -> PResult<Expr> {
    let toks = lexer::lex_at(src, origin)?;
    let mut p = Parser { toks, pos: 0 };
    p.skip_nl();
    let e = p.expr()?;
    p.skip_nl();
    if *p.peek() != Tok::Eof {
        return p.err(format!("unexpected {} after expression", p.describe()));
    }
    Ok(e)
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

fn mk(kind: ExprKind, span: Span) -> Expr {
    Expr { kind, span }
}

const ASSIGN_OPS: &[&str] = &["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>="];

impl Parser {
    // ---- token helpers ----

    fn peek(&self) -> &Tok {
        self.peek_at(0)
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn span(&self) -> Span {
        self.toks[self.pos.min(self.toks.len() - 1)].span
    }

    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("identifier `{s}`"),
            Tok::Int(s) | Tok::Float(s) => format!("number `{s}`"),
            Tok::Str(_) => "string literal".to_string(),
            Tok::Char(_) => "character literal".to_string(),
            Tok::Kw(k) => format!("keyword `{k}`"),
            Tok::P(p) => format!("`{p}`"),
            Tok::Newline => "end of line".to_string(),
            Tok::Eof => "end of file".to_string(),
        }
    }

    fn err<T>(&self, msg: impl Into<String>) -> PResult<T> {
        Err(Error { msg: msg.into(), span: self.span() })
    }

    fn is_p(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::P(p) if *p == s)
    }

    fn is_kw(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Kw(k) if *k == s)
    }

    fn eat_p(&mut self, s: &str) -> bool {
        let hit = self.is_p(s);
        if hit {
            self.bump();
        }
        hit
    }

    fn eat_kw(&mut self, s: &str) -> bool {
        let hit = self.is_kw(s);
        if hit {
            self.bump();
        }
        hit
    }

    fn expect_p(&mut self, s: &str) -> PResult<()> {
        if self.eat_p(s) {
            Ok(())
        } else {
            self.err(format!("expected `{s}`, found {}", self.describe()))
        }
    }

    fn expect_kw(&mut self, s: &str) -> PResult<()> {
        if self.eat_kw(s) {
            Ok(())
        } else {
            self.err(format!("expected `{s}`, found {}", self.describe()))
        }
    }

    fn ident(&mut self) -> PResult<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            _ => self.err(format!("expected identifier, found {}", self.describe())),
        }
    }

    /// Skips newlines; returns whether there was at least one.
    fn skip_nl(&mut self) -> bool {
        let mut any = false;
        while *self.peek() == Tok::Newline {
            self.bump();
            any = true;
        }
        any
    }

    /// End of statement: a newline, end of file, or a closing brace (not consumed).
    fn expect_nl(&mut self) -> PResult<()> {
        match self.peek() {
            Tok::Newline => {
                self.bump();
                Ok(())
            }
            Tok::Eof => Ok(()),
            Tok::P("}") => Ok(()),
            _ => self.err(format!("expected end of line, found {}", self.describe())),
        }
    }

    /// Comma-separated list up to `close`. Inside braces a newline also separates items.
    fn list<T>(
        &mut self,
        close: &'static str,
        mut item: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<Vec<T>> {
        let mut items = Vec::new();
        loop {
            self.skip_nl();
            if self.eat_p(close) {
                break;
            }
            items.push(item(self)?);
            let had_nl = self.skip_nl();
            if self.eat_p(",") {
                continue;
            }
            if self.eat_p(close) {
                break;
            }
            if had_nl && close == "}" {
                continue;
            }
            return self.err(format!("expected `,` or `{close}`, found {}", self.describe()));
        }
        Ok(items)
    }

    // ---- file and declarations ----

    fn file(&mut self) -> PResult<File> {
        self.skip_nl();
        let mut profile = None;
        if self.is_p("@") && *self.peek_at(1) == Tok::Ident("profile".into()) && *self.peek_at(2) == Tok::P("(") {
            self.bump();
            self.bump();
            self.bump();
            profile = Some(self.ident()?);
            self.expect_p(")")?;
            self.expect_nl()?;
        }
        let mut imports = Vec::new();
        loop {
            self.skip_nl();
            if !self.eat_kw("import") {
                break;
            }
            imports.push(self.import()?);
            self.expect_nl()?;
        }
        let mut decls = Vec::new();
        loop {
            self.skip_nl();
            if *self.peek() == Tok::Eof {
                break;
            }
            decls.push(self.decl()?);
            self.expect_nl()?;
        }
        Ok(File { profile, imports, decls, modules: Vec::new() })
    }

    fn import(&mut self) -> PResult<Import> {
        let first = self.ident()?;
        let (origin, path) = if let Tok::Str(parts) = self.peek().clone() {
            self.bump();
            match parts.as_slice() {
                [LexPart::Lit(s)] => (Some(first), s.clone()),
                _ => return self.err("import path cannot use interpolation"),
            }
        } else {
            let mut path = first;
            while self.eat_p(".") {
                path.push('.');
                path.push_str(&self.ident()?);
            }
            (None, path)
        };
        let alias = if self.eat_kw("as") { Some(self.ident()?) } else { None };
        // `import c "math.h" as m { fn sqrt(x: f64) -> f64 }`
        let decls = if origin.is_some() && self.is_p("{") { self.decl_body()? } else { Vec::new() };
        Ok(Import { origin, path, alias, decls })
    }

    fn starts_decl(&self) -> bool {
        match self.peek() {
            Tok::P("@") => true,
            Tok::Kw("fn") | Tok::Kw("type") => matches!(self.peek_at(1), Tok::Ident(_)),
            Tok::Kw(k) => matches!(*k, "pub" | "struct" | "enum" | "trait" | "impl"),
            _ => false,
        }
    }

    fn decl(&mut self) -> PResult<Decl> {
        let span = self.span();
        let mut attrs = Vec::new();
        while self.eat_p("@") {
            let name = self.ident()?;
            let args = if self.eat_p("(") { self.list(")", |p| p.expr())? } else { Vec::new() };
            attrs.push(Attr { name, args });
            self.skip_nl();
        }
        let public = self.eat_kw("pub");
        let kind = match self.peek() {
            Tok::Kw("fn") => DeclKind::Fn(self.fn_decl()?),
            Tok::Kw("struct") => DeclKind::Struct(self.struct_decl()?),
            Tok::Kw("enum") => DeclKind::Enum(self.enum_decl()?),
            Tok::Kw("trait") => DeclKind::Trait(self.trait_decl()?),
            Tok::Kw("impl") => DeclKind::Impl(self.impl_decl()?),
            Tok::Kw("let") => {
                self.bump();
                let name = self.ident()?;
                let ty = if self.eat_p(":") { Some(self.ty()?) } else { None };
                self.expect_p("=")?;
                DeclKind::Const { name, ty, value: self.expr()? }
            }
            Tok::Kw("type") => {
                self.bump();
                let name = self.ident()?;
                let generics = self.generics()?;
                self.expect_p("=")?;
                DeclKind::TypeAlias { name, generics, ty: self.ty()? }
            }
            _ => return self.err(format!("expected a declaration, found {}", self.describe())),
        };
        Ok(Decl { attrs, public, kind, span })
    }

    fn generics(&mut self) -> PResult<Vec<GenParam>> {
        if !self.eat_p("[") {
            return Ok(Vec::new());
        }
        self.list("]", |p| {
            if p.eat_kw("region") {
                return Ok(GenParam::Region(p.ident()?));
            }
            if p.eat_kw("comptime") {
                let name = p.ident()?;
                p.expect_p(":")?;
                return Ok(GenParam::Comptime { name, ty: p.ty()? });
            }
            let name = p.ident()?;
            let bounds = if p.eat_p(":") { p.bounds()? } else { Vec::new() };
            Ok(GenParam::Type { name, bounds })
        })
    }

    fn bounds(&mut self) -> PResult<Vec<Type>> {
        // A bound is a trait: a `?` after it belongs to what surrounds it,
        // as in `dyn Shape?`.
        let mut bounds = vec![self.ty_base()?];
        while self.eat_p("+") {
            bounds.push(self.ty_base()?);
        }
        Ok(bounds)
    }

    fn mode(&mut self) -> Mode {
        if self.eat_kw("mut") {
            Mode::Mut
        } else if self.eat_kw("own") {
            Mode::Own
        } else {
            Mode::Let
        }
    }

    fn param(&mut self) -> PResult<Param> {
        let comptime = self.eat_kw("comptime");
        let self_mode = matches!(self.peek(), Tok::Kw("mut") | Tok::Kw("own")) && *self.peek_at(1) == Tok::Kw("self");
        if self_mode || self.is_kw("self") {
            let mode = self.mode();
            self.expect_kw("self")?;
            return Ok(Param { comptime, name: "self".into(), mode, ty: None, default: None });
        }
        let name = self.ident()?;
        let (mut mode, mut ty) = (Mode::Let, None);
        if self.eat_p(":") {
            mode = self.mode();
            ty = Some(self.ty()?);
        }
        let default = if self.eat_p("=") { Some(self.expr()?) } else { None };
        Ok(Param { comptime, name, mode, ty, default })
    }

    /// `("->" "ref"? type)? ("!" type)?`
    fn ret(&mut self) -> PResult<(Option<Type>, bool, Option<Type>)> {
        let (mut ret, mut ret_ref) = (None, false);
        if self.eat_p("->") {
            // `ref T` is a by-reference return, `ref[r] T` / `ref mut T` is a reference type.
            if self.is_kw("ref") && !matches!(self.peek_at(1), Tok::P("[") | Tok::Kw("mut")) {
                self.bump();
                ret_ref = true;
            }
            ret = Some(self.ty()?);
        }
        let err = if self.eat_p("!") { Some(self.ty()?) } else { None };
        Ok((ret, ret_ref, err))
    }

    fn fn_decl(&mut self) -> PResult<FnDecl> {
        self.expect_kw("fn")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_p("(")?;
        let params = self.list(")", |p| p.param())?;
        let (ret, ret_ref, err) = self.ret()?;
        let mut where_ = Vec::new();
        if self.eat_kw("where") {
            where_.push(self.expr()?);
            while self.eat_p(",") {
                where_.push(self.expr()?);
            }
        }
        // The body may open on the next line.
        let save = self.pos;
        self.skip_nl();
        let body = if self.is_p("{") {
            let span = self.span();
            Some(mk(ExprKind::Block(self.block()?), span))
        } else {
            self.pos = save;
            if self.eat_p("=") {
                Some(self.expr()?)
            } else {
                None
            }
        };
        Ok(FnDecl { name, generics, params, ret, ret_ref, err, where_, body })
    }

    fn struct_decl(&mut self) -> PResult<StructDecl> {
        self.expect_kw("struct")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_p("{")?;
        let fields = self.list("}", |p| {
            let public = p.eat_kw("pub");
            let name = p.ident()?;
            p.expect_p(":")?;
            let ty = p.ty()?;
            let default = if p.eat_p("=") { Some(p.expr()?) } else { None };
            Ok(Field { public, name, ty, default })
        })?;
        Ok(StructDecl { name, generics, fields })
    }

    fn enum_decl(&mut self) -> PResult<EnumDecl> {
        self.expect_kw("enum")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        self.expect_p("{")?;
        let variants = self.list("}", |p| {
            let name = p.ident()?;
            let fields = if p.eat_p("(") {
                p.list(")", |p| {
                    let named = matches!(p.peek(), Tok::Ident(_)) && *p.peek_at(1) == Tok::P(":");
                    let label = if named {
                        let l = p.ident()?;
                        p.bump();
                        Some(l)
                    } else {
                        None
                    };
                    Ok((label, p.ty()?))
                })?
            } else {
                Vec::new()
            };
            let value = if p.eat_p("=") { Some(p.expr()?) } else { None };
            Ok(Variant { name, fields, value })
        })?;
        Ok(EnumDecl { name, generics, variants })
    }

    fn decl_body(&mut self) -> PResult<Vec<Decl>> {
        self.expect_p("{")?;
        let mut decls = Vec::new();
        loop {
            self.skip_nl();
            if self.eat_p("}") {
                break;
            }
            decls.push(self.decl()?);
            self.expect_nl()?;
        }
        Ok(decls)
    }

    fn trait_decl(&mut self) -> PResult<TraitDecl> {
        self.expect_kw("trait")?;
        let name = self.ident()?;
        let generics = self.generics()?;
        let supers = if self.eat_p(":") { self.bounds()? } else { Vec::new() };
        let methods = self.decl_body()?;
        Ok(TraitDecl { name, generics, supers, methods })
    }

    fn impl_decl(&mut self) -> PResult<ImplDecl> {
        self.expect_kw("impl")?;
        let generics = self.generics()?;
        let first = self.ty()?;
        let (trait_, ty) = if self.eat_kw("for") { (Some(first), self.ty()?) } else { (None, first) };
        let decls = self.decl_body()?;
        Ok(ImplDecl { generics, trait_, ty, decls })
    }

    // ---- types ----

    fn starts_type(&self) -> bool {
        match self.peek() {
            Tok::Ident(_) | Tok::P("[") | Tok::P("(") => true,
            Tok::Kw(k) => matches!(*k, "fn" | "ref" | "dyn" | "Self" | "soa" | "mut" | "type"),
            _ => false,
        }
    }

    fn ty(&mut self) -> PResult<Type> {
        let mut t = self.ty_base()?;
        while self.eat_p("?") {
            t = Type::Optional(Box::new(t));
        }
        Ok(t)
    }

    fn ty_base(&mut self) -> PResult<Type> {
        match self.peek().clone() {
            Tok::P("[") => self.slice_or_array(),
            Tok::P("(") => {
                self.bump();
                Ok(Type::Tuple(self.list(")", |p| p.ty())?))
            }
            Tok::Kw("soa") => {
                self.bump();
                if !self.is_p("[") {
                    return self.err("expected `[` after `soa`");
                }
                Ok(Type::Soa(Box::new(self.slice_or_array()?)))
            }
            Tok::Kw("fn") => {
                self.bump();
                self.expect_p("(")?;
                let params = self.list(")", |p| {
                    let mode = p.mode();
                    Ok((mode, p.ty()?))
                })?;
                let (ret, _, err) = self.ret()?;
                Ok(Type::Fn { params, ret: ret.map(Box::new), err: err.map(Box::new) })
            }
            Tok::Kw("ref") => {
                self.bump();
                let mut region = None;
                // `ref[r] T` names a region; `ref [T]` is a reference to a slice.
                if self.is_p("[") && matches!(self.peek_at(1), Tok::Ident(_)) && *self.peek_at(2) == Tok::P("]") {
                    let save = self.pos;
                    self.bump();
                    let name = self.ident()?;
                    self.bump();
                    if self.starts_type() {
                        region = Some(name);
                    } else {
                        self.pos = save;
                    }
                }
                let mutable = self.eat_kw("mut");
                Ok(Type::Ref { region, mutable, ty: Box::new(self.ty()?) })
            }
            Tok::Kw("dyn") => {
                self.bump();
                Ok(Type::Dyn(self.bounds()?))
            }
            Tok::Kw("Self") => {
                self.bump();
                Ok(Type::SelfType)
            }
            Tok::Kw("type") => {
                self.bump();
                Ok(Type::TypeType)
            }
            Tok::Ident(first) => {
                self.bump();
                let mut path = vec![first];
                while self.is_p(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                    self.bump();
                    path.push(self.ident()?);
                }
                let args = if self.eat_p("[") {
                    self.list("]", |p| {
                        if matches!(p.peek(), Tok::Int(_)) {
                            Ok(GenArg::Expr(p.expr()?))
                        } else {
                            Ok(GenArg::Type(p.ty()?))
                        }
                    })?
                } else {
                    Vec::new()
                };
                Ok(Type::Path { path, args })
            }
            _ => self.err(format!("expected a type, found {}", self.describe())),
        }
    }

    fn slice_or_array(&mut self) -> PResult<Type> {
        self.expect_p("[")?;
        let inner = Box::new(self.ty()?);
        if self.eat_p(";") {
            let len = Box::new(self.expr()?);
            self.expect_p("]")?;
            return Ok(Type::Array(inner, len));
        }
        self.expect_p("]")?;
        Ok(Type::Slice(inner))
    }

    // ---- statements ----

    fn block(&mut self) -> PResult<Block> {
        self.expect_p("{")?;
        let mut stmts = Vec::new();
        loop {
            self.skip_nl();
            if self.eat_p("}") {
                return Ok(Block { stmts, tail: None });
            }
            if self.is_kw("let") || self.is_kw("var") {
                let mutable = self.bump() == Tok::Kw("var");
                let pat = self.pattern()?;
                let ty = if self.eat_p(":") { Some(self.ty()?) } else { None };
                let value = if self.eat_p("=") { Some(self.expr()?) } else { None };
                stmts.push(Stmt::Let { mutable, pat, ty, value });
                self.expect_nl()?;
                continue;
            }
            if self.eat_kw("defer") {
                stmts.push(Stmt::Defer(self.expr()?));
                self.expect_nl()?;
                continue;
            }
            if self.starts_decl() {
                stmts.push(Stmt::Decl(self.decl()?));
                self.expect_nl()?;
                continue;
            }
            let e = self.expr()?;
            if let Tok::P(op) = *self.peek() {
                if ASSIGN_OPS.contains(&op) {
                    self.bump();
                    let value = self.expr()?;
                    stmts.push(Stmt::Assign { target: e, op, value });
                    self.expect_nl()?;
                    continue;
                }
            }
            // An expression right before the closing brace is the block's value.
            let had_nl = self.skip_nl();
            if self.eat_p("}") {
                return Ok(Block { stmts, tail: Some(Box::new(e)) });
            }
            if !had_nl {
                return self.err(format!("expected end of line, found {}", self.describe()));
            }
            stmts.push(Stmt::Expr(e));
        }
    }

    // ---- expressions ----

    fn starts_expr(&self) -> bool {
        match self.peek() {
            Tok::Ident(_) | Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::Char(_) => true,
            Tok::Kw(k) => matches!(
                *k,
                "true" | "false" | "self" | "Self" | "if" | "match" | "for" | "while" | "loop" | "fn"
                    | "comptime" | "unsafe" | "scope" | "with" | "return" | "break" | "continue"
            ),
            Tok::P(p) => matches!(*p, "(" | "[" | "-" | "!" | "~"),
            _ => false,
        }
    }

    fn expr(&mut self) -> PResult<Expr> {
        let mut e = self.binary(2)?;
        while self.eat_kw("catch") {
            let span = e.span;
            let binding = if matches!(self.peek(), Tok::Ident(_)) { Some(self.ident()?) } else { None };
            let handler = self.block()?;
            e = mk(ExprKind::Catch { expr: Box::new(e), binding, handler }, span);
        }
        Ok(e)
    }

    fn binop(&self) -> Option<(&'static str, u8)> {
        let Tok::P(p) = *self.peek() else { return None };
        let prec = match p {
            "||" => 2,
            "&&" => 3,
            "==" | "!=" | "<" | "<=" | ">" | ">=" => 4,
            ".." | "..=" => 5,
            "|" => 6,
            "^" => 7,
            "&" => 8,
            "<<" | ">>" => 9,
            "+" | "-" | "+%" | "-%" | "+|" | "-|" => 10,
            "*" | "/" | "%" | "*%" | "*|" => 11,
            _ => return None,
        };
        Some((p, prec))
    }

    fn binary(&mut self, min: u8) -> PResult<Expr> {
        let mut lhs = self.cast()?;
        while let Some((op, prec)) = self.binop() {
            if prec < min {
                break;
            }
            let span = lhs.span;
            self.bump();
            if prec == 5 {
                let hi = if self.starts_expr() { Some(Box::new(self.binary(6)?)) } else { None };
                lhs = mk(ExprKind::Range { lo: Some(Box::new(lhs)), hi, inclusive: op == "..=" }, span);
                continue;
            }
            let rhs = self.binary(prec + 1)?;
            lhs = mk(ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span);
            if prec == 4 && matches!(self.binop(), Some((_, 4))) {
                return self.err("comparison operators cannot be chained; use `&&`");
            }
        }
        Ok(lhs)
    }

    fn cast(&mut self) -> PResult<Expr> {
        let mut e = self.unary()?;
        while self.eat_kw("as") {
            let span = e.span;
            let ty = self.ty_base()?;
            e = mk(ExprKind::Cast { expr: Box::new(e), ty }, span);
        }
        Ok(e)
    }

    fn unary(&mut self) -> PResult<Expr> {
        if let Tok::P(op) = *self.peek() {
            if matches!(op, "-" | "!" | "~") {
                let span = self.span();
                self.bump();
                let expr = Box::new(self.unary()?);
                return Ok(mk(ExprKind::Unary { op, expr }, span));
            }
        }
        self.postfix()
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            let span = e.span;
            if self.eat_p("(") {
                let args = self.list(")", |p| {
                    let labeled = matches!(p.peek(), Tok::Ident(_)) && *p.peek_at(1) == Tok::P(":");
                    let label = if labeled {
                        let l = p.ident()?;
                        p.bump();
                        Some(l)
                    } else {
                        None
                    };
                    let mode = p.mode();
                    Ok(Arg { label, mode, value: p.expr()? })
                })?;
                e = mk(ExprKind::Call { callee: Box::new(e), args }, span);
            } else if self.eat_p("[") {
                let args = self.list("]", |p| p.expr())?;
                e = mk(ExprKind::Index { base: Box::new(e), args }, span);
            } else if self.eat_p(".") {
                let name = match self.peek().clone() {
                    Tok::Ident(s) | Tok::Int(s) => {
                        self.bump();
                        s
                    }
                    _ => return self.err(format!("expected field name, found {}", self.describe())),
                };
                e = mk(ExprKind::Field { base: Box::new(e), name }, span);
            } else if self.eat_p("?") {
                e = mk(ExprKind::Try(Box::new(e)), span);
            } else {
                return Ok(e);
            }
        }
    }

    fn cond(&mut self) -> PResult<Cond> {
        if self.eat_kw("let") {
            let pat = self.pattern()?;
            self.expect_p("=")?;
            return Ok(Cond::Let(pat, self.expr()?));
        }
        Ok(Cond::Expr(self.expr()?))
    }

    fn if_expr(&mut self) -> PResult<Expr> {
        let span = self.span();
        self.expect_kw("if")?;
        let cond = Box::new(self.cond()?);
        let then = self.block()?;
        let els = if self.eat_kw("else") {
            if self.is_kw("if") {
                Some(Box::new(self.if_expr()?))
            } else {
                let span = self.span();
                Some(Box::new(mk(ExprKind::Block(self.block()?), span)))
            }
        } else {
            None
        };
        Ok(mk(ExprKind::If { cond, then, els }, span))
    }

    fn opt_operand(&mut self) -> PResult<Option<Box<Expr>>> {
        Ok(if self.starts_expr() { Some(Box::new(self.expr()?)) } else { None })
    }

    fn primary(&mut self) -> PResult<Expr> {
        let span = self.span();
        let kind = match self.peek().clone() {
            Tok::Int(s) => {
                self.bump();
                ExprKind::Int(s)
            }
            Tok::Float(s) => {
                self.bump();
                ExprKind::Float(s)
            }
            Tok::Char(c) => {
                self.bump();
                ExprKind::Char(c)
            }
            Tok::Str(parts) => {
                self.bump();
                let mut out = Vec::new();
                for part in parts {
                    out.push(match part {
                        LexPart::Lit(s) => StrPart::Lit(s),
                        LexPart::Expr(src, at) => StrPart::Expr(parse_expr_at(&src, at)?),
                    });
                }
                ExprKind::Str(out)
            }
            Tok::Ident(s) => {
                self.bump();
                ExprKind::Path(s)
            }
            Tok::Kw(k) => match k {
                "true" | "false" => {
                    self.bump();
                    ExprKind::Bool(k == "true")
                }
                "self" | "Self" => {
                    self.bump();
                    ExprKind::Path(k.to_string())
                }
                "if" => return self.if_expr(),
                "match" => {
                    self.bump();
                    let scrutinee = Box::new(self.expr()?);
                    self.expect_p("{")?;
                    let arms = self.list("}", |p| {
                        let pat = p.pattern()?;
                        let guard = if p.eat_kw("if") { Some(p.expr()?) } else { None };
                        p.expect_p("=>")?;
                        Ok(Arm { pat, guard, body: p.expr()? })
                    })?;
                    ExprKind::Match { scrutinee, arms }
                }
                "for" => {
                    self.bump();
                    let pat = self.pattern()?;
                    self.expect_kw("in")?;
                    let iter = Box::new(self.expr()?);
                    ExprKind::For { pat, iter, body: self.block()? }
                }
                "while" => {
                    self.bump();
                    let cond = Box::new(self.cond()?);
                    ExprKind::While { cond, body: self.block()? }
                }
                "loop" => {
                    self.bump();
                    ExprKind::Loop(self.block()?)
                }
                "fn" => {
                    self.bump();
                    self.expect_p("(")?;
                    let params = self.list(")", |p| p.param())?;
                    let (ret, _, err) = self.ret()?;
                    ExprKind::Closure { params, ret, err, body: self.block()? }
                }
                "comptime" => {
                    self.bump();
                    let inner = if self.is_p("{") {
                        let span = self.span();
                        mk(ExprKind::Block(self.block()?), span)
                    } else {
                        // All of the expression that follows: `comptime a + b`.
                        self.expr()?
                    };
                    ExprKind::Comptime(Box::new(inner))
                }
                "unsafe" => {
                    self.bump();
                    ExprKind::Unsafe(self.block()?)
                }
                "scope" => {
                    self.bump();
                    let name = self.ident()?;
                    ExprKind::Scope { name, body: self.block()? }
                }
                "with" => {
                    self.bump();
                    let mut bindings = Vec::new();
                    loop {
                        let name = self.ident()?;
                        self.expect_p("=")?;
                        bindings.push((name, self.expr()?));
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    ExprKind::With { bindings, body: self.block()? }
                }
                "return" => {
                    self.bump();
                    ExprKind::Return(self.opt_operand()?)
                }
                "break" => {
                    self.bump();
                    ExprKind::Break(self.opt_operand()?)
                }
                "continue" => {
                    self.bump();
                    ExprKind::Continue
                }
                _ => return self.err(format!("expected an expression, found {}", self.describe())),
            },
            Tok::P("(") => {
                self.bump();
                let mut items = Vec::new();
                let mut trailing_comma = false;
                loop {
                    if self.eat_p(")") {
                        break;
                    }
                    items.push(self.expr()?);
                    trailing_comma = self.eat_p(",");
                    if !trailing_comma {
                        self.expect_p(")")?;
                        break;
                    }
                }
                if items.len() == 1 && !trailing_comma {
                    return Ok(items.pop().unwrap());
                }
                ExprKind::Tuple(items)
            }
            Tok::P("[") => {
                self.bump();
                if self.eat_p("]") {
                    ExprKind::Array(Vec::new())
                } else {
                    let first = self.expr()?;
                    if self.eat_p(";") {
                        let len = self.expr()?;
                        self.expect_p("]")?;
                        ExprKind::ArrayRepeat(Box::new(first), Box::new(len))
                    } else {
                        let mut items = vec![first];
                        if self.eat_p(",") {
                            items.extend(self.list("]", |p| p.expr())?);
                        } else {
                            self.expect_p("]")?;
                        }
                        ExprKind::Array(items)
                    }
                }
            }
            Tok::P("{") => ExprKind::Block(self.block()?),
            Tok::P(op @ (".." | "..=")) => {
                self.bump();
                let hi = if self.starts_expr() { Some(Box::new(self.binary(6)?)) } else { None };
                ExprKind::Range { lo: None, hi, inclusive: op == "..=" }
            }
            _ => return self.err(format!("expected an expression, found {}", self.describe())),
        };
        Ok(mk(kind, span))
    }

    // ---- patterns ----

    fn pattern(&mut self) -> PResult<Pattern> {
        let first = self.pattern_alt()?;
        if !self.is_p("|") {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.eat_p("|") {
            alts.push(self.pattern_alt()?);
        }
        Ok(Pattern::Or(alts))
    }

    fn pattern_literal(&mut self) -> PResult<Expr> {
        let span = self.span();
        if self.eat_p("-") {
            let inner = self.pattern_literal()?;
            return Ok(mk(ExprKind::Unary { op: "-", expr: Box::new(inner) }, span));
        }
        let kind = match self.bump() {
            Tok::Int(s) => ExprKind::Int(s),
            Tok::Float(s) => ExprKind::Float(s),
            Tok::Char(c) => ExprKind::Char(c),
            Tok::Kw("true") => ExprKind::Bool(true),
            Tok::Kw("false") => ExprKind::Bool(false),
            Tok::Str(parts) => match parts.as_slice() {
                [LexPart::Lit(s)] => ExprKind::Str(vec![StrPart::Lit(s.clone())]),
                _ => return Err(Error { msg: "patterns cannot use string interpolation".into(), span }),
            },
            _ => return Err(Error { msg: "expected a literal in pattern".into(), span }),
        };
        Ok(mk(kind, span))
    }

    fn pattern_alt(&mut self) -> PResult<Pattern> {
        match self.peek().clone() {
            Tok::Ident(s) if s == "_" => {
                self.bump();
                Ok(Pattern::Wildcard)
            }
            Tok::Int(_) | Tok::Float(_) | Tok::Str(_) | Tok::Char(_) | Tok::Kw("true") | Tok::Kw("false") | Tok::P("-") => {
                let lo = self.pattern_literal()?;
                if self.is_p("..") || self.is_p("..=") {
                    let inclusive = self.bump() == Tok::P("..=");
                    let has_hi = matches!(self.peek(), Tok::Int(_) | Tok::Float(_) | Tok::Char(_) | Tok::P("-"));
                    let hi = if has_hi { Some(Box::new(self.pattern_literal()?)) } else { None };
                    return Ok(Pattern::Range { lo: Box::new(lo), hi, inclusive });
                }
                Ok(Pattern::Literal(Box::new(lo)))
            }
            Tok::Kw("mut") => {
                self.bump();
                Ok(Pattern::Bind { mutable: true, name: self.ident()? })
            }
            Tok::Ident(first) => {
                self.bump();
                let mut path = vec![first];
                while self.eat_p(".") {
                    path.push(self.ident()?);
                }
                // By convention a capitalized or dotted name is a constructor, not a binding.
                let is_ctor = path.len() > 1 || path[0].chars().next().is_some_and(|c| c.is_uppercase());
                if self.eat_p("(") {
                    return Ok(Pattern::Ctor { path, args: self.list(")", |p| p.pattern())? });
                }
                if is_ctor && self.eat_p("{") {
                    let mut rest = false;
                    let fields = self.list("}", |p| {
                        if p.eat_p("..") {
                            rest = true;
                            return Ok(None);
                        }
                        let name = p.ident()?;
                        let pat = if p.eat_p(":") { Some(p.pattern()?) } else { None };
                        Ok(Some((name, pat)))
                    })?;
                    return Ok(Pattern::Struct { path, fields: fields.into_iter().flatten().collect(), rest });
                }
                if is_ctor {
                    return Ok(Pattern::Ctor { path, args: Vec::new() });
                }
                Ok(Pattern::Bind { mutable: false, name: path.pop().unwrap() })
            }
            Tok::P("(") => {
                self.bump();
                Ok(Pattern::Tuple(self.list(")", |p| p.pattern())?))
            }
            Tok::P("[") => {
                self.bump();
                let items = self.list("]", |p| if p.eat_p("..") { Ok(Pattern::Rest) } else { p.pattern() })?;
                Ok(Pattern::Slice(items))
            }
            _ => self.err(format!("expected a pattern, found {}", self.describe())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sx(src: &str) -> String {
        sexpr(&parse_expr(src).unwrap_or_else(|e| panic!("{src}: {}", e.msg)))
    }

    #[test]
    fn precedence() {
        assert_eq!(sx("1 + 2 * 3"), "(+ 1 (* 2 3))");
        assert_eq!(sx("(1 + 2) * 3"), "(* (+ 1 2) 3)");
        assert_eq!(sx("a - b - c"), "(- (- a b) c)");
        assert_eq!(sx("a || b && c == d"), "(|| a (&& b (== c d)))");
        assert_eq!(sx("-a * b"), "(* (- a) b)");
        assert_eq!(sx("a / n as f64"), "(/ a (as n))");
        assert_eq!(sx("a +% b *| c"), "(+% a (*| b c))");
        assert_eq!(sx("a | b & c ^ d"), "(| a (^ (& b c) d))");
    }

    #[test]
    fn postfix_and_ranges() {
        assert_eq!(sx("a.b(c)[d]?"), "(? (index (call (. a b) c) d))");
        assert_eq!(sx("t.0.1"), "(. (. t 0) 1)");
        assert_eq!(sx("0..n + 1"), "(.. 0 (+ n 1))");
        assert_eq!(sx("xs[..i]"), "(index xs (.. _ i))");
        assert_eq!(sx("xs[1..]"), "(index xs (.. 1 _))");
        assert_eq!(sx("(a, b)"), "(tuple a b)");
        assert_eq!(sx("(a)"), "a");
    }

    #[test]
    fn chained_comparison_is_rejected() {
        assert!(parse_expr("a < b < c").is_err());
    }

    #[test]
    fn newlines_end_statements_only_where_they_can() {
        let src = "fn f() {\n    let a = 1 +\n        2\n    let b = foo\n        .bar()\n        .baz()\n    -a\n}\n";
        let file = parse_file(src).unwrap_or_else(|e| panic!("{}:{} {}", e.span.line, e.span.col, e.msg));
        let DeclKind::Fn(f) = &file.decls[0].kind else { panic!() };
        let Some(Expr { kind: ExprKind::Block(b), .. }) = &f.body else { panic!() };
        assert_eq!(b.stmts.len(), 2, "two lets");
        assert_eq!(sexpr(b.tail.as_ref().unwrap()), "(- a)", "a leading minus starts a new statement");
    }

    #[test]
    fn examples_parse() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/examples");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "mou") {
                let src = std::fs::read_to_string(&path).unwrap();
                if let Err(e) = parse_file(&src) {
                    panic!("{}:{}:{}: {}", path.display(), e.span.line, e.span.col, e.msg);
                }
                count += 1;
            }
        }
        assert!(count >= 3, "expected the example programs to be present");
    }
}
