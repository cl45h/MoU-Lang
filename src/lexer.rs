//! Lexer: source text -> tokens, including the significant-newline rule.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// Line in the file, plus the module number shifted by `FILE_SHIFT`.
    pub line: u32,
    pub col: u32,
}

/// Bits of `Span::line` that hold the line; the rest say which module.
pub const FILE_SHIFT: u32 = 20;

impl Span {
    /// The line inside its own file.
    pub fn local_line(self) -> u32 {
        self.line & ((1 << FILE_SHIFT) - 1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// Source of an interpolated `{expr}` and where it starts.
    Expr(String, Span),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Int(String),
    Float(String),
    Str(Vec<StrPart>),
    Char(char),
    Kw(&'static str),
    P(&'static str),
    Newline,
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Error {
    pub msg: String,
    pub span: Span,
}

pub const KEYWORDS: &[&str] = &[
    "as", "break", "catch", "comptime", "continue", "defer", "dyn", "else", "enum", "false", "fn",
    "for", "if", "impl", "import", "in", "let", "loop", "match", "mut", "own", "pub", "ref",
    "region", "return", "scope", "self", "Self", "soa", "struct", "trait", "true", "type",
    "unsafe", "var", "where", "while", "with",
];

/// Longest first, so greedy matching picks the right one.
pub const PUNCT: &[&str] = &[
    "<<=", ">>=", "..=", "+%", "-%", "*%", "+|", "-|", "*|", "==", "!=", "<=", ">=", "&&", "||",
    "<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "->", "=>", "..", "+", "-", "*",
    "/", "%", "<", ">", "=", "!", "~", "&", "|", "^", ".", ",", ":", ";", "(", ")", "[", "]", "{",
    "}", "?", "@",
];

/// A newline is dropped when the next line starts with one of these.
pub const CONTINUES_LINE: &[&str] = &[
    ".", "?", "==", "!=", "<", "<=", ">", ">=", "&&", "||", "+", "*", "/", "%", "+%", "-%", "*%",
    "+|", "-|", "*|", "&", "|", "^", "<<", ">>", "..", "..=", "=", "+=", "-=", "*=", "/=", "%=",
    "&=", "|=", "^=", "<<=", ">>=", "->", "=>", ",",
];

/// The tokens of a text, one on each line, in the form the lexer written
/// in Mou (`compiler/lexer.mou`) prints them, so that the two can be compared.
pub fn dump(src: &str) -> String {
    use std::fmt::Write;
    let escape = |s: &str| s.replace('\\', "\\\\").replace('\n', "\\n");
    let mut out = String::new();
    let toks = match lex(src) {
        Ok(toks) => toks,
        Err(e) => return format!("error {}:{} {}\n", e.span.line, e.span.col, e.msg),
    };
    for t in toks {
        let at = format!("{}:{}", t.span.line, t.span.col);
        let _ = match t.tok {
            Tok::Ident(s) => writeln!(out, "{at} ident {s}"),
            Tok::Int(s) => writeln!(out, "{at} int {s}"),
            Tok::Float(s) => writeln!(out, "{at} float {s}"),
            Tok::Char(c) => writeln!(out, "{at} char {}", c as u32),
            Tok::Kw(k) => writeln!(out, "{at} kw {k}"),
            Tok::P(p) => writeln!(out, "{at} p {p}"),
            Tok::Newline => writeln!(out, "{at} newline"),
            Tok::Eof => writeln!(out, "{at} eof"),
            Tok::Str(parts) => {
                let _ = writeln!(out, "{at} str {}", parts.len());
                for part in parts {
                    let _ = match part {
                        StrPart::Lit(s) => writeln!(out, "  lit {}", escape(&s)),
                        StrPart::Expr(s, at) => writeln!(out, "  expr {}:{} {}", at.line, at.col, escape(&s)),
                    };
                }
                Ok(())
            }
        };
    }
    out
}

pub fn lex(src: &str) -> Result<Vec<Token>, Error> {
    lex_at(src, Span { line: 1, col: 1 })
}

pub fn lex_at(src: &str, origin: Span) -> Result<Vec<Token>, Error> {
    let mut lx = Lexer { chars: src.chars().collect(), i: 0, line: origin.line, col: origin.col };
    Ok(filter_newlines(lx.run()?))
}

struct Lexer {
    chars: Vec<char>,
    i: usize,
    line: u32,
    col: u32,
}

impl Lexer {
    fn peek(&self, n: usize) -> Option<char> {
        self.chars.get(self.i + n).copied()
    }

    fn span(&self) -> Span {
        Span { line: self.line, col: self.col }
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(k, c)| self.peek(k) == Some(c))
    }

    fn err<T>(&self, msg: &str, span: Span) -> Result<T, Error> {
        Err(Error { msg: msg.to_string(), span })
    }

    fn run(&mut self) -> Result<Vec<Token>, Error> {
        let mut out: Vec<Token> = Vec::new();
        'outer: loop {
            let span = self.span();
            let Some(c) = self.peek(0) else {
                out.push(Token { tok: Tok::Eof, span });
                break;
            };
            if c == '\n' {
                self.advance();
                out.push(Token { tok: Tok::Newline, span });
                continue;
            }
            if c.is_whitespace() {
                self.advance();
                continue;
            }
            if c == '/' && self.peek(1) == Some('/') {
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.advance();
                }
                continue;
            }
            if c == '/' && self.peek(1) == Some('*') {
                self.block_comment(span)?;
                continue;
            }
            if c.is_alphabetic() || c == '_' {
                let mut s = String::new();
                while let Some(c) = self.peek(0).filter(|c| c.is_alphanumeric() || *c == '_') {
                    s.push(c);
                    self.advance();
                }
                let tok = match KEYWORDS.iter().find(|k| **k == s) {
                    Some(k) => Tok::Kw(k),
                    None => Tok::Ident(s),
                };
                out.push(Token { tok, span });
                continue;
            }
            if c.is_ascii_digit() {
                // After a dot (`t.0.1`) a number is always a plain integer.
                let after_dot = matches!(out.last(), Some(Token { tok: Tok::P("."), .. }));
                let tok = self.number(after_dot);
                out.push(Token { tok, span });
                continue;
            }
            if c == '"' {
                let tok = self.string(span)?;
                out.push(Token { tok, span });
                continue;
            }
            if c == '\'' {
                self.advance();
                let ch = match self.advance() {
                    Some('\\') => {
                        let e = self.advance();
                        match e {
                            Some(e) => unescape(e),
                            None => return self.err("unterminated character literal", span),
                        }
                    }
                    Some(ch) => ch,
                    None => return self.err("unterminated character literal", span),
                };
                if self.advance() != Some('\'') {
                    return self.err("expected closing `'` in character literal", span);
                }
                out.push(Token { tok: Tok::Char(ch), span });
                continue;
            }
            for p in PUNCT {
                if self.starts_with(p) {
                    for _ in 0..p.len() {
                        self.advance();
                    }
                    out.push(Token { tok: Tok::P(p), span });
                    continue 'outer;
                }
            }
            return Err(Error { msg: format!("unexpected character `{c}`"), span });
        }
        Ok(out)
    }

    fn block_comment(&mut self, start: Span) -> Result<(), Error> {
        let mut depth = 0;
        loop {
            if self.starts_with("/*") {
                depth += 1;
                self.advance();
                self.advance();
            } else if self.starts_with("*/") {
                depth -= 1;
                self.advance();
                self.advance();
                if depth == 0 {
                    return Ok(());
                }
            } else if self.advance().is_none() {
                return self.err("unterminated block comment", start);
            }
        }
    }

    fn digits(&mut self, s: &mut String) {
        while let Some(c) = self.peek(0).filter(|c| c.is_ascii_digit() || *c == '_') {
            s.push(c);
            self.advance();
        }
    }

    fn number(&mut self, after_dot: bool) -> Tok {
        let mut s = String::new();
        if !after_dot && self.peek(0) == Some('0') && matches!(self.peek(1), Some('x' | 'b' | 'o')) {
            s.push(self.advance().unwrap());
            s.push(self.advance().unwrap());
            while let Some(c) = self.peek(0).filter(|c| c.is_ascii_alphanumeric() || *c == '_') {
                s.push(c);
                self.advance();
            }
            return Tok::Int(s);
        }
        self.digits(&mut s);
        if after_dot {
            return Tok::Int(s);
        }
        let mut float = false;
        if self.peek(0) == Some('.') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
            float = true;
            s.push(self.advance().unwrap());
            self.digits(&mut s);
        }
        if matches!(self.peek(0), Some('e' | 'E')) {
            let signed = matches!(self.peek(1), Some('+' | '-'));
            let digit_at = if signed { 2 } else { 1 };
            if self.peek(digit_at).is_some_and(|c| c.is_ascii_digit()) {
                float = true;
                for _ in 0..digit_at {
                    s.push(self.advance().unwrap());
                }
                self.digits(&mut s);
            }
        }
        if float {
            Tok::Float(s)
        } else {
            Tok::Int(s)
        }
    }

    fn string(&mut self, start: Span) -> Result<Tok, Error> {
        self.advance();
        let mut parts = Vec::new();
        let mut cur = String::new();
        loop {
            let Some(c) = self.peek(0) else {
                return self.err("unterminated string literal", start);
            };
            match c {
                '"' => {
                    self.advance();
                    break;
                }
                '\\' => {
                    self.advance();
                    match self.advance() {
                        Some(e) => cur.push(unescape(e)),
                        None => return self.err("unterminated string literal", start),
                    }
                }
                '{' if self.peek(1) == Some('{') => {
                    self.advance();
                    self.advance();
                    cur.push('{');
                }
                '}' if self.peek(1) == Some('}') => {
                    self.advance();
                    self.advance();
                    cur.push('}');
                }
                '{' => {
                    self.advance();
                    let espan = self.span();
                    let mut depth = 1;
                    let mut src = String::new();
                    loop {
                        let Some(c) = self.advance() else {
                            return self.err("unterminated `{` in string interpolation", espan);
                        };
                        if c == '\n' {
                            return self.err("a `{` in a string starts a value to insert, which has to end with `}` on the same line: for a brace alone write `{{`", espan);
                        }
                        if c == '{' {
                            depth += 1;
                        } else if c == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        src.push(c);
                    }
                    if !cur.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut cur)));
                    }
                    parts.push(StrPart::Expr(src, espan));
                }
                _ => {
                    self.advance();
                    cur.push(c);
                }
            }
        }
        if !cur.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(cur));
        }
        Ok(Tok::Str(parts))
    }
}

