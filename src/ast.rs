//! Abstract syntax tree produced by the parser.
#![allow(dead_code)] // Some nodes are parsed but not consumed by any stage yet.

use crate::lexer::Span;

#[derive(Debug)]
pub struct File {
    pub profile: Option<String>,
    pub imports: Vec<Import>,
    pub decls: Vec<Decl>,
    /// The modules of a loaded program. Empty for a file parsed on its own,
    /// which is then a single module that imports nothing but built-ins.
    pub modules: Vec<Module>,
}

#[derive(Debug)]
pub struct Module {
    /// Dotted name, as in `import`. Empty for the program's own file.
    pub name: String,
    /// Its declarations inside `File::decls`.
    pub decls: std::ops::Range<usize>,
    pub imports: Vec<ModImport>,
    /// `@profile(name)` at the top of its file.
    pub profile: Option<String>,
}

#[derive(Debug)]
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

#[derive(Debug)]
pub struct Import {
    /// `c`, `cpp` or `py` for foreign imports.
    pub origin: Option<String>,
    pub path: String,
    pub alias: Option<String>,
    /// For a C header: the functions of it the program uses.
    pub decls: Vec<Decl>,
}

#[derive(Debug)]
pub struct Attr {
    pub name: String,
    pub args: Vec<Expr>,
}

#[derive(Debug)]
pub struct Decl {
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub kind: DeclKind,
    pub span: Span,
}

#[derive(Debug)]
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
            DeclKind::Impl(i) => ("impl", format!("{:?}", i.ty)),
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

#[derive(Debug)]
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

#[derive(Debug)]
pub struct Param {
    pub comptime: bool,
    pub name: String,
    pub mode: Mode,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

#[derive(Debug)]
pub enum GenParam {
    Type { name: String, bounds: Vec<Type> },
    Region(String),
    Comptime { name: String, ty: Type },
}

#[derive(Debug)]
pub struct StructDecl {
    pub name: String,
    pub generics: Vec<GenParam>,
    pub fields: Vec<Field>,
}

#[derive(Debug)]
pub struct Field {
    pub public: bool,
    pub name: String,
    pub ty: Type,
    pub default: Option<Expr>,
}

#[derive(Debug)]
pub struct EnumDecl {
    pub name: String,
    pub generics: Vec<GenParam>,
    pub variants: Vec<Variant>,
}

#[derive(Debug)]
pub struct Variant {
    pub name: String,
    pub fields: Vec<(Option<String>, Type)>,
    pub value: Option<Expr>,
}

#[derive(Debug)]
pub struct TraitDecl {
    pub name: String,
    pub generics: Vec<GenParam>,
    pub supers: Vec<Type>,
    pub methods: Vec<Decl>,
}

#[derive(Debug)]
pub struct ImplDecl {
    pub generics: Vec<GenParam>,
    pub trait_: Option<Type>,
    pub ty: Type,
    pub decls: Vec<Decl>,
}

#[derive(Debug)]
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

#[derive(Debug)]
pub enum GenArg {
    Type(Type),
    Expr(Expr),
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug)]
pub enum StrPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug)]
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

#[derive(Debug)]
pub struct Arg {
    pub label: Option<String>,
    pub mode: Mode,
    pub value: Expr,
}

#[derive(Debug)]
pub enum Cond {
    Expr(Expr),
    Let(Pattern, Expr),
}

#[derive(Debug)]
pub struct Arm {
    pub pat: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug)]
pub enum Stmt {
    Let { mutable: bool, pat: Pattern, ty: Option<Type>, value: Option<Expr> },
    Assign { target: Expr, op: &'static str, value: Expr },
    Defer(Expr),
    Expr(Expr),
    Decl(Decl),
}

#[derive(Debug)]
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
