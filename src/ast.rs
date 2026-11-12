//! Abstract syntax tree produced by the parser.
#![allow(dead_code)] // Some nodes are parsed but not consumed by any stage yet.

use crate::lexer::Span;

#[derive(Debug, Clone)]
pub struct File {
    pub profile: Option<String>,
    pub imports: Vec<Import>,
    pub decls: Vec<Decl>,
    /// The modules of a loaded program. Empty for a file parsed on its own,
    /// which is then a single module that imports nothing but built-ins.
    pub modules: Vec<Module>,
}

#[derive(Debug, Clone)]
pub struct Module {
    /// Dotted name, as in `import`. Empty for the program's own file.
    pub name: String,
    /// Its declarations inside `File::decls`.
    pub decls: std::ops::Range<usize>,
    pub imports: Vec<ModImport>,
    /// `@profile(name)` at the top of its file.
    pub profile: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModImport {
    /// Name the importing module uses.
    pub alias: String,
    pub path: String,
    /// Number of the imported module, or `None` for one the compiler provides.
    pub module: Option<usize>,
    /// `c` for a C header.
    pub origin: Option<String>,
    /// The functions of a C header the module declares.
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct Import {
    /// `c`, `cpp` or `py` for foreign imports.
    pub origin: Option<String>,
    pub path: String,
    pub alias: Option<String>,
    /// For a C header: the functions of it the program uses.
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub kind: DeclKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum DeclKind {
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Trait(TraitDecl),
    Impl(ImplDecl),
    Const { name: String, ty: Option<Type>, value: Expr },
    TypeAlias { name: String, generics: Vec<GenParam>, ty: Type },
}

impl DeclKind {
    pub fn describe(&self) -> (&'static str, String) {
        match self {
            DeclKind::Fn(f) => ("fn", f.name.clone()),
            DeclKind::Struct(s) => ("struct", s.name.clone()),
            DeclKind::Enum(e) => ("enum", e.name.clone()),
            DeclKind::Trait(t) => ("trait", t.name.clone()),
            DeclKind::Impl(i) => ("impl", ty_text(&i.ty)),
            DeclKind::Const { name, .. } => ("let", name.clone()),
            DeclKind::TypeAlias { name, .. } => ("type", name.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Immutable borrow (the default).
    Let,
    /// Exclusive mutable borrow.
    Mut,
    /// Ownership transfer.
    Own,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: String,
    pub generics: Vec<GenParam>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub ret_ref: bool,
    pub err: Option<Type>,
    pub where_: Vec<Expr>,
    pub body: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub comptime: bool,
    pub name: String,
    pub mode: Mode,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub enum GenParam {
    Type { name: String, bounds: Vec<Type> },
    Region(String),
    Comptime { name: String, ty: Type },
}

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub name: String,
    pub generics: Vec<GenParam>,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub public: bool,
    pub name: String,
    pub ty: Type,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub name: String,
    pub generics: Vec<GenParam>,
    pub variants: Vec<Variant>,
}

#[derive(Debug, Clone)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<(Option<String>, Type)>,
    pub value: Option<Expr>,
}

#[derive(Debug, Clone)]
pub struct TraitDecl {
    pub name: String,
    pub generics: Vec<GenParam>,
    pub supers: Vec<Type>,
    pub methods: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub struct ImplDecl {
    pub generics: Vec<GenParam>,
    pub trait_: Option<Type>,
    pub ty: Type,
    pub decls: Vec<Decl>,
}

#[derive(Debug, Clone)]
pub enum Type {
    Path { path: Vec<String>, args: Vec<GenArg> },
    Slice(Box<Type>),
    Array(Box<Type>, Box<Expr>),
    Soa(Box<Type>),
    Tuple(Vec<Type>),
    Fn { params: Vec<(Mode, Type)>, ret: Option<Box<Type>>, err: Option<Box<Type>> },
    Ref { region: Option<String>, mutable: bool, ty: Box<Type> },
    Dyn(Vec<Type>),
    Optional(Box<Type>),
    SelfType,
    TypeType,
}

#[derive(Debug, Clone)]
pub enum GenArg {
    Type(Type),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StrPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(String),
    Float(String),
    Str(Vec<StrPart>),
    Char(char),
    Bool(bool),
    /// A single name. Dotted access is a chain of `Field`.
    Path(String),
    Tuple(Vec<Expr>),
    Array(Vec<Expr>),
    ArrayRepeat(Box<Expr>, Box<Expr>),
    Block(Block),
    If { cond: Box<Cond>, then: Block, els: Option<Box<Expr>> },
    Match { scrutinee: Box<Expr>, arms: Vec<Arm> },
    For { pat: Pattern, iter: Box<Expr>, body: Block },
    While { cond: Box<Cond>, body: Block },
    Loop(Block),
    Closure { params: Vec<Param>, ret: Option<Type>, err: Option<Type>, body: Block },
    Comptime(Box<Expr>),
    Unsafe(Block),
    Scope { name: String, body: Block },
    With { bindings: Vec<(String, Expr)>, body: Block },
    Return(Option<Box<Expr>>),
    Break(Option<Box<Expr>>),
    Continue,
    Unary { op: &'static str, expr: Box<Expr> },
    Binary { op: &'static str, lhs: Box<Expr>, rhs: Box<Expr> },
    Range { lo: Option<Box<Expr>>, hi: Option<Box<Expr>>, inclusive: bool },
    Cast { expr: Box<Expr>, ty: Type },
    Call { callee: Box<Expr>, args: Vec<Arg> },
    /// `base[args]`: indexing or generic application, decided by name resolution.
    Index { base: Box<Expr>, args: Vec<Expr> },
    Field { base: Box<Expr>, name: String },
    Try(Box<Expr>),
    Catch { expr: Box<Expr>, binding: Option<String>, handler: Block },
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub label: Option<String>,
    pub mode: Mode,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub enum Cond {
    Expr(Expr),
    Let(Pattern, Expr),
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pat: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Let { mutable: bool, pat: Pattern, ty: Option<Type>, value: Option<Expr> },
    Assign { target: Expr, op: &'static str, value: Expr },
    Defer(Expr),
    Expr(Expr),
    Decl(Decl),
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Wildcard,
    Literal(Box<Expr>),
    Range { lo: Box<Expr>, hi: Option<Box<Expr>>, inclusive: bool },
    Bind { mutable: bool, name: String },
    /// Enum variant or positional struct: `Circle(_, r)`, `Order.Less`, `None`.
    Ctor { path: Vec<String>, args: Vec<Pattern> },
    Struct { path: Vec<String>, fields: Vec<(String, Option<Pattern>)>, rest: bool },
    Tuple(Vec<Pattern>),
    Slice(Vec<Pattern>),
    Rest,
    Or(Vec<Pattern>),
}

/// Compact s-expression form of an expression, used by tests.
pub fn sexpr(e: &Expr) -> String {
    let opt = |o: &Option<Box<Expr>>| o.as_ref().map_or("_".to_string(), |e| sexpr(e));
    match &e.kind {
        ExprKind::Int(s) | ExprKind::Float(s) | ExprKind::Path(s) => s.clone(),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::Unary { op, expr } => format!("({op} {})", sexpr(expr)),
        ExprKind::Binary { op, lhs, rhs } => format!("({op} {} {})", sexpr(lhs), sexpr(rhs)),
        ExprKind::Range { lo, hi, inclusive } => {
            format!("({} {} {})", if *inclusive { "..=" } else { ".." }, opt(lo), opt(hi))
        }
        ExprKind::Cast { expr, .. } => format!("(as {})", sexpr(expr)),
        ExprKind::Call { callee, args } => {
            let args: String = args.iter().map(|a| format!(" {}", sexpr(&a.value))).collect();
            format!("(call {}{args})", sexpr(callee))
        }
        ExprKind::Index { base, args } => {
            let args: String = args.iter().map(|a| format!(" {}", sexpr(a))).collect();
            format!("(index {}{args})", sexpr(base))
        }
        ExprKind::Field { base, name } => format!("(. {} {name})", sexpr(base)),
        ExprKind::Try(e) => format!("(? {})", sexpr(e)),
        ExprKind::Tuple(items) => {
            format!("(tuple{})", items.iter().map(|a| format!(" {}", sexpr(a))).collect::<String>())
        }
        _ => "<expr>".to_string(),
    }
}

// ---- the tree as text ----
//
// One form for all of the tree, which the parser written in Mou
// (`compiler/parser.mou`) prints too, so that the two can be compared.

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

fn seq<T>(items: &[T], f: impl Fn(&T) -> String) -> String {
    format!("[{}]", items.iter().map(f).collect::<Vec<_>>().join(" "))
}

fn opt<T>(o: &Option<T>, f: impl Fn(&T) -> String) -> String {
    o.as_ref().map_or("_".to_string(), f)
}

fn mode_text(m: &Mode) -> &'static str {
    match m {
        Mode::Let => "let",
        Mode::Mut => "mut",
        Mode::Own => "own",
    }
}

fn dots(inclusive: bool) -> &'static str {
    if inclusive { "..=" } else { ".." }
}

fn var_text(mutable: bool) -> &'static str {
    if mutable { "mut" } else { "let" }
}

fn ty_text(t: &Type) -> String {
    match t {
        Type::Path { path, args } => {
            let arg = |a: &GenArg| match a {
                GenArg::Type(t) => ty_text(t),
                GenArg::Expr(e) => expr_text(e),
            };
            format!("(path {} {})", path.join("."), seq(args, arg))
        }
        Type::Slice(t) => format!("(slice {})", ty_text(t)),
        Type::Array(t, n) => format!("(array {} {})", ty_text(t), expr_text(n)),
        Type::Soa(t) => format!("(soa {})", ty_text(t)),
        Type::Tuple(ts) => format!("(tuple {})", seq(ts, ty_text)),
        Type::Fn { params, ret, err } => {
            let param = |p: &(Mode, Type)| format!("({} {})", mode_text(&p.0), ty_text(&p.1));
            format!("(fn {} {} {})", seq(params, param), opt(ret, |t| ty_text(t)), opt(err, |t| ty_text(t)))
        }
        Type::Ref { region, mutable, ty } => format!("(ref {} {} {})", opt(region, |r| r.clone()), var_text(*mutable), ty_text(ty)),
        Type::Dyn(ts) => format!("(dyn {})", seq(ts, ty_text)),
        Type::Optional(t) => format!("(opt {})", ty_text(t)),
        Type::SelfType => "Self".to_string(),
        Type::TypeType => "type".to_string(),
    }
}

fn pattern_text(p: &Pattern) -> String {
    match p {
        Pattern::Wildcard => "_".to_string(),
        Pattern::Literal(e) => format!("(lit {})", expr_text(e)),
        Pattern::Range { lo, hi, inclusive } => format!("(prange {} {} {})", expr_text(lo), opt(hi, |e| expr_text(e)), dots(*inclusive)),
        Pattern::Bind { mutable, name } => format!("(bind {} {name})", var_text(*mutable)),
        Pattern::Ctor { path, args } => format!("(ctor {} {})", path.join("."), seq(args, pattern_text)),
        Pattern::Struct { path, fields, rest } => {
            let field = |f: &(String, Option<Pattern>)| format!("({} {})", f.0, opt(&f.1, pattern_text));
            format!("(pstruct {} {} {})", path.join("."), seq(fields, field), if *rest { "rest" } else { "norest" })
        }
        Pattern::Tuple(ps) => format!("(ptuple {})", seq(ps, pattern_text)),
        Pattern::Slice(ps) => format!("(pslice {})", seq(ps, pattern_text)),
        Pattern::Rest => "..".to_string(),
        Pattern::Or(ps) => format!("(or {})", seq(ps, pattern_text)),
    }
}

fn cond_text(c: &Cond) -> String {
    match c {
        Cond::Expr(e) => expr_text(e),
        Cond::Let(p, e) => format!("(let {} {})", pattern_text(p), expr_text(e)),
    }
}

fn param_text(p: &Param) -> String {
    let kind = if p.comptime { "comptime" } else { "plain" };
    format!("(param {kind} {} {} {} {})", p.name, mode_text(&p.mode), opt(&p.ty, ty_text), opt(&p.default, expr_text))
}

fn block_text(b: &Block) -> String {
    let stmt = |s: &Stmt| match s {
        Stmt::Let { mutable, pat, ty, value } => {
            format!("(let {} {} {} {})", var_text(*mutable), pattern_text(pat), opt(ty, ty_text), opt(value, expr_text))
        }
        Stmt::Assign { target, op, value } => format!("(assign {op} {} {})", expr_text(target), expr_text(value)),
        Stmt::Defer(e) => format!("(defer {})", expr_text(e)),
        Stmt::Expr(e) => format!("(do {})", expr_text(e)),
        Stmt::Decl(d) => decl_text(d),
    };
    format!("{{{} {}}}", seq(&b.stmts, stmt), opt(&b.tail, |e| expr_text(e)))
}

fn expr_text(e: &Expr) -> String {
    let some = |o: &Option<Box<Expr>>| opt(o, |e| expr_text(e));
    let body = match &e.kind {
        ExprKind::Int(s) => format!("int {s}"),
        ExprKind::Float(s) => format!("float {s}"),
        ExprKind::Str(parts) => {
            let part = |p: &StrPart| match p {
                StrPart::Lit(s) => quote(s),
                StrPart::Expr(e) => expr_text(e),
            };
            format!("str {}", seq(parts, part))
        }
        ExprKind::Char(c) => format!("char {}", *c as u32),
        ExprKind::Bool(b) => format!("bool {b}"),
        ExprKind::Path(s) => format!("name {s}"),
        ExprKind::Tuple(items) => format!("tuple {}", seq(items, expr_text)),
        ExprKind::Array(items) => format!("array {}", seq(items, expr_text)),
        ExprKind::ArrayRepeat(v, n) => format!("repeat {} {}", expr_text(v), expr_text(n)),
        ExprKind::Block(b) => format!("block {}", block_text(b)),
        ExprKind::If { cond, then, els } => format!("if {} {} {}", cond_text(cond), block_text(then), some(els)),
        ExprKind::Match { scrutinee, arms } => {
            let arm = |a: &Arm| format!("(arm {} {} {})", pattern_text(&a.pat), opt(&a.guard, expr_text), expr_text(&a.body));
            format!("match {} {}", expr_text(scrutinee), seq(arms, arm))
        }
        ExprKind::For { pat, iter, body } => format!("for {} {} {}", pattern_text(pat), expr_text(iter), block_text(body)),
        ExprKind::While { cond, body } => format!("while {} {}", cond_text(cond), block_text(body)),
        ExprKind::Loop(b) => format!("loop {}", block_text(b)),
        ExprKind::Closure { params, ret, err, body } => {
            format!("closure {} {} {} {}", seq(params, param_text), opt(ret, ty_text), opt(err, ty_text), block_text(body))
        }
        ExprKind::Comptime(e) => format!("comptime {}", expr_text(e)),
        ExprKind::Unsafe(b) => format!("unsafe {}", block_text(b)),
        ExprKind::Scope { name, body } => format!("scope {name} {}", block_text(body)),
        ExprKind::With { bindings, body } => {
            let binding = |b: &(String, Expr)| format!("({} {})", b.0, expr_text(&b.1));
            format!("with {} {}", seq(bindings, binding), block_text(body))
        }
        ExprKind::Return(v) => format!("return {}", some(v)),
        ExprKind::Break(v) => format!("break {}", some(v)),
        ExprKind::Continue => "continue".to_string(),
        ExprKind::Unary { op, expr } => format!("un {op} {}", expr_text(expr)),
        ExprKind::Binary { op, lhs, rhs } => format!("bin {op} {} {}", expr_text(lhs), expr_text(rhs)),
        ExprKind::Range { lo, hi, inclusive } => format!("range {} {} {}", some(lo), some(hi), dots(*inclusive)),
        ExprKind::Cast { expr, ty } => format!("cast {} {}", expr_text(expr), ty_text(ty)),
        ExprKind::Call { callee, args } => {
            let arg = |a: &Arg| format!("(arg {} {} {})", opt(&a.label, |l| l.clone()), mode_text(&a.mode), expr_text(&a.value));
            format!("call {} {}", expr_text(callee), seq(args, arg))
        }
        ExprKind::Index { base, args } => format!("index {} {}", expr_text(base), seq(args, expr_text)),
        ExprKind::Field { base, name } => format!("field {} {name}", expr_text(base)),
        ExprKind::Try(e) => format!("try {}", expr_text(e)),
        ExprKind::Catch { expr, binding, handler } => {
            format!("catch {} {} {}", expr_text(expr), opt(binding, |b| b.clone()), block_text(handler))
        }
    };
    format!("({}:{} {body})", e.span.line, e.span.col)
}

fn generics_text(gs: &[GenParam]) -> String {
    seq(gs, |g| match g {
        GenParam::Type { name, bounds } => format!("(tparam {name} {})", seq(bounds, ty_text)),
        GenParam::Region(name) => format!("(region {name})"),
        GenParam::Comptime { name, ty } => format!("(cparam {name} {})", ty_text(ty)),
    })
}

fn decl_text(d: &Decl) -> String {
    let attr = |a: &Attr| format!("(attr {} {})", a.name, seq(&a.args, expr_text));
    let kind = match &d.kind {
        DeclKind::Fn(f) => format!(
            "(fn {} {} {} {} {} {} {} {})",
            f.name,
            generics_text(&f.generics),
            seq(&f.params, param_text),
            opt(&f.ret, ty_text),
            if f.ret_ref { "ref" } else { "val" },
            opt(&f.err, ty_text),
            seq(&f.where_, expr_text),
            opt(&f.body, expr_text)
        ),
        DeclKind::Struct(s) => {
            let field = |f: &Field| {
                format!("(field {} {} {} {})", if f.public { "pub" } else { "priv" }, f.name, ty_text(&f.ty), opt(&f.default, expr_text))
            };
            format!("(struct {} {} {})", s.name, generics_text(&s.generics), seq(&s.fields, field))
        }
        DeclKind::Enum(e) => {
            let variant = |v: &Variant| {
                let field = |f: &(Option<String>, Type)| format!("({} {})", opt(&f.0, |l| l.clone()), ty_text(&f.1));
                format!("(variant {} {} {})", v.name, seq(&v.fields, field), opt(&v.value, expr_text))
            };
            format!("(enum {} {} {})", e.name, generics_text(&e.generics), seq(&e.variants, variant))
        }
        DeclKind::Trait(t) => format!("(trait {} {} {} {})", t.name, generics_text(&t.generics), seq(&t.supers, ty_text), seq(&t.methods, decl_text)),
        DeclKind::Impl(i) => {
            format!("(impl {} {} {} {})", generics_text(&i.generics), opt(&i.trait_, ty_text), ty_text(&i.ty), seq(&i.decls, decl_text))
        }
        DeclKind::Const { name, ty, value } => format!("(const {name} {} {})", opt(ty, ty_text), expr_text(value)),
        DeclKind::TypeAlias { name, generics, ty } => format!("(alias {name} {} {})", generics_text(generics), ty_text(ty)),
    };
    format!("(decl {}:{} {} {} {kind})", d.span.line, d.span.col, seq(&d.attrs, attr), if d.public { "pub" } else { "priv" })
}

/// Declarations as text, for what is printed about a module.
pub fn decls_dump(decls: &[Decl]) -> String {
    seq(decls, decl_text)
}

/// The tree of a file, or its first error, as text.
pub fn dump(parsed: &Result<File, crate::lexer::Error>) -> String {
    let file = match parsed {
        Ok(f) => f,
        Err(e) => return format!("error {}:{} {}\n", e.span.line, e.span.col, e.msg),
    };
    let mut out = format!("profile {}\n", opt(&file.profile, |p| p.clone()));
    for i in &file.imports {
        let origin = opt(&i.origin, |o| o.clone());
        out.push_str(&format!("(import {origin} {} {} {})\n", quote(&i.path), opt(&i.alias, |a| a.clone()), seq(&i.decls, decl_text)));
    }
    for d in &file.decls {
        out.push_str(&decl_text(d));
        out.push('\n');
    }
    out
}

/// The declarations written inside the blocks of others, each one before
/// those inside it. A declaration in a block is a declaration of its
/// module: it is written there to be next to what uses it.
pub fn nested_decls(decls: &[Decl]) -> Vec<Decl> {
    fn in_block(b: &Block, out: &mut Vec<Decl>) {
        for s in &b.stmts {
            match s {
                Stmt::Let { value, .. } => {
                    if let Some(v) = value {
                        in_expr(v, out);
                    }
                }
                Stmt::Assign { target, value, .. } => {
                    in_expr(target, out);
                    in_expr(value, out);
                }
                Stmt::Defer(e) | Stmt::Expr(e) => in_expr(e, out),
                Stmt::Decl(d) => {
                    out.push(d.clone());
                    in_decl(d, out);
                }
            }
        }
        if let Some(t) = &b.tail {
            in_expr(t, out);
        }
    }
    fn in_cond(c: &Cond, out: &mut Vec<Decl>) {
        match c {
            Cond::Expr(e) | Cond::Let(_, e) => in_expr(e, out),
        }
    }
    fn in_expr(e: &Expr, out: &mut Vec<Decl>) {
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Path(_) | ExprKind::Continue => {}
            ExprKind::Str(parts) => {
                for p in parts {
                    if let StrPart::Expr(inner) = p {
                        in_expr(inner, out);
                    }
                }
            }
            ExprKind::Tuple(items) | ExprKind::Array(items) => items.iter().for_each(|i| in_expr(i, out)),
            ExprKind::ArrayRepeat(a, b) => {
                in_expr(a, out);
                in_expr(b, out);
            }
            ExprKind::Block(b) | ExprKind::Loop(b) | ExprKind::Unsafe(b) => in_block(b, out),
            ExprKind::If { cond, then, els } => {
                in_cond(cond, out);
                in_block(then, out);
                if let Some(x) = els {
                    in_expr(x, out);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                in_expr(scrutinee, out);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        in_expr(g, out);
                    }
                    in_expr(&arm.body, out);
                }
            }
            ExprKind::For { iter, body, .. } => {
                in_expr(iter, out);
                in_block(body, out);
            }
            ExprKind::While { cond, body } => {
                in_cond(cond, out);
                in_block(body, out);
            }
            ExprKind::Closure { body, .. } | ExprKind::Scope { body, .. } => in_block(body, out),
            ExprKind::With { bindings, body } => {
                bindings.iter().for_each(|b| in_expr(&b.1, out));
                in_block(body, out);
            }
            ExprKind::Comptime(x) | ExprKind::Try(x) => in_expr(x, out),
            ExprKind::Return(x) | ExprKind::Break(x) => {
                if let Some(x) = x {
                    in_expr(x, out);
                }
            }
            ExprKind::Unary { expr: x, .. } | ExprKind::Cast { expr: x, .. } | ExprKind::Field { base: x, .. } => in_expr(x, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                in_expr(lhs, out);
                in_expr(rhs, out);
            }
            ExprKind::Range { lo, hi, .. } => {
                for x in [lo, hi].into_iter().flatten() {
                    in_expr(x, out);
                }
            }
            ExprKind::Call { callee, args } => {
                in_expr(callee, out);
                args.iter().for_each(|a| in_expr(&a.value, out));
            }
            ExprKind::Index { base, args } => {
                in_expr(base, out);
                args.iter().for_each(|a| in_expr(a, out));
            }
            ExprKind::Catch { expr: x, handler, .. } => {
                in_expr(x, out);
                in_block(handler, out);
            }
        }
    }
    fn in_decl(d: &Decl, out: &mut Vec<Decl>) {
        match &d.kind {
            DeclKind::Fn(f) => {
                f.params.iter().filter_map(|p| p.default.as_ref()).for_each(|e| in_expr(e, out));
                f.where_.iter().for_each(|e| in_expr(e, out));
                if let Some(b) = &f.body {
                    in_expr(b, out);
                }
            }
            DeclKind::Struct(s) => s.fields.iter().filter_map(|f| f.default.as_ref()).for_each(|e| in_expr(e, out)),
            DeclKind::Enum(e) => e.variants.iter().filter_map(|v| v.value.as_ref()).for_each(|e| in_expr(e, out)),
            DeclKind::Trait(t) => t.methods.iter().for_each(|m| in_decl(m, out)),
            DeclKind::Impl(i) => i.decls.iter().for_each(|m| in_decl(m, out)),
            DeclKind::Const { value, .. } => in_expr(value, out),
            DeclKind::TypeAlias { .. } => {}
        }
    }
    let mut out = Vec::new();
    decls.iter().for_each(|d| in_decl(d, &mut out));
    out
}