fn unescape(c: char) -> char {
    match c {
        'n' => '\n',
        't' => '\t',
        'r' => '\r',
        '0' => '\0',
        other => other,
    }
}

/// Applies the statement-terminator rule. A newline survives only when it can
/// end a statement: outside `()`/`[]`, after a token that can end an
/// expression, and not followed by a token that continues one.
fn filter_newlines(raw: Vec<Token>) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    let mut stack: Vec<&'static str> = Vec::new();
    for (i, t) in raw.iter().enumerate() {
        match t.tok {
            Tok::P(p) if matches!(p, "(" | "[" | "{") => stack.push(p),
            Tok::P(p) if matches!(p, ")" | "]" | "}") => {
                stack.pop();
            }
            _ => {}
        }
        if t.tok == Tok::Newline {
            if stack.last().is_some_and(|s| *s == "(" || *s == "[") {
                continue;
            }
            let Some(prev) = out.last() else { continue };
            match prev.tok {
                Tok::Newline => continue,
                Tok::P(p) if !matches!(p, ")" | "]" | "}" | "?") => continue,
                _ => {}
            }
            let next = raw[i + 1..].iter().map(|t| &t.tok).find(|t| **t != Tok::Newline);
            let continues = match next {
                Some(Tok::P(p)) => CONTINUES_LINE.contains(p),
                Some(Tok::Kw(k)) => matches!(*k, "else" | "catch" | "where" | "as"),
                _ => false,
            };
            if continues {
                continue;
            }
        }
        out.push(t.clone());
    }
    out
}
