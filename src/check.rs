//! Type checker for the core language.
//!
//! Checked: primitives, structs, enums, tuples, slices, optionals, results,
//! functions, methods of non-generic `impl` blocks, `let`/`var`, assignment
//! and mutability, parameter modes at call sites, `if`/`match` (with one-level
//! exhaustiveness), loops, `?`, `catch` and ignored results.
//!
//! Not checked yet, and reported as such instead of silently accepted:
//! generics, traits, imported modules, closures without parameter types and
//! declarations nested in blocks. Anything that touches those gets the type
//! `Unknown`, which is compatible with everything.

use std::collections::HashMap;

use crate::ast::*;
use crate::lexer::{Error, Span};

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int(&'static str),
    Float(&'static str),
    /// An integer literal that has not been given a concrete type yet.
    IntLit,
    FloatLit,
    Bool,
    Char,
    Str,
    Unit,
    /// Type of expressions that do not produce a value (`return`, `break`).
    Never,
    /// Something the checker does not model yet. Compatible with everything.
    Unknown,
    /// A struct with its type arguments.
    Struct(usize, Vec<Ty>),
    Enum(usize, Vec<Ty>),
    Tuple(Vec<Ty>),
    Slice(Box<Ty>),
    /// Fixed-size array. The length is not tracked until `comptime` exists.
    Array(Box<Ty>),
    Range(Box<Ty>),
    Optional(Box<Ty>),
    Result(Box<Ty>, Box<Ty>),
    Fn(Box<Sig>),
    /// A type provided by a built-in module, such as `io.Error`.
    Builtin(&'static str),
    /// A type parameter, inside the definition of a generic function or trait.
    Param(String),
    /// `dyn Trait`: a value of any type that implements the trait.
    Dyn(usize, Vec<Ty>),
    /// What `spawn` gives: a task whose result has this type.
    Task(Box<Ty>),
    /// A channel between tasks, of values of that type.
    Chan(Box<Ty>),
    /// A value shared between tasks, that one of them changes at a time.
    Mutex(Box<Ty>),
    /// What a `comptime` parameter is in one copy of its function: the
    /// number the call gave.
    Value(i128),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sig {
    pub params: Vec<ParamSig>,
    /// Already wrapped in `Result` when the function declares an error.
    pub ret: Ty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParamSig {
    pub name: String,
    pub mode: Mode,
    pub ty: Ty,
    pub optional: bool,
}

fn plain_param(name: &str, ty: Ty) -> ParamSig {
    ParamSig { name: name.into(), mode: Mode::Let, ty, optional: false }
}

/// What is known of a type: its name, what it is, the name and the type
/// of each field of a struct, and the same for each variant of an enum.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub name: String,
    pub kind: &'static str,
    pub fields: Vec<(String, String)>,
    pub variants: Vec<(String, Vec<(String, String)>)>,
}

pub struct Report {
    pub checked_fns: usize,
    pub skipped_decls: Vec<String>,
    pub skipped_exprs: usize,
}

/// Identity of an expression node: its address, stable while the tree is borrowed.
pub fn id(e: &Expr) -> usize {
    e as *const Expr as usize
}

/// What a name, field access or call refers to.
#[derive(Debug, Clone, PartialEq)]
pub enum Res {
    /// A function of a C header: index in `Info::externs`.
    Extern(usize),
    Local,
    NoneValue,
    Fn(usize),
    Const(usize),
    /// A unit variant used as a value, or a variant constructor call.
    Variant(usize, usize),
    /// A struct field or tuple element, by position.
    Field(usize),
    Len,
    SomeCtor,
    StructCtor(usize),
    /// `Type.name(...)`: (type name, function name).
    Assoc(String, String),
    /// `value.name(...)`: (type name, method name).
    Method(String, String),
    Builtin(&'static str),
    /// A call to a generic function: index in `Info::instances`.
    Generic(usize),
    /// A method called on a `dyn Trait` value: its position among the
    /// methods the value carries, and whether it takes `mut self`.
    Dyn(usize, bool),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// Index in `fns`.
    Fn(usize),
    /// Key in `methods`.
    Method(String, String),
}

pub struct Instance {
    pub f: Target,
    pub targs: Vec<Ty>,
    pub shown: String,
    pub sig: Sig,
}

pub struct TraitInfo {
    pub name: String,
    pub methods: Vec<(String, Sig, Option<Mode>)>,
    /// Methods the trait gives a body to: name, then the index of the
    /// trait in `file.decls` and of the method inside it.
    pub defaults: Vec<(String, usize, usize)>,
    /// The traits a type must also implement, in terms of `tparams`.
    pub supers: Vec<Bound>,
    /// The type parameters of a generic trait: `T` in `Into[T]`.
    pub tparams: Vec<String>,
    /// The methods that return `ref`, and whether it is `ref mut`.
    pub refs: Vec<(String, bool)>,
    /// The methods with type parameters of their own, and those parameters.
    pub own: Vec<(String, TParams)>,
}

/// A trait with its type arguments: `Into[str]`.
pub type Bound = (usize, Vec<Ty>);

/// Type parameters with the traits each must implement.
pub type TParams = Vec<(String, Vec<Bound>)>;

/// One `impl Trait for Type`.
#[derive(Clone)]
struct ImplInfo {
    /// The arguments of the trait, in terms of the parameters of the `impl`.
    targs: Vec<Ty>,
    self_ty: Ty,
    tparams: TParams,
}

/// What the name of a method becomes when an `impl` of a generic trait
/// gives it: a type can implement `Into[str]` and `Into[int]`, each with
/// its own `into`.
fn suffix_of(traits: &[TraitInfo], shown: &str, b: &Bound) -> String {
    if traits[b.0].tparams.is_empty() {
        String::new()
    } else {
        format!("@{shown}")
    }
}

fn trait_env(traits: &[TraitInfo], b: &Bound) -> HashMap<String, Ty> {
    traits[b.0].tparams.iter().cloned().zip(b.1.iter().cloned()).collect()
}

/// A trait and everything it requires, directly or not.
pub fn with_supers(traits: &[TraitInfo], b: &Bound) -> Vec<Bound> {
    let mut all = vec![b.clone()];
    let mut k = 0;
    while k < all.len() {
        let env = trait_env(traits, &all[k]);
        for s in &traits[all[k].0].supers {
            let s = (s.0, s.1.iter().map(|t| subst(t, &env)).collect());
            if !all.contains(&s) {
                all.push(s);
            }
        }
        k += 1;
    }
    all
}

/// The methods a `dyn Trait` value carries, in order: those of the trait
/// and of what it requires that take `self` and name `Self` nowhere else.
pub fn dyn_slots(traits: &[TraitInfo], bound: &Bound) -> Vec<(String, Sig, Option<Mode>)> {
    let unit: HashMap<String, Ty> = [("Self".to_string(), Ty::Unit)].into();
    let mut slots: Vec<(String, Sig, Option<Mode>)> = Vec::new();
    for b in with_supers(traits, bound) {
        let env = trait_env(traits, &b);
        for m in &traits[b.0].methods {
            let sig = subst_sig(&m.1, &env);
            let generic = traits[b.0].own.iter().any(|o| o.0 == m.0);
            if m.2.is_some() && !generic && subst_sig(&sig, &unit) == sig && !slots.iter().any(|s| s.0 == m.0) {
                slots.push((m.0.clone(), sig, m.2));
            }
        }
    }
    slots
}

/// The declaration a method is checked and lowered from.
pub fn method_decl<'a>(file: &'a File, m: &Method) -> Option<&'a Decl> {
    match (m.default, &file.decls[m.default.map_or(m.at.0, |d| d.0)].kind) {
        (Some((_, mi)), DeclKind::Trait(t)) => t.methods.get(mi),
        (None, DeclKind::Impl(im)) => im.decls.get(m.at.1),
        _ => None,
    }
}

/// Instance number used while checking a generic definition.
const DEFINITION: usize = usize::MAX;

/// Everything later stages need from type checking.
pub struct Info {
    /// Type of every checked expression, before any coercion.
    /// The key is (instance, expression): instance 0 is ordinary code and
    /// instance `k + 1` is the body of `instances[k]`.
    pub types: HashMap<(usize, usize), Ty>,
    /// Type an expression is coerced to where it is used, when one is required.
    pub wants: HashMap<(usize, usize), Ty>,
    pub res: HashMap<(usize, usize), Res>,
    /// Generic functions with their type arguments, one per distinct use.
    pub instances: Vec<Instance>,
    /// Index in `fns` to index in `file.decls`.
    pub decl_of_fn: Vec<usize>,
    pub structs: Vec<StructInfo>,
    pub enums: Vec<EnumInfo>,
    pub fns: Vec<FnInfo>,
    /// Index in `file.decls` to index in `fns`.
    pub fn_of_decl: HashMap<usize, usize>,
    /// Index in `file.decls` of an `impl` to the name of its type.
    pub impl_key: HashMap<usize, String>,
    /// Index in `file.decls` of an `impl` of a generic trait to what it
    /// adds to the names of its methods in `methods`.
    pub impl_suffix: HashMap<usize, String>,
    pub methods: HashMap<(String, String), Method>,
    /// For each `value as dyn Trait`: the methods the value carries, in
    /// order, each a `Res::Method` or a `Res::Generic`.
    pub dyns: HashMap<(usize, usize), Vec<Res>>,
    /// The C functions the program declares.
    pub externs: Vec<Extern>,
    /// The structs marked `@c`, and the type of C each one is.
    pub cstructs: HashMap<usize, String>,
    pub traits: Vec<TraitInfo>,
    /// Calls of functions that return `ref mut` whose result is written to.
    pub ref_writes: std::collections::HashSet<(usize, usize)>,
    /// What each `meta.info(T)` gives, by the instance and the call.
    pub metas: HashMap<(usize, usize), Meta>,
}

#[cfg(test)]
pub fn check(file: &File) -> Result<Report, Vec<Error>> {
    analyze(file).map(|(report, _)| report)
}

fn checker(file: &File) -> Checker<'_> {
    Checker {
        decls_only: false,
        types: HashMap::new(),
        wants: HashMap::new(),
        res: HashMap::new(),
        inst: 0,
        tenv: HashMap::new(),
        bounds: HashMap::new(),
        traits: Vec::new(),
        trait_impls: HashMap::new(),
        overloads: HashMap::new(),
        impl_suffix: HashMap::new(),
        active: Vec::new(),
        pre_args: None,
        hint: None,
        alias_depth: 0,
        pending: None,
        instances: Vec::new(),
        dyns: HashMap::new(),
        externs: Vec::new(),
        extern_ids: HashMap::new(),
        foreign_todo: Vec::new(),
        metas: HashMap::new(),
        cstructs: HashMap::new(),
        unsafe_depth: 0,
        ref_calls: HashMap::new(),
        ref_writes: Default::default(),
        var_of: HashMap::new(),
        instance_ids: HashMap::new(),
        decl_of_fn: Vec::new(),
        closures: Vec::new(),
        next_var: 0,
        mut_log: Vec::new(),
        frozen: Vec::new(),
        receiver: None,
        arg_logs: None,
        imports: HashMap::new(),
        file,
        structs: Vec::new(),
        enums: Vec::new(),
        fns: Vec::new(),
        consts: Vec::new(),
        const_exprs: Vec::new(),
        aliases: HashMap::new(),
        module: 0,
        module_of: Vec::new(),
        mods: Vec::new(),
        public: std::collections::HashSet::new(),
        globals: HashMap::new(),
        methods: HashMap::new(),
        fn_of_decl: HashMap::new(),
        impl_ty: HashMap::new(),
        errors: Vec::new(),
        report: Report { checked_fns: 0, skipped_decls: Vec::new(), skipped_exprs: 0 },
        scopes: Vec::new(),
        ctx: Vec::new(),
        self_ty: None,
        cur: Span::default(),
    }
}

pub fn analyze(file: &File) -> Result<(Report, Info), Vec<Error>> {
    let mut c = checker(file);
    c.run();
    if c.errors.is_empty() {
        let impl_key = c.impl_ty.iter().map(|(i, t)| (*i, c.key(t))).collect();
        let info = Info {
            types: c.types,
            wants: c.wants,
            res: c.res,
            instances: c.instances,
            decl_of_fn: c.decl_of_fn,
            structs: c.structs,
            enums: c.enums,
            fns: c.fns,
            fn_of_decl: c.fn_of_decl,
            impl_key,
            impl_suffix: c.impl_suffix,
            methods: c.methods,
            dyns: c.dyns,
            externs: c.externs,
            cstructs: c.cstructs,
            metas: c.metas,
            traits: c.traits,
            ref_writes: c.ref_writes,
        };
        Ok((c.report, info))
    } else {
        c.errors.sort_by_key(|e| (e.span.line, e.span.col));
        // A generic body is checked once per instance: report each mistake once.
        c.errors.dedup_by(|a, b| a.span.line == b.span.line && a.span.col == b.span.col && a.msg == b.msg);
        Err(c.errors)
    }
}

/// What the declarations of a program say, without looking into any
/// expression: the names of each module, the fields of the types, the
/// signatures of functions and methods, and what implements each trait.
/// The checker written in Mou (`compiler/check.mou`) prints the same.
pub fn decls_dump(file: &File) -> String {
    use std::fmt::Write;
    let mut c = checker(file);
    c.decls_only = true;
    c.run();
    c.errors.sort_by_key(|e| (e.span.line, e.span.col));
    c.errors.dedup_by(|a, b| a.span.line == b.span.line && a.span.col == b.span.col && a.msg == b.msg);
    let text = |t: &Ty| plain_text(t);
    let list = |ts: &[Ty]| ts.iter().map(plain_text).collect::<Vec<_>>().join(", ");
    let mode = |m: Mode| match m {
        Mode::Let => "let",
        Mode::Mut => "mut",
        Mode::Own => "own",
    };
    let self_mode = |m: Option<Mode>| m.map_or("_", mode);
    let sig = |s: &Sig| {
        let ps: Vec<String> = s.params.iter().map(|p| format!("{} {}: {} {}", mode(p.mode), p.name, text(&p.ty), p.optional)).collect();
        format!("({}) -> {}", ps.join(", "), text(&s.ret))
    };
    let bound = |b: &Bound| format!("T{}[{}]", b.0, list(&b.1));
    let bounds = |bs: &[Bound]| bs.iter().map(bound).collect::<Vec<_>>().join(" + ");
    let tparams = |ts: &TParams| ts.iter().map(|(n, bs)| format!("{n}: {}", bounds(bs))).collect::<Vec<_>>().join(", ");
    let mut out = String::new();
    for e in &c.errors {
        let _ = writeln!(out, "error {}:{} {}", e.span.line, e.span.col, e.msg);
    }
    for (m, name) in c.mods.iter().enumerate() {
        let _ = writeln!(out, "module {m} {}", if name.is_empty() { "-" } else { name });
    }
    let mut globals: Vec<_> = c.globals.iter().collect();
    globals.sort_by(|a, b| a.0.cmp(b.0));
    for ((m, name), g) in globals {
        let what = match g {
            Global::Struct(i) => format!("struct {i}"),
            Global::Enum(i) => format!("enum {i}"),
            Global::Fn(i) => format!("fn {i}"),
            Global::Const(i) => format!("const {i}"),
            Global::Alias => "alias".to_string(),
            Global::Trait(i) => format!("trait {i}"),
            Global::Import => "import".to_string(),
            Global::Module(i) => format!("module {i}"),
        };
        let vis = if c.public.contains(&(*m, name.clone())) { "pub" } else { "priv" };
        let _ = writeln!(out, "global {m} {name} {vis} {what}");
    }
    let mut imports: Vec<_> = c.imports.iter().collect();
    imports.sort();
    for ((m, name), path) in imports {
        let _ = writeln!(out, "import {m} {name} {path}");
    }
    let mut ids: Vec<_> = c.extern_ids.iter().collect();
    ids.sort();
    for ((m, alias, name), i) in ids {
        let _ = writeln!(out, "externid {m} {alias} {name} {i}");
    }
    for x in &c.externs {
        let _ = writeln!(out, "extern {} {} {}", x.name, x.header, sig(&x.sig));
    }
    for (i, s) in c.structs.iter().enumerate() {
        let _ = writeln!(out, "struct {i} {} {} {} [{}]", s.name, s.module, s.generic, s.tparams.join(", "));
        for f in &s.fields {
            let _ = writeln!(out, "  field {} {} {}", f.name, text(&f.ty), f.optional);
        }
    }
    for (i, e) in c.enums.iter().enumerate() {
        let _ = writeln!(out, "enum {i} {} {} {} [{}]", e.name, e.module, e.generic, e.tparams.join(", "));
        for v in &e.variants {
            let fields: Vec<String> = v.fields.iter().map(|(n, t)| format!("{}: {}", n.as_deref().unwrap_or("_"), text(t))).collect();
            let _ = writeln!(out, "  variant {} ({})", v.name, fields.join(", "));
        }
    }
    for (i, f) in c.fns.iter().enumerate() {
        let _ = writeln!(out, "fn {i} {} {} [{}] {}", c.decl_of_fn[i], f.generic, tparams(&f.tparams), sig(&f.sig));
    }
    for (i, t) in c.traits.iter().enumerate() {
        let _ = writeln!(out, "trait {i} {} [{}] {}", t.name, t.tparams.join(", "), bounds(&t.supers));
        for m in &t.methods {
            let _ = writeln!(out, "  method {} {} {}", m.0, self_mode(m.2), sig(&m.1));
        }
        for d in &t.defaults {
            let _ = writeln!(out, "  default {} {} {}", d.0, d.1, d.2);
        }
        for r in &t.refs {
            let _ = writeln!(out, "  ref {} {}", r.0, r.1);
        }
        for o in &t.own {
            let _ = writeln!(out, "  own {} [{}]", o.0, tparams(&o.1));
        }
    }
    let mut methods: Vec<_> = c.methods.iter().collect();
    methods.sort_by(|a, b| a.0.cmp(b.0));
    for ((key, name), m) in methods {
        let at = if m.at.1 == usize::MAX { "-1".to_string() } else { m.at.1.to_string() };
        let default = m.default.map_or("_".to_string(), |d| format!("{}.{}", d.0, d.1));
        let env: Vec<String> = m.trait_env.iter().map(|(n, t)| format!("{n}={}", text(t))).collect();
        let of = m.of_trait.as_ref().map_or("_".to_string(), bound);
        let _ = writeln!(out, "method {key} {name} {} [{}] {} at={}.{at} self={} default={default} env={} of={of}", self_mode(m.self_mode), tparams(&m.tparams), sig(&m.sig), m.at.0, text(&m.self_ty), env.join(","));
    }
    let mut impls: Vec<_> = c.impl_ty.iter().collect();
    impls.sort_by_key(|x| *x.0);
    for (di, t) in impls {
        let _ = writeln!(out, "impl {di} {} {}", c.key(t), text(t));
    }
    let mut suffixes: Vec<_> = c.impl_suffix.iter().collect();
    suffixes.sort();
    for (di, s) in suffixes {
        let _ = writeln!(out, "suffix {di} {}", if s.is_empty() { "-" } else { s });
    }
    let mut impls: Vec<_> = c.trait_impls.iter().collect();
    impls.sort_by(|a, b| a.0.cmp(b.0));
    for ((t, key), infos) in impls {
        for info in infos {
            let _ = writeln!(out, "implements {t} {key} [{}] {} [{}]", list(&info.targs), text(&info.self_ty), tparams(&info.tparams));
        }
    }
    let mut overloads: Vec<_> = c.overloads.iter().collect();
    overloads.sort_by(|a, b| a.0.cmp(b.0));
    for ((key, name), all) in overloads {
        for (mangled, b) in all {
            let _ = writeln!(out, "overload {key} {name} {mangled} {}", bound(b));
        }
    }
    let mut aliases: Vec<_> = c.aliases.iter().collect();
    aliases.sort_by(|a, b| a.0.cmp(b.0));
    for ((m, name), a) in aliases {
        let state = match a {
            Alias::Todo(i) => format!("todo {i}"),
            Alias::Busy => "busy".to_string(),
            Alias::Done(t) => format!("done {}", text(t)),
        };
        let _ = writeln!(out, "alias {m} {name} {state}");
    }
    for s in &c.report.skipped_decls {
        let _ = writeln!(out, "skipped {s}");
    }
    let _ = writeln!(out, "skipped_exprs {}", c.report.skipped_exprs);
    out
}

/// Every expression of a program, each one before those inside it.
fn all_exprs(file: &File) -> Vec<&Expr> {
    fn pat<'a>(p: &'a Pattern, out: &mut Vec<&'a Expr>) {
        match p {
            Pattern::Literal(e) => expr(e, out),
            Pattern::Range { lo, hi, .. } => {
                expr(lo, out);
                if let Some(h) = hi {
                    expr(h, out);
                }
            }
            Pattern::Ctor { args, .. } | Pattern::Tuple(args) | Pattern::Slice(args) | Pattern::Or(args) => args.iter().for_each(|p| pat(p, out)),
            Pattern::Struct { fields, .. } => fields.iter().filter_map(|f| f.1.as_ref()).for_each(|p| pat(p, out)),
            Pattern::Wildcard | Pattern::Bind { .. } | Pattern::Rest => {}
        }
    }
    fn cond<'a>(c: &'a Cond, out: &mut Vec<&'a Expr>) {
        match c {
            Cond::Expr(e) => expr(e, out),
            Cond::Let(p, e) => {
                pat(p, out);
                expr(e, out);
            }
        }
    }
    fn block<'a>(b: &'a Block, out: &mut Vec<&'a Expr>) {
        for s in &b.stmts {
            match s {
                Stmt::Let { pat: p, value, .. } => {
                    pat(p, out);
                    if let Some(v) = value {
                        expr(v, out);
                    }
                }
                Stmt::Assign { target, value, .. } => {
                    expr(target, out);
                    expr(value, out);
                }
                Stmt::Defer(e) | Stmt::Expr(e) => expr(e, out),
                Stmt::Decl(_) => {}
            }
        }
        if let Some(t) = &b.tail {
            expr(t, out);
        }
    }
    fn expr<'a>(e: &'a Expr, out: &mut Vec<&'a Expr>) {
        out.push(e);
        match &e.kind {
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Char(_) | ExprKind::Bool(_) | ExprKind::Path(_) | ExprKind::Continue => {}
            ExprKind::Str(parts) => {
                for p in parts {
                    if let StrPart::Expr(inner) = p {
                        expr(inner, out);
                    }
                }
            }
            ExprKind::Tuple(items) | ExprKind::Array(items) => items.iter().for_each(|i| expr(i, out)),
            ExprKind::ArrayRepeat(a, b) => {
                expr(a, out);
                expr(b, out);
            }
            ExprKind::Block(b) | ExprKind::Loop(b) | ExprKind::Unsafe(b) => block(b, out),
            ExprKind::If { cond: c, then, els } => {
                cond(c, out);
                block(then, out);
                if let Some(x) = els {
                    expr(x, out);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                expr(scrutinee, out);
                for arm in arms {
                    pat(&arm.pat, out);
                    if let Some(g) = &arm.guard {
                        expr(g, out);
                    }
                    expr(&arm.body, out);
                }
            }
            ExprKind::For { pat: p, iter, body } => {
                pat(p, out);
                expr(iter, out);
                block(body, out);
            }
            ExprKind::While { cond: c, body } => {
                cond(c, out);
                block(body, out);
            }
            ExprKind::Closure { body, .. } | ExprKind::Scope { body, .. } => block(body, out),
            ExprKind::With { bindings, body } => {
                bindings.iter().for_each(|b| expr(&b.1, out));
                block(body, out);
            }
            ExprKind::Comptime(x) | ExprKind::Try(x) => expr(x, out),
            ExprKind::Return(x) | ExprKind::Break(x) => {
                if let Some(x) = x {
                    expr(x, out);
                }
            }
            ExprKind::Unary { expr: x, .. } | ExprKind::Cast { expr: x, .. } | ExprKind::Field { base: x, .. } => expr(x, out),
            ExprKind::Binary { lhs, rhs, .. } => {
                expr(lhs, out);
                expr(rhs, out);
            }
            ExprKind::Range { lo, hi, .. } => {
                for x in [lo, hi].into_iter().flatten() {
                    expr(x, out);
                }
            }
            ExprKind::Call { callee, args } => {
                expr(callee, out);
                args.iter().for_each(|a| expr(&a.value, out));
            }
            ExprKind::Index { base, args } => {
                expr(base, out);
                args.iter().for_each(|a| expr(a, out));
            }
            ExprKind::Catch { expr: x, handler, .. } => {
                expr(x, out);
                block(handler, out);
            }
        }
    }
    fn decl<'a>(d: &'a Decl, out: &mut Vec<&'a Expr>) {
        match &d.kind {
            DeclKind::Fn(f) => {
                f.params.iter().filter_map(|p| p.default.as_ref()).for_each(|e| expr(e, out));
                f.where_.iter().for_each(|e| expr(e, out));
                if let Some(b) = &f.body {
                    expr(b, out);
                }
            }
            DeclKind::Struct(s) => s.fields.iter().filter_map(|f| f.default.as_ref()).for_each(|e| expr(e, out)),
            DeclKind::Enum(e) => e.variants.iter().filter_map(|v| v.value.as_ref()).for_each(|e| expr(e, out)),
            DeclKind::Trait(t) => t.methods.iter().for_each(|m| decl(m, out)),
            DeclKind::Impl(i) => i.decls.iter().for_each(|m| decl(m, out)),
            DeclKind::Const { value, .. } => expr(value, out),
            DeclKind::TypeAlias { .. } => {}
        }
    }
    let mut out = Vec::new();
    file.decls.iter().for_each(|d| decl(d, &mut out));
    out
}

/// What the checker finds out about every expression of a program: its
/// type, the type it is used as and what it refers to, for ordinary code
/// and for each instance of a generic function. The checker written in
/// Mou (`compiler/check.mou`) prints the same.
pub fn info_dump(file: &File) -> String {
    use std::fmt::Write;
    let mut c = checker(file);
    c.run();
    c.errors.sort_by_key(|e| (e.span.line, e.span.col));
    c.errors.dedup_by(|a, b| a.span.line == b.span.line && a.span.col == b.span.col && a.msg == b.msg);
    let text = plain_text;
    let list = |ts: &[Ty]| ts.iter().map(plain_text).collect::<Vec<_>>().join(", ");
    let mode = |m: Mode| match m {
        Mode::Let => "let",
        Mode::Mut => "mut",
        Mode::Own => "own",
    };
    let sig = |s: &Sig| {
        let ps: Vec<String> = s.params.iter().map(|p| format!("{} {}: {} {}", mode(p.mode), p.name, text(&p.ty), p.optional)).collect();
        format!("({}) -> {}", ps.join(", "), text(&s.ret))
    };
    let res = |r: &Res| match r {
        Res::Extern(i) => format!("extern {i}"),
        Res::Local => "local".to_string(),
        Res::NoneValue => "none".to_string(),
        Res::Fn(i) => format!("fn {i}"),
        Res::Const(i) => format!("const {i}"),
        Res::Variant(i, v) => format!("variant {i}.{v}"),
        Res::Field(k) => format!("field {k}"),
        Res::Len => "len".to_string(),
        Res::SomeCtor => "some".to_string(),
        Res::StructCtor(i) => format!("struct {i}"),
        Res::Assoc(t, n) => format!("assoc {t}.{n}"),
        Res::Method(t, n) => format!("method {t}.{n}"),
        Res::Builtin(s) => format!("builtin {s}"),
        Res::Generic(k) => format!("generic {k}"),
        Res::Dyn(k, m) => format!("dyn {k} {m}"),
    };
    let mut out = String::new();
    for e in &c.errors {
        let _ = writeln!(out, "error {}:{} {}", e.span.line, e.span.col, e.msg);
    }
    let _ = writeln!(out, "checked {} skipped_exprs {}", c.report.checked_fns, c.report.skipped_exprs);
    for s in &c.report.skipped_decls {
        let _ = writeln!(out, "skipped {s}");
    }
    for (i, t) in c.consts.iter().enumerate() {
        let _ = writeln!(out, "const {i} {}", t.as_ref().map_or("_".to_string(), text));
    }
    for (k, i) in c.instances.iter().enumerate() {
        let target = match &i.f {
            Target::Fn(f) => format!("fn {f}"),
            Target::Method(t, n) => format!("method {t}.{n}"),
        };
        let _ = writeln!(out, "instance {k} {target} [{}] <{}> {}", list(&i.targs), i.shown, sig(&i.sig));
    }
    let exprs = all_exprs(file);
    let seq: HashMap<usize, usize> = exprs.iter().enumerate().map(|(k, e)| (id(e), k)).collect();
    let slot = |inst: usize| if inst == DEFINITION { 0 } else { inst + 1 };
    let mut keys = std::collections::BTreeSet::new();
    let mut unseen = 0;
    let all = c.types.keys().chain(c.wants.keys()).chain(c.res.keys()).chain(c.dyns.keys()).chain(c.ref_writes.iter());
    for (inst, e) in all {
        match seq.get(e) {
            Some(s) => {
                keys.insert((slot(*inst), *s));
            }
            None => unseen += 1,
        }
    }
    for (sl, s) in keys {
        let e = exprs[s];
        let key = (if sl == 0 { DEFINITION } else { sl - 1 }, id(e));
        let t = c.types.get(&key).map_or("-".to_string(), text);
        let w = c.wants.get(&key).map_or("-".to_string(), text);
        let r = c.res.get(&key).map_or("-".to_string(), res);
        let d = c.dyns.get(&key).map_or("-".to_string(), |t| t.iter().map(res).collect::<Vec<_>>().join("; "));
        let _ = writeln!(out, "e {sl} {s} {}:{} t={t} w={w} r={r} d={d} rw={}", e.span.line, e.span.col, c.ref_writes.contains(&key));
    }
    let _ = writeln!(out, "unseen {unseen}");
    out
}

/// A type as text, without the names of the program.
fn plain_text(t: &Ty) -> String {
    use Ty::*;
    let text = plain_text;
    let list = |ts: &[Ty]| ts.iter().map(text).collect::<Vec<_>>().join(", ");
    match t {
        Int(k) | Float(k) | Builtin(k) => k.to_string(),
        IntLit => "intlit".into(),
        FloatLit => "floatlit".into(),
        Bool => "bool".into(),
        Char => "char".into(),
        Str => "str".into(),
        Unit => "()".into(),
        Never => "never".into(),
        Unknown => "_".into(),
        Struct(i, ts) => format!("S{i}[{}]", list(ts)),
        Enum(i, ts) => format!("E{i}[{}]", list(ts)),
        Dyn(i, ts) => format!("dyn{i}[{}]", list(ts)),
        Tuple(ts) => format!("({})", list(ts)),
        Slice(t) => format!("[{}]", text(t)),
        Array(t) => format!("[{}; _]", text(t)),
        Value(v) => format!("={v}"),
        Range(t) => format!("range {}", text(t)),
        Task(t) => format!("task {}", text(t)),
        Chan(t) => format!("chan {}", text(t)),
        Mutex(t) => format!("mutex {}", text(t)),
        Optional(t) => format!("{}?", text(t)),
        Result(t, e) => format!("{} ! {}", text(t), text(e)),
        Param(n) => format!("'{n}"),
        Fn(sig) => {
            let ps: Vec<String> = sig
                .params
                .iter()
                .map(|p| {
                    let mode = match p.mode {
                        Mode::Let => "let",
                        Mode::Mut => "mut",
                        Mode::Own => "own",
                    };
                    format!("{mode} {}", text(&p.ty))
                })
                .collect();
            format!("fn({}) -> {}", ps.join(", "), text(&sig.ret))
        }
    }
}

const INT_KINDS: &[&str] =
    &["i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "isize", "usize"];

fn prim(name: &str) -> Option<Ty> {
    Some(match name {
        "int" => Ty::Int("i64"),
        "float" | "f64" => Ty::Float("f64"),
        "f32" => Ty::Float("f32"),
        "bool" => Ty::Bool,
        "char" => Ty::Char,
        "str" => Ty::Str,
        "ptr" => Ty::Builtin("ptr"),
        _ => Ty::Int(INT_KINDS.iter().find(|k| **k == name)?),
    })
}

fn is_int(t: &Ty) -> bool {
    matches!(t, Ty::Int(_) | Ty::IntLit | Ty::Unknown)
}

fn is_num(t: &Ty) -> bool {
    matches!(t, Ty::Int(_) | Ty::IntLit | Ty::Float(_) | Ty::FloatLit | Ty::Unknown)
}

fn boxed(t: Option<Ty>) -> Option<Box<Ty>> {
    t.map(Box::new)
}

/// The common type of two expressions that must agree (branches, operands).
pub fn join(a: &Ty, b: &Ty) -> Option<Ty> {
    use Ty::*;
    Some(match (a, b) {
        (Unknown, _) | (_, Unknown) => Unknown,
        (Never, x) | (x, Never) => x.clone(),
        (IntLit, IntLit) => IntLit,
        (IntLit, Int(k)) | (Int(k), IntLit) => Int(k),
        (Int(x), Int(y)) if widens(x, y) => Int(y),
        (Int(x), Int(y)) if widens(y, x) => Int(x),
        (IntLit, Float(k)) | (Float(k), IntLit) => Float(k),
        (FloatLit, Float(k)) | (Float(k), FloatLit) => Float(k),
        (IntLit, FloatLit) | (FloatLit, IntLit) | (FloatLit, FloatLit) => FloatLit,
        (Optional(x), Optional(y)) => Optional(boxed(join(x, y))?),
        (Optional(x), y) | (y, Optional(x)) => Optional(boxed(join(x, y))?),
        (Tuple(x), Tuple(y)) if x.len() == y.len() => {
            Tuple(x.iter().zip(y).map(|(p, q)| join(p, q)).collect::<Option<Vec<_>>>()?)
        }
        (Slice(x), Slice(y)) | (Array(x), Slice(y)) | (Slice(x), Array(y)) => Slice(boxed(join(x, y))?),
        (Array(x), Array(y)) => Array(boxed(join(x, y))?),
        (Range(x), Range(y)) => Range(boxed(join(x, y))?),
        (Result(x, e), Result(y, f)) => Result(boxed(join(x, y))?, boxed(join(e, f))?),
        (Fn(x), Fn(y)) if fn_compatible(x, y) => a.clone(),
        (Struct(i, x), Struct(j, y)) if i == j && same_args(x, y) => Struct(*i, fill(x, y)),
        (Enum(i, x), Enum(j, y)) if i == j && same_args(x, y) => Enum(*i, fill(x, y)),
        _ if a == b => a.clone(),
        _ => return None,
    })
}

/// Whether a value of type `act` can be used where `exp` is required.
/// Whether every value of one integer type is a value of another, so
/// that it converts without `as`: `u8` to `int`, but not `int` to `u8`.
pub fn widens(from: &str, to: &str) -> bool {
    let (a, b) = (crate::mir::bounds(from), crate::mir::bounds(to));
    // What a `usize` holds depends on the machine: it only converts to itself.
    let fixed = |k: &str| !k.ends_with("size");
    from == to || (fixed(from) && fixed(to) && b.0 <= a.0 && a.1 <= b.1)
}

pub fn assignable(exp: &Ty, act: &Ty) -> bool {
    use Ty::*;
    match (exp, act) {
        (_, Never) | (_, Unknown) | (Unknown, _) => true,
        (Int(_), IntLit) | (Float(_), IntLit) | (Float(_), FloatLit) => true,
        (Int(to), Int(from)) => widens(from, to),
        (IntLit | FloatLit, _) => join(exp, act).is_some(),
        (Optional(e), Optional(a)) => assignable(e, a),
        (Optional(e), a) => assignable(e, a),
        (Tuple(e), Tuple(a)) => e.len() == a.len() && e.iter().zip(a).all(|(p, q)| assignable(p, q)),
        (Slice(e), Slice(a)) | (Slice(e), Array(a)) | (Array(e), Array(a)) | (Range(e), Range(a)) => {
            assignable(e, a)
        }
        (Result(e, x), Result(a, y)) => assignable(e, a) && assignable(x, y),
        (Fn(e), Fn(a)) => fn_compatible(e, a),
        (Struct(i, e), Struct(j, a)) | (Enum(i, e), Enum(j, a)) => i == j && same_args(e, a),
        _ => exp == act,
    }
}

/// Whether two types are the same once the parts nothing has decided yet
/// (the element of `[]`, the argument of `Tree.Leaf`) are filled in. Unlike
/// `assignable` it allows no conversion: the values are not touched.
fn same(a: &Ty, b: &Ty) -> bool {
    use Ty::*;
    match (a, b) {
        (Never | Unknown, _) | (_, Never | Unknown) => true,
        (Struct(i, x), Struct(j, y)) | (Enum(i, x), Enum(j, y)) => i == j && same_args(x, y),
        (Tuple(x), Tuple(y)) => same_args(x, y),
        (Slice(x), Slice(y)) | (Array(x), Array(y)) | (Range(x), Range(y)) | (Optional(x), Optional(y)) | (Task(x), Task(y)) | (Chan(x), Chan(y)) | (Mutex(x), Mutex(y)) => same(x, y),
        (Result(x, e), Result(y, f)) => same(x, y) && same(e, f),
        _ => a == b,
    }
}

fn same_args(a: &[Ty], b: &[Ty]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
}

/// The more decided of two types that are `same`.
fn fill_one(a: &Ty, b: &Ty) -> Ty {
    use Ty::*;
    let bx = |x: &Ty, y: &Ty| Box::new(fill_one(x, y));
    match (a, b) {
        (Never | Unknown, other) | (other, Never | Unknown) => other.clone(),
        (Struct(i, x), Struct(_, y)) => Struct(*i, fill(x, y)),
        (Enum(i, x), Enum(_, y)) => Enum(*i, fill(x, y)),
        (Tuple(x), Tuple(y)) => Tuple(fill(x, y)),
        (Slice(x), Slice(y)) => Slice(bx(x, y)),
        (Array(x), Array(y)) => Array(bx(x, y)),
        (Range(x), Range(y)) => Range(bx(x, y)),
        (Optional(x), Optional(y)) => Optional(bx(x, y)),
        (Result(x, e), Result(y, f)) => Result(bx(x, y), bx(e, f)),
        _ => a.clone(),
    }
}

fn fill(a: &[Ty], b: &[Ty]) -> Vec<Ty> {
    a.iter().zip(b).map(|(x, y)| fill_one(x, y)).collect()
}

/// Whether a type still has a part nothing has decided.
fn open(t: &Ty) -> bool {
    use Ty::*;
    match t {
        Never | Unknown => true,
        Struct(_, ts) | Enum(_, ts) | Tuple(ts) => ts.iter().any(open),
        Slice(t) | Array(t) | Range(t) | Optional(t) | Task(t) | Chan(t) | Mutex(t) => open(t),
        Result(t, e) => open(t) || open(e),
        _ => false,
    }
}

fn fn_compatible(a: &Sig, b: &Sig) -> bool {
    a.params.len() == b.params.len()
        && a.params.iter().zip(&b.params).all(|(p, q)| {
            p.mode == q.mode && assignable(&p.ty, &q.ty) && assignable(&q.ty, &p.ty)
        })
        && assignable(&a.ret, &b.ret)
}

/// Gives literal types their default concrete type (`int`, `f64`).
fn concretize(t: &Ty) -> Ty {
    use Ty::*;
    match t {
        IntLit => Int("i64"),
        FloatLit => Float("f64"),
        Tuple(ts) => Tuple(ts.iter().map(concretize).collect()),
        Slice(t) => Slice(Box::new(concretize(t))),
        Array(t) => Array(Box::new(concretize(t))),
        Range(t) => Range(Box::new(concretize(t))),
        Task(t) => Task(Box::new(concretize(t))),
        Chan(t) => Chan(Box::new(concretize(t))),
        Mutex(t) => Mutex(Box::new(concretize(t))),
        Optional(t) => Optional(Box::new(concretize(t))),
        Struct(i, ts) => Struct(*i, ts.iter().map(concretize).collect()),
        Enum(i, ts) => Enum(*i, ts.iter().map(concretize).collect()),
        other => other.clone(),
    }
}

/// Replaces type parameters by the types bound to them.
pub fn subst(t: &Ty, env: &HashMap<String, Ty>) -> Ty {
    use Ty::*;
    let b = |t: &Ty| Box::new(subst(t, env));
    match t {
        Param(n) => env.get(n).cloned().unwrap_or_else(|| t.clone()),
        Tuple(ts) => Tuple(ts.iter().map(|t| subst(t, env)).collect()),
        Slice(t) => Slice(b(t)),
        Array(t) => Array(b(t)),
        Range(t) => Range(b(t)),
        Task(t) => Task(b(t)),
        Chan(t) => Chan(b(t)),
        Mutex(t) => Mutex(b(t)),
        Optional(t) => Optional(b(t)),
        Result(t, e) => Result(b(t), b(e)),
        Fn(sig) => Fn(Box::new(subst_sig(sig, env))),
        Struct(i, ts) => Struct(*i, ts.iter().map(|t| subst(t, env)).collect()),
        Enum(i, ts) => Enum(*i, ts.iter().map(|t| subst(t, env)).collect()),
        Dyn(i, ts) => Dyn(*i, ts.iter().map(|t| subst(t, env)).collect()),
        other => other.clone(),
    }
}

/// The substitution that gives the type parameters `names` the types `args`.
pub fn targ_env(names: &[String], args: &[Ty]) -> HashMap<String, Ty> {
    names.iter().cloned().zip(args.iter().cloned()).collect()
}

fn subst_sig(sig: &Sig, env: &HashMap<String, Ty>) -> Sig {
    let params = sig.params.iter().map(|p| ParamSig { ty: subst(&p.ty, env), ..p.clone() }).collect();
    Sig { params, ret: subst(&sig.ret, env) }
}

/// Finds what the type parameters of `pat` must be for `actual` to fit.
fn bind(pat: &Ty, actual: &Ty, env: &mut HashMap<String, Ty>) {
    use Ty::*;
    match (pat, actual) {
        (_, Unknown | Never) => {}
        (Param(n), _) => {
            let t = match env.get(n) {
                Some(seen) => join(seen, actual).unwrap_or_else(|| seen.clone()),
                None => actual.clone(),
            };
            env.insert(n.clone(), t);
        }
        (Tuple(ps), Tuple(ts)) => ps.iter().zip(ts).for_each(|(p, t)| bind(p, t, env)),
        (Struct(i, ps), Struct(j, ts)) | (Enum(i, ps), Enum(j, ts)) | (Dyn(i, ps), Dyn(j, ts)) if i == j => {
            ps.iter().zip(ts).for_each(|(p, t)| bind(p, t, env))
        }
        (Slice(p) | Array(p), Slice(t) | Array(t)) | (Range(p), Range(t)) | (Optional(p), Optional(t)) | (Task(p), Task(t)) | (Chan(p), Chan(t)) | (Mutex(p), Mutex(t)) => bind(p, t, env),
        (Optional(p), t) => bind(p, t, env),
        (Result(p, pe), Result(t, te)) => {
            bind(p, t, env);
            bind(pe, te, env);
        }
        (Fn(p), Fn(t)) => {
            p.params.iter().zip(&t.params).for_each(|(a, b)| bind(&a.ty, &b.ty, env));
            bind(&p.ret, &t.ret, env);
        }
        _ => {}
    }
}

/// Binds type parameters from the type a call is expected to produce.
fn bind_hint(pat: &Ty, hint: &Ty, env: &mut HashMap<String, Ty>) {
    match (pat, hint) {
        (Ty::Result(ok, _), h) if !matches!(h, Ty::Result(..)) => bind_hint(ok, h, env),
        (p, Ty::Optional(h)) if !matches!(p, Ty::Optional(_) | Ty::Param(_)) => bind_hint(p, h, env),
        _ => bind(pat, hint, env),
    }
}

/// Completes what the arguments of a call decided with the type the call
/// is expected to produce. The arguments win: the expected type only says
/// what the result must be assignable to. It decides the parameters no
/// argument mentions and the type of the literals.
fn hinted(pat: &Ty, hint: &Ty, env: &mut HashMap<String, Ty>) {
    let mut wanted = HashMap::new();
    bind_hint(pat, hint, &mut wanted);
    for (name, want) in wanted {
        let t = match env.get(&name) {
            None => want,
            Some(got) if concretize(got) != *got && assignable(&want, got) => want,
            Some(got) if open(got) && same(got, &want) => fill_one(got, &want),
            Some(_) => continue,
        };
        env.insert(name, t);
    }
}

/// Whether a struct or enum inside the type has a type argument nothing decided.
fn undecided(t: &Ty) -> bool {
    use Ty::*;
    match t {
        Struct(_, ts) | Enum(_, ts) => ts.iter().any(open),
        Tuple(ts) => ts.iter().any(undecided),
        Slice(t) | Array(t) | Range(t) | Optional(t) | Task(t) | Chan(t) | Mutex(t) => undecided(t),
        Result(t, e) => undecided(t) || undecided(e),
        _ => false,
    }
}

/// Whether a type has no parameters or unknown parts left.
fn concrete(t: &Ty) -> bool {
    use Ty::*;
    match t {
        Param(_) | Unknown => false,
        Tuple(ts) | Struct(_, ts) | Enum(_, ts) | Dyn(_, ts) => ts.iter().all(concrete),
        Slice(t) | Array(t) | Range(t) | Optional(t) | Task(t) | Chan(t) | Mutex(t) => concrete(t),
        Result(t, e) => concrete(t) && concrete(e),
        Fn(sig) => sig.params.iter().all(|p| concrete(&p.ty)) && concrete(&sig.ret),
        _ => true,
    }
}

/// Runs the functions on types over a fixed list of types and prints what
/// they give. The same list goes through the ones written in Mou
/// (`compiler/types.mou`), and the two texts have to be equal.
pub fn types_selftest() -> String {
    use std::fmt::Write;
    use Ty::*;
    let text = plain_text;
    let b = |t: Ty| Box::new(t);
    let p = |n: &str| Param(n.to_string());
    let f = |params: Vec<(Mode, Ty)>, ret: Ty| {
        let params = params.into_iter().map(|(mode, ty)| ParamSig { name: "x".into(), mode, ty, optional: false }).collect();
        Fn(Box::new(Sig { params, ret }))
    };
    let int = Int("i64");
    let samples: Vec<Ty> = vec![
        int.clone(),
        Int("u8"),
        Int("i32"),
        Int("u64"),
        Int("usize"),
        Int("i128"),
        Int("u128"),
        IntLit,
        FloatLit,
        Float("f64"),
        Float("f32"),
        Bool,
        Char,
        Str,
        Unit,
        Never,
        Unknown,
        Struct(0, vec![]),
        Struct(1, vec![int.clone()]),
        Struct(1, vec![Unknown]),
        Struct(1, vec![IntLit]),
        Enum(0, vec![p("T")]),
        Enum(0, vec![Str]),
        Tuple(vec![IntLit, Str]),
        Tuple(vec![Int("u8"), Str]),
        Slice(b(int.clone())),
        Slice(b(IntLit)),
        Array(b(IntLit)),
        Array(b(Never)),
        Range(b(IntLit)),
        Range(b(Int("i32"))),
        Optional(b(int.clone())),
        Optional(b(Never)),
        Optional(b(Optional(b(IntLit)))),
        Result(b(int.clone()), b(Str)),
        Result(b(IntLit), b(Unknown)),
        f(vec![(Mode::Let, int.clone()), (Mode::Mut, Slice(b(Str)))], Bool),
        f(vec![(Mode::Let, IntLit), (Mode::Mut, Slice(b(Str)))], Bool),
        Builtin("io.Error"),
        p("T"),
        p("U"),
        Dyn(0, vec![]),
        Dyn(1, vec![p("T")]),
        Slice(b(p("T"))),
        Result(b(p("T")), b(p("U"))),
        f(vec![(Mode::Let, p("T"))], Optional(b(p("U")))),
    ];
    let env_text = |env: &HashMap<String, Ty>| {
        let mut items: Vec<String> = env.iter().map(|(n, t)| format!("{n}={}", text(t))).collect();
        items.sort();
        items.join(",")
    };
    let mut out = String::new();
    let fixed: HashMap<String, Ty> = [("T".to_string(), int.clone()), ("U".to_string(), Slice(b(Str)))].into();
    for (i, t) in samples.iter().enumerate() {
        let _ = writeln!(out, "one {i} {} | {} | {} {} {} | {}", text(t), text(&concretize(t)), open(t), undecided(t), concrete(t), text(&subst(t, &fixed)));
    }
    for (i, x) in samples.iter().enumerate() {
        for (j, y) in samples.iter().enumerate() {
            let joined = join(x, y).map_or("none".to_string(), |t| text(&t));
            let filled = if same(x, y) { text(&fill_one(x, y)) } else { "-".to_string() };
            let _ = writeln!(out, "pair {i} {j} {joined} | {} {} | {filled}", assignable(x, y), same(x, y));
            let mut env = HashMap::new();
            bind(x, y, &mut env);
            let mut hint: HashMap<String, Ty> = [("T".to_string(), IntLit)].into();
            hinted(x, y, &mut hint);
            if !env.is_empty() || hint.len() != 1 || hint["T"] != IntLit {
                let _ = writeln!(out, "bind {i} {j} {} | {}", env_text(&env), env_text(&hint));
            }
        }
    }
    for a in INT_KINDS {
        for c in INT_KINDS {
            let _ = writeln!(out, "widens {a} {c} {}", widens(a, c));
        }
    }
    out
}

pub fn int_value(s: &str) -> Option<u128> {
    let s = s.replace('_', "");
    let (digits, radix) = match s.get(..2) {
        Some("0x") => (&s[2..], 16),
        Some("0b") => (&s[2..], 2),
        Some("0o") => (&s[2..], 8),
        _ => (&s[..], 10),
    };
    u128::from_str_radix(digits, radix).ok()
}

fn fits(kind: &str, negative: bool, v: u128) -> bool {
    let signed = kind.starts_with('i');
    let bits: u32 = kind[1..].parse().unwrap_or(64);
    if negative {
        return if signed { v <= 1u128 << (bits - 1) } else { v == 0 };
    }
    let max = match (signed, bits) {
        (true, _) => (1u128 << (bits - 1)) - 1,
        (false, 128) => u128::MAX,
        (false, _) => (1u128 << bits) - 1,
    };
    v <= max
}

/// The expression that produces the value of `e`, looking through block tails.
fn tail_of(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Block(Block { tail: Some(t), .. }) => tail_of(t),
        _ => e,
    }
}

/// How a function gives its result by reference: `Some(true)` for
/// `-> ref mut T`, `Some(false)` for `-> ref T`.
pub fn ref_kind(f: &FnDecl) -> Option<bool> {
    match &f.ret {
        Some(Type::Ref { mutable: true, region: None, .. }) => Some(true),
        _ if f.ret_ref => Some(false),
        _ => None,
    }
}

fn root_name(e: &Expr) -> Option<&str> {
    match &e.kind {
        ExprKind::Path(n) => Some(n),
        ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => root_name(base),
        _ => None,
    }
}

pub struct FieldInfo {
    pub name: String,
    pub ty: Ty,
    pub optional: bool,
}

pub struct StructInfo {
    pub name: String,
    pub module: usize,
    /// Has parameters the checker does not handle (regions, values).
    pub generic: bool,
    pub tparams: Vec<String>,
    pub fields: Vec<FieldInfo>,
}

pub struct VariantInfo {
    pub name: String,
    pub fields: Vec<(Option<String>, Ty)>,
}

pub struct EnumInfo {
    pub name: String,
    pub module: usize,
    pub generic: bool,
    pub tparams: Vec<String>,
    pub variants: Vec<VariantInfo>,
}

pub struct FnInfo {
    pub sig: Sig,
    /// Has generic parameters the checker does not handle (regions, values).
    pub generic: bool,
    /// Type parameters and the traits that bound each one.
    pub tparams: TParams,
}

#[derive(Clone)]
pub struct Method {
    pub sig: Sig,
    /// `None` for associated functions without `self`.
    pub self_mode: Option<Mode>,
    /// Type parameters of the `impl` followed by those of the method.
    pub tparams: TParams,
    /// Index of the `impl` in `file.decls` and of the method inside it.
    pub at: (usize, usize),
    /// The type the `impl` is for, in terms of its type parameters.
    pub self_ty: Ty,
    /// For a method the `impl` takes from its trait: where the trait
    /// declares it. Such a method is checked once per type, as an instance.
    pub default: Option<(usize, usize)>,
    /// For such a method: the type parameters of the trait and what the
    /// `impl` gives them, in terms of `tparams`.
    pub trait_env: Vec<(String, Ty)>,
    /// For such a method: the trait the `impl` is of, with its arguments.
    pub of_trait: Option<Bound>,
}

impl StructInfo {
    pub fn field_tys(&self, args: &[Ty]) -> Vec<Ty> {
        let env = targ_env(&self.tparams, args);
        self.fields.iter().map(|f| subst(&f.ty, &env)).collect()
    }
}

impl EnumInfo {
    pub fn field_tys(&self, variant: usize, args: &[Ty]) -> Vec<Ty> {
        let env = targ_env(&self.tparams, args);
        self.variants[variant].fields.iter().map(|(_, t)| subst(t, &env)).collect()
    }
}

#[derive(Clone, Copy)]
enum Global {
    Struct(usize),
    Enum(usize),
    Fn(usize),
    Const(usize),
    Alias,
    Trait(usize),
    /// A module the compiler provides, or one it cannot see into.
    Import,
    /// A module of the program, by number.
    Module(usize),
}

enum Alias {
    Todo(usize),
    Busy,
    Done(Ty),
}

#[derive(Clone)]
struct Var {
    ty: Ty,
    mutable: bool,
    /// Distinguishes variables with the same name.
    id: usize,
    /// Borrowed parameters cannot be moved out of with `own`.
    movable: bool,
    /// An immutable variable declared without a value: assigned once, later.
    deferred: bool,
}

/// A variable, or a field or element inside one, that an expression denotes.
#[derive(Clone, Debug)]
struct Access {
    root: usize,
    name: String,
    segs: Vec<Seg>,
}

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Field(String),
    /// The index when it is a literal.
    Index(Option<String>),
}

/// Whether two accesses can touch the same memory. Different fields and
/// different literal indices do not overlap; anything else may.
fn overlap(a: &Access, b: &Access) -> bool {
    a.root == b.root
        && a.segs.iter().zip(&b.segs).all(|(x, y)| match (x, y) {
            (Seg::Field(p), Seg::Field(q)) => p == q,
            (Seg::Index(Some(p)), Seg::Index(Some(q))) => p == q,
            _ => true,
        })
}

fn scalar(t: &Ty) -> bool {
    matches!(t, Ty::Int(_) | Ty::IntLit | Ty::Float(_) | Ty::FloatLit | Ty::Bool | Ty::Char | Ty::Unit)
}

struct Loop {
    /// Join of the `break` values seen so far.
    ty: Option<Ty>,
    /// Only `loop` can produce a value with `break`.
    valued: bool,
}

struct FnCtx {
    ret: Ty,
    err: Option<Ty>,
    loops: Vec<Loop>,
    /// The function returns `ref`: its result is the last expression.
    by_ref: bool,
}

struct Checker<'a> {
    /// Stop after the declarations, without looking into any expression:
    /// what `mou decls` prints.
    decls_only: bool,
    types: HashMap<(usize, usize), Ty>,
    wants: HashMap<(usize, usize), Ty>,
    res: HashMap<(usize, usize), Res>,
    /// Instance being checked: 0 for ordinary code, `DEFINITION` for the
    /// body of a generic function checked against its bounds.
    inst: usize,
    /// Type parameters in scope: a `Ty::Param` in a definition, the actual
    /// type in an instance.
    tenv: HashMap<String, Ty>,
    /// Traits each type parameter in scope is bound by.
    bounds: HashMap<String, Vec<Bound>>,
    traits: Vec<TraitInfo>,
    /// (trait, type name) pairs for which an `impl` exists.
    /// (trait, type) to the bounds the `impl` puts on each type argument.
    trait_impls: HashMap<(usize, String), Vec<ImplInfo>>,
    /// For a type and a method name that impls of generic traits give it:
    /// each such method, as named in `methods`, with the trait it is of.
    overloads: HashMap<(String, String), Vec<(String, Bound)>>,
    impl_suffix: HashMap<usize, String>,
    /// While an instance is checked: what its bounds say each type implements.
    active: Vec<(Ty, Bound)>,
    /// The types of the arguments of the call being resolved, when choosing
    /// what it calls needed them.
    pre_args: Option<(Vec<Ty>, Vec<(usize, usize)>)>,
    /// Type the expression about to be checked is expected to have.
    hint: Option<Ty>,
    alias_depth: usize,
    /// The same, for the expression being checked right now.
    pending: Option<Ty>,
    instances: Vec<Instance>,
    dyns: HashMap<(usize, usize), Vec<Res>>,
    externs: Vec<Extern>,
    /// C function of each (module, name of the import, name).
    extern_ids: HashMap<(usize, String, String), usize>,
    /// The headers of C whose functions are taken in once the structs
    /// they may name have their fields.
    foreign_todo: Vec<(usize, String, String, &'a [Decl])>,
    metas: HashMap<(usize, usize), Meta>,
    /// The structs marked `@c`, and the type of C each one is.
    cstructs: HashMap<usize, String>,
    /// How many `unsafe` blocks are open.
    unsafe_depth: usize,
    /// Calls of functions that return `ref`: whether it is `ref mut`, and
    /// whether the result is part of the receiver (else, of the first argument).
    ref_calls: HashMap<(usize, usize), (bool, bool)>,
    ref_writes: std::collections::HashSet<(usize, usize)>,
    /// The variable each use of a local name refers to.
    var_of: HashMap<(usize, usize), usize>,
    instance_ids: HashMap<(Target, String), usize>,
    decl_of_fn: Vec<usize>,
    /// For each closure being checked, the number of scopes outside it.
    closures: Vec<usize>,
    next_var: usize,
    /// Every `mut` access and assignment checked so far.
    mut_log: Vec<(Access, Span)>,
    /// What the enclosing `for` loops iterate over.
    frozen: Vec<Access>,
    /// The receiver of the method call whose arguments are checked next.
    receiver: Option<(Access, bool)>,
    /// Where in `mut_log` each argument of the next call was evaluated.
    arg_logs: Option<Vec<(usize, usize)>>,
    /// Name an import is visible as, to its path.
    imports: HashMap<(usize, String), String>,
    file: &'a File,
    structs: Vec<StructInfo>,
    enums: Vec<EnumInfo>,
    fns: Vec<FnInfo>,
    consts: Vec<Option<Ty>>,
    /// What each constant is declared as, to compute the ones a
    /// `comptime` parameter is given.
    const_exprs: Vec<&'a Expr>,
    aliases: HashMap<(usize, String), Alias>,
    /// Module of the declaration being checked.
    module: usize,
    /// Module of every declaration of `file.decls`.
    module_of: Vec<usize>,
    /// Dotted name of every module; empty for the program's own file.
    mods: Vec<String>,
    /// Names declared `pub`, by module.
    public: std::collections::HashSet<(usize, String)>,
    globals: HashMap<(usize, String), Global>,
    methods: HashMap<(String, String), Method>,
    fn_of_decl: HashMap<usize, usize>,
    impl_ty: HashMap<usize, Ty>,
    errors: Vec<Error>,
    report: Report,
    scopes: Vec<HashMap<String, Var>>,
    ctx: Vec<FnCtx>,
    self_ty: Option<Ty>,
    cur: Span,
}

/// A function of a C header.
#[derive(Debug, Clone)]
pub struct Extern {
    pub name: String,
    pub header: String,
    pub sig: Sig,
}

/// A function a built-in module provides: the name later stages know it
/// by, and its signature.
fn native(module: &str, name: &str) -> Option<(&'static str, Sig)> {
    let param = |name: &str, ty: Ty| ParamSig { name: name.into(), mode: Mode::Let, ty, optional: false };
    let fails = |ty: Ty| Ty::Result(Box::new(ty), Box::new(Ty::Builtin("io.Error")));
    let list = |ty: Ty| Ty::Slice(Box::new(ty));
    let int = Ty::Int("i64");
    let (native, params, ret) = match (module, name) {
        ("std.io", "print") => ("io.print", vec![param("text", Ty::Str)], fails(Ty::Unit)),
        ("std.io", "eprint") => ("io.eprint", vec![param("text", Ty::Str)], fails(Ty::Unit)),
        ("std.io", "read_line") => ("io.read_line", vec![], fails(Ty::Optional(Box::new(Ty::Str)))),
        ("std.io", "read_file") => ("io.read_file", vec![param("path", Ty::Str)], fails(Ty::Str)),
        ("std.io", "write_file") => ("io.write_file", vec![param("path", Ty::Str), param("text", Ty::Str)], fails(Ty::Unit)),
        ("std.io", "args") => ("io.args", vec![], list(Ty::Str)),
        ("std.task", "cancelled") => ("task.cancelled", vec![], Ty::Bool),
        ("std.raw", "concat") => ("raw.concat", vec![param("parts", list(Ty::Str))], Ty::Str),
        ("std.raw", "from_chars") => ("raw.from_chars", vec![param("chars", list(Ty::Char))], Ty::Str),
        ("std.raw", "hash") => ("raw.hash", vec![param("text", Ty::Str)], int),
        ("std.raw", "mix") => ("raw.mix", vec![param("n", int.clone())], int),
        ("std.raw", "parse_float") => ("raw.parse_float", vec![param("text", Ty::Str)], Ty::Optional(Box::new(Ty::Float("f64")))),
        _ => return None,
    };
    Some((native, Sig { params, ret }))
}

impl<'a> Checker<'a> {
    /// Takes in the functions a module declares of a C header.
    fn foreign(&mut self, module: usize, alias: &str, header: &str, decls: &'a [Decl]) {
        let plain = |t: &Ty| matches!(t, Ty::Float(_) | Ty::Bool) || matches!(t, Ty::Int(k) if !k.ends_with("128"));
        let only = "a C function can only take and return numbers of up to 64 bits, `bool`, `str`, `ptr`, structs marked `@c` and, to take, arrays of numbers and functions of numbers";
        // An address goes as `ptr`, or as `ptr?` when it may be null.
        let address = |t: &Ty| *t == Ty::Builtin("ptr") || *t == Ty::Optional(Box::new(Ty::Builtin("ptr")));
        self.module = module;
        for d in decls {
            let DeclKind::Fn(f) = &d.kind else {
                self.err(d.span, "only functions can be declared for a C header");
                continue;
            };
            if f.body.is_some() || !f.generics.is_empty() || f.err.is_some() || !f.where_.is_empty() || f.ret_ref {
                self.err(d.span, format!("`{}` is a C function: it is declared with its parameters and its result, and nothing else", f.name));
                continue;
            }
            let mut params = Vec::new();
            for p in &f.params {
                let ty = match &p.ty {
                    Some(t) => self.resolve(t, d.span),
                    None => Ty::Unknown,
                };
                // A struct of C goes by value, or as its address when it is `mut`.
                // So does an array of numbers: C is given where they are.
                let record = matches!(&ty, Ty::Struct(i, targs) if targs.is_empty() && self.cstructs.contains_key(i))
                    || matches!(&ty, Ty::Slice(t) if matches!(&**t, Ty::Float(_)) || matches!(&**t, Ty::Int(k) if !k.ends_with("128")));
                // A function goes as its address, for C to call: one of
                // numbers, `bool` and `ptr?`.
                let nullable = |t: &Ty| *t == Ty::Optional(Box::new(Ty::Builtin("ptr")));
                let callback = match &ty {
                    Ty::Fn(sig) => {
                        sig.params.iter().all(|q| q.mode == Mode::Let && (plain(&q.ty) || nullable(&q.ty)))
                            && (plain(&sig.ret) || sig.ret == Ty::Unit || address(&sig.ret))
                    }
                    _ => false,
                };
                let fits = if record { matches!(p.mode, Mode::Let | Mode::Mut) } else { p.mode == Mode::Let && (plain(&ty) || ty == Ty::Str || address(&ty) || callback) };
                if !fits {
                    self.err(d.span, format!("the parameter `{}` of `{}`: {only}", p.name, f.name));
                }
                let mode = if record && p.mode == Mode::Mut { Mode::Mut } else { Mode::Let };
                params.push(ParamSig { name: p.name.clone(), mode, ty, optional: false });
            }
            let ret = match &f.ret {
                Some(t) => self.resolve(t, d.span),
                None => Ty::Unit,
            };
            // A C string comes back as `str`, or as `str?` when it may be null.
            let text = ret == Ty::Str || ret == Ty::Optional(Box::new(Ty::Str));
            let record = matches!(&ret, Ty::Struct(i, targs) if targs.is_empty() && self.cstructs.contains_key(i));
            if !plain(&ret) && ret != Ty::Unit && !text && !address(&ret) && !record {
                self.err(d.span, format!("the result of `{}`: {only}", f.name));
            }
            self.extern_ids.insert((module, alias.to_string(), f.name.clone()), self.externs.len());
            self.externs.push(Extern { name: f.name.clone(), header: header.to_string(), sig: Sig { params, ret } });
        }
        self.module = 0;
    }

    /// Whether the module being checked is a `system` one.
    fn in_system(&self) -> bool {
        let profile = match self.file.modules.get(self.module) {
            Some(m) => m.profile.as_deref(),
            None => self.file.profile.as_deref(),
        };
        profile == Some("system")
    }

    fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.errors.push(Error { msg: msg.into(), span });
    }

    fn is_io(&self, name: &str) -> bool {
        self.imports.get(&(self.module, name.to_string())).is_some_and(|p| p == "std.io")
    }

    /// Marks an expression as not checked and gives it the unknown type.
    fn skip(&mut self) -> Ty {
        self.report.skipped_exprs += 1;
        Ty::Unknown
    }

    fn show(&self, t: &Ty) -> String {
        use Ty::*;
        match t {
            Int(k) | Float(k) => k.to_string(),
            IntLit => "integer literal".into(),
            FloatLit => "float literal".into(),
            Bool => "bool".into(),
            Char => "char".into(),
            Str => "str".into(),
            Unit => "()".into(),
            Never => "never".into(),
            Unknown => "_".into(),
            Builtin(name) => name.to_string(),
            Param(name) => name.clone(),
            Dyn(t, targs) => format!("dyn {}", self.show_bound(&(*t, targs.clone()))),
            Struct(..) | Enum(..) => self.with_args(&self.key(t), match t {
                Struct(_, args) | Enum(_, args) => args,
                _ => &[],
            }),
            Tuple(ts) => format!("({})", ts.iter().map(|t| self.show(t)).collect::<Vec<_>>().join(", ")),
            Slice(t) => format!("[{}]", self.show(t)),
            Array(t) => format!("[{}; _]", self.show(t)),
            Value(v) => v.to_string(),
            Range(t) => format!("range of {}", self.show(t)),
            Task(t) => format!("Task[{}]", self.show(t)),
            Chan(t) => format!("Chan[{}]", self.show(t)),
            Mutex(t) => format!("Mutex[{}]", self.show(t)),
            Optional(t) if matches!(**t, Never) => "None".into(),
            Optional(t) => format!("{}?", self.show(t)),
            Result(t, e) => format!("{} ! {}", self.show(t), self.show(e)),
            Fn(sig) => {
                let ps: Vec<String> = sig
                    .params
                    .iter()
                    .map(|p| {
                        let mode = match p.mode {
                            Mode::Let => "",
                            Mode::Mut => "mut ",
                            Mode::Own => "own ",
                        };
                        format!("{mode}{}", self.show(&p.ty))
                    })
                    .collect();
                format!("fn({}) -> {}", ps.join(", "), self.show(&sig.ret))
            }
        }
    }

    fn with_args(&self, name: &str, args: &[Ty]) -> String {
        if args.is_empty() {
            return name.to_string();
        }
        format!("{name}[{}]", args.iter().map(|t| self.show(t)).collect::<Vec<_>>().join(", "))
    }

    /// The name methods and trait implementations of a type are filed under:
    /// a generic type has one set for all of its type arguments.
    fn key(&self, t: &Ty) -> String {
        match t {
            Ty::Struct(i, _) => self.qualified(self.structs[*i].module, &self.structs[*i].name),
            Ty::Enum(i, _) => self.qualified(self.enums[*i].module, &self.enums[*i].name),
            _ => self.show(t),
        }
    }

    /// A name as other modules see it: `std.list.List`.
    fn qualified(&self, module: usize, name: &str) -> String {
        match self.mods.get(module) {
            Some(m) if !m.is_empty() => format!("{m}.{name}"),
            _ => name.to_string(),
        }
    }

    /// What a name means at the top level of the module being checked.
    fn global(&self, name: &str) -> Option<Global> {
        self.global_in(self.module, name)
    }

    fn global_in(&self, module: usize, name: &str) -> Option<Global> {
        self.globals.get(&(module, name.to_string())).copied()
    }

    fn at_decl(&mut self, i: usize) {
        self.module = self.module_of[i];
    }

    /// Whether the module being checked may use a name of another module.
    /// Reports it when it may not.
    fn visible(&mut self, module: usize, name: &str, span: Span) -> bool {
        if module == self.module || self.public.contains(&(module, name.to_string())) {
            return true;
        }
        let shown = self.mods[module].clone();
        if self.global_in(module, name).is_some() {
            self.err(span, format!("`{name}` is private to the module `{shown}`: it is not declared `pub`"));
        } else {
            self.err(span, format!("the module `{shown}` has no `{name}`"));
        }
        false
    }

    /// The top-level name an expression refers to, with its module:
    /// `name`, or `module.name` for a name of an imported module.
    fn gref<'e>(&mut self, e: &'e Expr) -> Option<(usize, &'e str)> {
        match &e.kind {
            ExprKind::Path(n) if self.lookup(n).is_none() => Some((self.module, n)),
            ExprKind::Field { base, name } => {
                let ExprKind::Path(b) = &base.kind else { return None };
                if self.lookup(b).is_some() {
                    return None;
                }
                let Some(Global::Module(m)) = self.global(b) else { return None };
                self.visible(m, name, e.span).then_some((m, name.as_str()))
            }
            _ => None,
        }
    }

    /// Whether an expression is `module.name` for a module of the program.
    fn in_module(&self, e: &Expr) -> bool {
        let ExprKind::Field { base, .. } = &e.kind else { return false };
        let ExprKind::Path(b) = &base.kind else { return false };
        self.lookup(b).is_none() && matches!(self.global(b), Some(Global::Module(_)))
    }

    /// A type of another module, named from the module being checked.
    fn named_in(&mut self, module: usize, name: &str, args: Vec<Ty>, span: Span) -> Ty {
        if module == self.module {
            return self.named(name, args, span);
        }
        if !self.visible(module, name, span) {
            return Ty::Unknown;
        }
        // The name is looked up in its own module, where the type
        // parameters of the code being checked mean nothing.
        let saved = (self.module, std::mem::take(&mut self.tenv));
        self.module = module;
        let t = self.named(name, args, span);
        (self.module, self.tenv) = saved;
        t
    }

    /// The trait a bound or an `impl` names, with its type arguments.
    fn trait_of(&mut self, t: &Type, span: Span) -> Option<Bound> {
        let Type::Path { path, args } = t else { return None };
        let found = match path.as_slice() {
            [name] => self.global(name),
            [m, name] => match self.global(m) {
                Some(Global::Module(m)) if self.visible(m, name, span) => self.global_in(m, name),
                _ => None,
            },
            _ => None,
        };
        let Some(Global::Trait(ti)) = found else { return None };
        let mut targs = Vec::new();
        for a in args {
            match a {
                GenArg::Type(t) => targs.push(self.resolve(t, span)),
                GenArg::Expr(_) => return None,
            }
        }
        let want = self.traits[ti].tparams.len();
        if targs.len() != want {
            let name = self.traits[ti].name.clone();
            self.err(span, format!("trait `{name}` takes {want} type arguments, found {}", targs.len()));
            targs.resize(want, Ty::Unknown);
        }
        Some((ti, targs))
    }

    /// A bound as it is written: `Into[str]`.
    fn show_bound(&self, b: &Bound) -> String {
        self.with_args(&self.traits[b.0].name, &b.1)
    }

    /// Whether the module being checked may call a method. Reports it when not.
    fn method_visible(&mut self, m: &Method, name: &str, span: Span) -> bool {
        let file = self.file;
        let module = self.module_of[m.at.0];
        let DeclKind::Impl(im) = &file.decls[m.at.0].kind else { return true };
        if module == self.module || im.trait_.is_some() || im.decls[m.at.1].public {
            return true;
        }
        let shown = self.mods[module].clone();
        self.err(span, format!("the method `{name}` is private to the module `{shown}`: it is not declared `pub`"));
        false
    }

    /// Names of the type parameters, or `None` when there are region or
    /// value parameters, which are not checked yet.
    fn param_names(generics: &[GenParam]) -> Option<Vec<String>> {
        generics.iter().map(|g| match g {
            GenParam::Type { name, .. } => Some(name.clone()),
            _ => None,
        }).collect()
    }

    /// Starts checking the fields of a generic struct or enum.
    fn enter_type(&mut self, generics: &[GenParam], span: Span) {
        if generics.is_empty() {
            return;
        }
        self.inst = DEFINITION;
        for g in generics {
            if let GenParam::Type { name, bounds } = g {
                if !bounds.is_empty() {
                    self.err(span, format!("bounds on the type parameter `{name}` of a type are not supported: put them on the `impl`"));
                }
                self.tenv.insert(name.clone(), Ty::Param(name.clone()));
            }
        }
    }

    // ---- declarations ----

    fn run(&mut self) {
        let file = self.file;
        if file.modules.is_empty() {
            // A file checked on its own: one module, and what it imports
            // is either built in or unknown.
            self.module_of = vec![0; file.decls.len()];
            self.mods = vec![String::new()];
            for imp in &file.imports {
                let name = match (&imp.alias, &imp.origin) {
                    (Some(a), _) => a.clone(),
                    (None, None) => imp.path.rsplit('.').next().unwrap_or(&imp.path).to_string(),
                    (None, Some(_)) => continue,
                };
                self.imports.insert((0, name.clone()), imp.path.clone());
                self.globals.insert((0, name.clone()), Global::Import);
                if imp.origin.as_deref() == Some("c") {
                    self.foreign_todo.push((0, name.clone(), imp.path.clone(), &imp.decls));
                }
            }
        } else {
            self.module_of = vec![0; file.decls.len()];
            for (m, module) in file.modules.iter().enumerate() {
                self.mods.push(module.name.clone());
                for i in module.decls.clone() {
                    self.module_of[i] = m;
                }
                for imp in &module.imports {
                    let global = match imp.module {
                        Some(target) => Global::Module(target),
                        None => {
                            self.imports.insert((m, imp.alias.clone()), imp.path.clone());
                            Global::Import
                        }
                    };
                    self.globals.insert((m, imp.alias.clone()), global);
                    if imp.origin.as_deref() == Some("c") {
                        self.foreign_todo.push((m, imp.alias.clone(), imp.path.clone(), &imp.decls));
                    }
                }
            }
        }
        // Pass 1: names.
        for (i, d) in file.decls.iter().enumerate() {
            self.at_decl(i);
            let (name, global) = match &d.kind {
                DeclKind::Struct(s) => {
                    let tparams = Self::param_names(&s.generics);
                    let generic = tparams.is_none();
                    self.structs.push(StructInfo { name: s.name.clone(), module: self.module, generic, tparams: tparams.unwrap_or_default(), fields: Vec::new() });
                    (&s.name, Global::Struct(self.structs.len() - 1))
                }
                DeclKind::Enum(e) => {
                    let tparams = Self::param_names(&e.generics);
                    let generic = tparams.is_none();
                    self.enums.push(EnumInfo { name: e.name.clone(), module: self.module, generic, tparams: tparams.unwrap_or_default(), variants: Vec::new() });
                    (&e.name, Global::Enum(self.enums.len() - 1))
                }
                DeclKind::Fn(f) => (&f.name, Global::Fn(usize::MAX)),
                DeclKind::Const { name, value, .. } => {
                    self.consts.push(None);
                    self.const_exprs.push(value);
                    (name, Global::Const(self.consts.len() - 1))
                }
                DeclKind::TypeAlias { name, .. } => (name, Global::Alias),
                DeclKind::Trait(t) => {
                    self.traits.push(TraitInfo {
                        name: t.name.clone(),
                        methods: Vec::new(),
                        defaults: Vec::new(),
                        supers: Vec::new(),
                        tparams: Self::param_names(&t.generics).unwrap_or_default(),
                        refs: Vec::new(),
                        own: Vec::new(),
                    });
                    (&t.name, Global::Trait(self.traits.len() - 1))
                }
                DeclKind::Impl(_) => continue,
            };
            if self.global(name).is_some() || prim(name).is_some() {
                self.err(d.span, format!("`{name}` is already defined"));
                continue;
            }
            let global = match global {
                Global::Fn(_) => {
                    self.fns.push(FnInfo { sig: Sig { params: Vec::new(), ret: Ty::Unit }, generic: false, tparams: Vec::new() });
                    self.decl_of_fn.push(i);
                    self.fn_of_decl.insert(i, self.fns.len() - 1);
                    Global::Fn(self.fns.len() - 1)
                }
                Global::Alias => {
                    self.aliases.insert((self.module, name.clone()), Alias::Todo(i));
                    global
                }
                other => other,
            };
            if d.public {
                self.public.insert((self.module, name.clone()));
            }
            self.globals.insert((self.module, name.clone()), global);
        }
        // Pass 2: fields of structs and enums.
        for (di, d) in file.decls.iter().enumerate() {
            self.at_decl(di);
            self.cur = d.span;
            match &d.kind {
                DeclKind::Struct(s) => {
                    let Some(Global::Struct(i)) = self.global(&s.name) else { continue };
                    if self.structs[i].generic {
                        self.report.skipped_decls.push(format!("struct {}", s.name));
                        continue;
                    }
                    self.enter_type(&s.generics, d.span);
                    let mut fields = Vec::new();
                    for f in &s.fields {
                        if fields.iter().any(|x: &FieldInfo| x.name == f.name) {
                            self.err(d.span, format!("field `{}` is defined twice in `{}`", f.name, s.name));
                        }
                        let ty = self.resolve(&f.ty, d.span);
                        if let (Some(default), false) = (&f.default, self.decls_only) {
                            self.expect(default, &ty, "field default");
                        }
                        fields.push(FieldInfo { name: f.name.clone(), ty, optional: f.default.is_some() });
                    }
                    if let Some(a) = d.attrs.iter().find(|a| a.name == "c") {
                        // The struct is one of a C header: its fields are
                        // the members of that one, by name.
                        let written = match a.args.as_slice() {
                            [Expr { kind: ExprKind::Str(parts), .. }] => match parts.as_slice() {
                                [StrPart::Lit(s)] => Some(s.clone()),
                                _ => None,
                            },
                            _ => None,
                        };
                        let member = |t: &Ty| {
                            matches!(t, Ty::Float(_) | Ty::Bool | Ty::Builtin("ptr"))
                                || matches!(t, Ty::Int(k) if !k.ends_with("128"))
                                || *t == Ty::Optional(Box::new(Ty::Builtin("ptr")))
                        };
                        let named = written.filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ' '));
                        match named {
                            None => self.err(d.span, "`@c` takes the name of the type in C, as in `@c(\"struct timeval\")`"),
                            Some(_) if !self.structs[i].tparams.is_empty() => self.err(d.span, "a struct of C cannot have type parameters"),
                            Some(_) if fields.is_empty() || !fields.iter().all(|f| member(&f.ty)) => {
                                self.err(d.span, "a struct of C can only have fields that are numbers of up to 64 bits, `bool` or `ptr`");
                            }
                            Some(name) => {
                                self.cstructs.insert(i, name);
                            }
                        }
                    }
                    self.structs[i].fields = fields;
                    self.leave_generic();
                }
                DeclKind::Enum(e) => {
                    let Some(Global::Enum(i)) = self.global(&e.name) else { continue };
                    if self.enums[i].generic {
                        self.report.skipped_decls.push(format!("enum {}", e.name));
                        continue;
                    }
                    self.enter_type(&e.generics, d.span);
                    let mut variants = Vec::new();
                    for v in &e.variants {
                        if variants.iter().any(|x: &VariantInfo| x.name == v.name) {
                            self.err(d.span, format!("variant `{}` is defined twice in `{}`", v.name, e.name));
                        }
                        let fields = v.fields.iter().map(|(n, t)| (n.clone(), self.resolve(t, d.span))).collect();
                        if let (Some(value), false) = (&v.value, self.decls_only) {
                            let t = self.expr(value);
                            if !is_int(&t) {
                                self.err(value.span, "an enum value must be an integer");
                            }
                        }
                        variants.push(VariantInfo { name: v.name.clone(), fields });
                    }
                    self.enums[i].variants = variants;
                    self.leave_generic();
                }
                DeclKind::TypeAlias { name, generics, .. } => {
                    if generics.is_empty() {
                        self.resolve_alias(name, d.span);
                    } else if Self::param_names(generics).is_none() {
                        self.report.skipped_decls.push(format!("type {name}"));
                    }
                }
                _ => {}
            }
        }
        for (m, alias, header, decls) in std::mem::take(&mut self.foreign_todo) {
            self.foreign(m, &alias, &header, decls);
        }
        // Pass 3: signatures of traits, functions and methods.
        for (di, d) in file.decls.iter().enumerate() {
            self.at_decl(di);
            let DeclKind::Trait(t) = &d.kind else { continue };
            let Some(Global::Trait(ti)) = self.global(&t.name) else { continue };
            let Some(tparams) = self.type_params(&t.generics, d.span) else {
                self.report.skipped_decls.push(format!("trait {} (region and value parameters)", t.name));
                continue;
            };
            self.enter_definition(&tparams);
            for sup in &t.supers {
                match self.trait_of(sup, d.span) {
                    Some(st) if st.0 == ti => self.err(d.span, format!("trait `{}` cannot require itself", t.name)),
                    Some(st) => self.traits[ti].supers.push(st),
                    None => self.err(d.span, format!("what `{}` requires must be a trait", t.name)),
                }
            }
            self.self_ty = Some(Ty::Param("Self".into()));
            for (mi, m) in t.methods.iter().enumerate() {
                let DeclKind::Fn(f) = &m.kind else { continue };
                let Some(own) = self.type_params(&f.generics, m.span) else {
                    self.report.skipped_decls.push(format!("fn {}.{} (region and value parameters)", t.name, f.name));
                    continue;
                };
                if let Some((n, _)) = own.iter().find(|(n, _)| self.tenv.contains_key(n)) {
                    self.err(m.span, format!("type parameter `{n}` is already a parameter of the trait"));
                }
                for (n, bs) in &own {
                    self.tenv.insert(n.clone(), Ty::Param(n.clone()));
                    self.bounds.insert(n.clone(), bs.clone());
                }
                let (sig, self_mode) = self.signature(f, m.span);
                for (n, _) in &own {
                    self.tenv.remove(n);
                    self.bounds.remove(n);
                }
                if self.traits[ti].methods.iter().any(|m| m.0 == f.name) {
                    self.err(m.span, format!("method `{}` is already declared by `{}`", f.name, t.name));
                    continue;
                }
                if !own.is_empty() {
                    self.traits[ti].own.push((f.name.clone(), own));
                }
                if let Some(mutable) = ref_kind(f) {
                    self.traits[ti].refs.push((f.name.clone(), mutable));
                }
                self.traits[ti].methods.push((f.name.clone(), sig, self_mode));
                if f.body.is_some() {
                    self.traits[ti].defaults.push((f.name.clone(), di, mi));
                }
            }
            self.self_ty = None;
            self.leave_generic();
        }
        let mut required: Vec<(usize, Bound, Ty, Span)> = Vec::new();
        for (i, d) in file.decls.iter().enumerate() {
            self.at_decl(i);
            self.cur = d.span;
            match &d.kind {
                DeclKind::Fn(f) => {
                    let Some(&idx) = self.fn_of_decl.get(&i) else { continue };
                    let known: TParams = f.params.iter().filter(|p| p.comptime).map(|p| (p.name.clone(), Vec::new())).collect();
                    if f.generics.is_empty() && known.is_empty() {
                        self.fns[idx].sig = self.signature(f, d.span).0;
                    } else if let Some(mut tparams) = self.type_params(&f.generics, d.span) {
                        // A `comptime` parameter makes copies of the function
                        // as a type parameter does: one for each value.
                        tparams.extend(known);
                        self.enter_definition(&tparams);
                        self.fns[idx].sig = self.signature(f, d.span).0;
                        for (p, q) in f.params.iter().zip(&self.fns[idx].sig.params.clone()) {
                            if p.comptime && !matches!(q.ty, Ty::Int(_)) && !matches!(p.ty, Some(Type::TypeType)) {
                                self.err(d.span, format!("`{}` is a `comptime` parameter, and for now it has to be an integer or a `type`", p.name));
                            }
                        }
                        self.fns[idx].tparams = tparams;
                        self.leave_generic();
                    } else {
                        self.fns[idx].generic = true;
                        self.report.skipped_decls.push(format!("fn {}", f.name));
                    }
                }
                DeclKind::Impl(im) => {
                    let Some(tparams) = self.type_params(&im.generics, d.span) else {
                        self.report.skipped_decls.push("generic impl".to_string());
                        continue;
                    };
                    self.enter_definition(&tparams);
                    let ty = self.resolve(&im.ty, d.span);
                    if ty == Ty::Unknown {
                        self.leave_generic();
                        continue;
                    }
                    // A generic type has one `impl` for all of its type arguments.
                    let targs = match &ty {
                        Ty::Struct(_, a) | Ty::Enum(_, a) => a.clone(),
                        _ => Vec::new(),
                    };
                    let plain = targs.len() == tparams.len()
                        && targs.iter().zip(&tparams).all(|(a, (n, _))| *a == Ty::Param(n.clone()));
                    if !plain {
                        let shown = self.show(&ty);
                        self.err(d.span, format!("an `impl` for `{shown}` is not supported: a generic `impl` must be for a generic struct or enum and name all of its parameters, as in `impl[T] List[T]`"));
                        self.leave_generic();
                        continue;
                    }
                    let key = self.key(&ty);
                    let bound = im.trait_.as_ref().and_then(|tr| self.trait_of(tr, d.span));
                    let suffix = match &bound {
                        Some(b) => suffix_of(&self.traits, &self.show_bound(b), b),
                        None => String::new(),
                    };
                    self.impl_suffix.insert(i, suffix.clone());
                    for (mi, m) in im.decls.iter().enumerate() {
                        let DeclKind::Fn(f) = &m.kind else { continue };
                        let Some(own) = self.type_params(&f.generics, m.span) else {
                            self.report.skipped_decls.push(format!("fn {key}.{}", f.name));
                            continue;
                        };
                        if let Some((n, _)) = own.iter().find(|(n, _)| tparams.iter().any(|(t, _)| t == n)) {
                            self.err(m.span, format!("type parameter `{n}` is already a parameter of the `impl`"));
                        }
                        self.enter_definition(&own);
                        self.self_ty = Some(ty.clone());
                        let (sig, self_mode) = self.signature(f, m.span);
                        if f.params.iter().any(|p| p.comptime) {
                            self.err(m.span, "a method cannot have `comptime` parameters yet: only a function can");
                        }
                        self.self_ty = None;
                        self.leave_generic();
                        self.enter_definition(&tparams);
                        let k = (key.clone(), format!("{}{suffix}", f.name));
                        if self.methods.contains_key(&k) {
                            self.err(m.span, format!("method `{}` is already defined for `{key}`", f.name));
                        } else if let (Some(b), false) = (&bound, suffix.is_empty()) {
                            self.overloads.entry((key.clone(), f.name.clone())).or_default().push((k.1.clone(), b.clone()));
                        }
                        let all = tparams.iter().cloned().chain(own).collect();
                        self.methods.insert(k, Method { sig, self_mode, tparams: all, at: (i, mi), self_ty: ty.clone(), default: None, trait_env: Vec::new(), of_trait: None });
                    }
                    if im.trait_.is_some() {
                        if let Some(b) = self.conformance(bound, &ty, &key, &tparams, i, d.span) {
                            for sup in with_supers(&self.traits, &b).into_iter().skip(1) {
                                required.push((b.0, sup, ty.clone(), d.span));
                            }
                        }
                    }
                    self.leave_generic();
                    self.impl_ty.insert(i, ty);
                }
                _ => {}
            }
        }
        // A type that implements a trait implements what the trait requires.
        for (t, sup, ty, span) in required {
            let key = self.key(&ty);
            if !self.implementations(&ty, sup.0).iter().any(|(got, _, _)| got.1 == sup.1) {
                let (tname, sname) = (self.traits[t].name.clone(), self.show_bound(&sup));
                self.err(span, format!("`{tname}` requires `{sname}`, which `{key}` does not implement: add `impl {sname} for {key}`"));
            }
        }
        if self.decls_only {
            return;
        }
        // Pass 4: constants, in order.
        for (di, d) in file.decls.iter().enumerate() {
            self.at_decl(di);
            let DeclKind::Const { name, ty, value } = &d.kind else { continue };
            let Some(Global::Const(i)) = self.global(name) else { continue };
            let declared = ty.as_ref().map(|t| self.resolve(t, d.span));
            let actual = self.expr(value);
            let t = match declared {
                Some(t) => {
                    self.coerce(value, &t, &actual, "constant");
                    t
                }
                None => concretize(&actual),
            };
            self.consts[i] = Some(t);
        }
        // Pass 5: bodies.
        for (i, d) in file.decls.iter().enumerate() {
            self.at_decl(i);
            match &d.kind {
                DeclKind::Fn(f) => {
                    let Some(&idx) = self.fn_of_decl.get(&i) else { continue };
                    if !self.fns[idx].generic {
                        let sig = self.fns[idx].sig.clone();
                        let tparams = self.fns[idx].tparams.clone();
                        if !tparams.is_empty() {
                            self.enter_definition(&tparams);
                        }
                        self.body(f, &sig, None, d.span);
                        self.leave_generic();
                    }
                }
                DeclKind::Impl(im) => {
                    let Some(ty) = self.impl_ty.get(&i).cloned() else { continue };
                    let key = self.key(&ty);
                    for m in &im.decls {
                        let DeclKind::Fn(f) = &m.kind else { continue };
                        let name = format!("{}{}", f.name, self.impl_suffix.get(&i).map_or("", |s| s.as_str()));
                        let Some(method) = self.methods.get(&(key.clone(), name)).cloned() else {
                            continue;
                        };
                        self.enter_definition(&method.tparams);
                        self.self_ty = Some(ty.clone());
                        self.body(f, &method.sig, method.self_mode, m.span);
                        self.self_ty = None;
                        self.leave_generic();
                    }
                }
                // The bodies a trait gives: checked here for any `Self` that
                // implements the trait, and again for each type that uses them.
                DeclKind::Trait(t) => {
                    let Some(Global::Trait(ti)) = self.global(&t.name) else { continue };
                    for (name, _, mi) in self.traits[ti].defaults.clone() {
                        let DeclKind::Fn(f) = &t.methods[mi].kind else { continue };
                        let Some((_, sig, self_mode)) = self.traits[ti].methods.iter().find(|m| m.0 == name).cloned() else {
                            continue;
                        };
                        let own: Vec<Ty> = self.traits[ti].tparams.iter().map(|n| Ty::Param(n.clone())).collect();
                        let mut tparams = self.type_params(&t.generics, d.span).unwrap_or_default();
                        tparams.push(("Self".to_string(), vec![(ti, own)]));
                        if let Some((_, own)) = self.traits[ti].own.iter().find(|o| o.0 == name) {
                            tparams.extend(own.iter().cloned());
                        }
                        self.enter_definition(&tparams);
                        self.self_ty = Some(Ty::Param("Self".into()));
                        self.body(f, &sig, self_mode, t.methods[mi].span);
                        self.self_ty = None;
                        self.leave_generic();
                    }
                }
                _ => {}
            }
        }
        // Pass 6: bodies of generic functions, once per set of type arguments.
        let mut k = 0;
        while k < self.instances.len() {
            if k >= 500 {
                self.err(self.cur, "more than 500 instances of generic functions: the recursion never settles on a type");
                break;
            }
            let (target, targs) = (self.instances[k].f.clone(), self.instances[k].targs.clone());
            match target {
                Target::Fn(fi) => {
                    let d = &file.decls[self.decl_of_fn[fi]];
                    let DeclKind::Fn(f) = &d.kind else { break };
                    self.at_decl(self.decl_of_fn[fi]);
                    self.inst = k + 1;
                    self.tenv = self.fns[fi].tparams.iter().map(|(n, _)| n.clone()).zip(targs).collect();
                    self.activate(&self.fns[fi].tparams.clone());
                    let sig = self.signature(f, d.span).0;
                    self.instances[k].sig = sig.clone();
                    self.body(f, &sig, None, d.span);
                }
                Target::Method(t, n) => {
                    let Some(m) = self.methods.get(&(t, n)).cloned() else { break };
                    let Some(md) = method_decl(file, &m) else { break };
                    let DeclKind::Fn(f) = &md.kind else { break };
                    // A body of the trait sees the names of the trait's module.
                    self.at_decl(m.default.map_or(m.at.0, |d| d.0));
                    self.inst = k + 1;
                    self.tenv = m.tparams.iter().map(|(n, _)| n.clone()).zip(targs).collect();
                    // What the trait calls `T` is what the `impl` gave it.
                    for (n, t) in &m.trait_env {
                        let t = subst(t, &self.tenv);
                        self.tenv.insert(n.clone(), t);
                    }
                    self.self_ty = Some(subst(&m.self_ty, &self.tenv));
                    self.activate(&m.tparams);
                    // A body of the trait runs for the `impl` that took it.
                    if let (Some(b), Some(me)) = (&m.of_trait, self.self_ty.clone()) {
                        let b = (b.0, b.1.iter().map(|a| subst(a, &self.tenv)).collect());
                        self.active.push((me, b));
                    }
                    let (sig, self_mode) = self.signature(f, md.span);
                    self.instances[k].sig = sig.clone();
                    self.body(f, &sig, self_mode, md.span);
                    self.self_ty = None;
                }
            }
            self.leave_generic();
            k += 1;
        }
    }

    /// The type parameters of a declaration, or `None` when it also has
    /// region or value parameters, which are not checked yet.
    fn type_params(&mut self, generics: &[GenParam], span: Span) -> Option<TParams> {
        // A bound may name the other parameters: `[T: Into[U], U]`.
        let mut added = Vec::new();
        for g in generics {
            if let GenParam::Type { name, .. } = g {
                if !self.tenv.contains_key(name) {
                    self.tenv.insert(name.clone(), Ty::Param(name.clone()));
                    added.push(name.clone());
                }
            }
        }
        let out = self.type_params_in(generics, span);
        for name in added {
            self.tenv.remove(&name);
        }
        out
    }

    fn type_params_in(&mut self, generics: &[GenParam], span: Span) -> Option<TParams> {
        let mut out = Vec::new();
        for g in generics {
            let GenParam::Type { name, bounds } = g else { return None };
            let mut traits = Vec::new();
            for b in bounds {
                match self.trait_of(b, span) {
                    Some(t) => traits.push(t),
                    _ => self.err(span, format!("the bound of `{name}` must be a trait")),
                }
            }
            out.push((name.clone(), traits));
        }
        Some(out)
    }

    /// Notes what the bounds of the instance being checked say its types
    /// implement, to choose among several impls of one trait.
    fn activate(&mut self, tparams: &TParams) {
        self.active = tparams
            .iter()
            .filter_map(|(n, bs)| self.tenv.get(n).map(|t| (t.clone(), bs)))
            .flat_map(|(t, bs)| bs.iter().map(move |b| (t.clone(), b.clone())))
            .map(|(t, b)| (t, (b.0, b.1.iter().map(|a| subst(a, &self.tenv)).collect())))
            .collect();
    }

    fn enter_definition(&mut self, tparams: &[(String, Vec<Bound>)]) {
        if tparams.is_empty() {
            return;
        }
        self.inst = DEFINITION;
        for (name, traits) in tparams {
            self.tenv.insert(name.clone(), Ty::Param(name.clone()));
            self.bounds.insert(name.clone(), traits.clone());
        }
    }

    fn leave_generic(&mut self) {
        self.inst = 0;
        self.tenv.clear();
        self.bounds.clear();
        self.active.clear();
    }

    /// Checks that an `impl Trait for Type` provides what the trait asks for.
    fn conformance(&mut self, bound: Option<Bound>, ty: &Ty, key: &str, tparams: &[(String, Vec<Bound>)], at: usize, span: Span) -> Option<Bound> {
        let Some(bound) = bound else {
            self.report.skipped_decls.push(format!("conformance of `{key}` to a trait that is not checked"));
            return None;
        };
        let t = bound.0;
        let shown = self.show_bound(&bound);
        let suffix = suffix_of(&self.traits, &shown, &bound);
        // A type implements a trait once for each choice of type arguments.
        if let Some(others) = self.trait_impls.get(&(t, key.to_string())) {
            if others.iter().any(|o| o.targs == bound.1) {
                self.err(span, format!("`{key}` already implements `{shown}`"));
                return None;
            }
            if !bound.1.iter().all(concrete) || others.iter().any(|o| !o.targs.iter().all(concrete)) {
                let tname = self.traits[t].name.clone();
                self.err(span, format!("`{key}` already implements `{tname}`: to implement it again, every `impl` must give the trait type arguments without type parameters"));
                return None;
            }
        }
        let trait_env: Vec<(String, Ty)> = self.traits[t].tparams.iter().cloned().zip(bound.1.iter().cloned()).collect();
        let mut env: HashMap<String, Ty> = trait_env.iter().cloned().collect();
        env.insert("Self".to_string(), ty.clone());
        for (name, sig, self_mode) in self.traits[t].methods.clone() {
            let own = self.traits[t].own.iter().find(|o| o.0 == name).map(|o| o.1.clone()).unwrap_or_default();
            let mut want = subst_sig(&sig, &env);
            let tname = shown.clone();
            let default = self.traits[t].defaults.iter().find(|d| d.0 == name).map(|d| (d.1, d.2));
            let mangled = format!("{name}{suffix}");
            match self.methods.get(&(key.to_string(), mangled.clone())) {
                None => match default {
                    Some(d) => {
                        let m = Method {
                            sig: want,
                            self_mode,
                            tparams: tparams.iter().cloned().chain(own.iter().cloned()).collect(),
                            at: (at, usize::MAX),
                            self_ty: ty.clone(),
                            default: Some(d),
                            trait_env: trait_env.clone(),
                            of_trait: Some(bound.clone()),
                        };
                        if !suffix.is_empty() {
                            self.overloads.entry((key.to_string(), name.clone())).or_default().push((mangled.clone(), bound.clone()));
                        }
                        self.methods.insert((key.to_string(), mangled), m);
                    }
                    None => self.err(span, format!("`impl {tname} for {key}` is missing the method `{name}`")),
                },
                Some(m) => {
                    // The method may name its own type parameters differently.
                    let mine = &m.tparams[tparams.len().min(m.tparams.len())..];
                    let rename: HashMap<String, Ty> = own.iter().zip(mine).map(|(a, b)| (a.0.clone(), Ty::Param(b.0.clone()))).collect();
                    want = subst_sig(&want, &rename);
                    let bounds = |bs: &[Bound]| -> Vec<Bound> { bs.iter().map(|b| (b.0, b.1.iter().map(|a| subst(&subst(a, &env), &rename)).collect())).collect() };
                    let generics = mine.len() == own.len() && own.iter().zip(mine).all(|(a, b)| bounds(&a.1) == b.1);
                    let same = generics
                        && m.self_mode == self_mode
                        && m.sig.ret == want.ret
                        && m.sig.params.len() == want.params.len()
                        && m.sig.params.iter().zip(&want.params).all(|(a, b)| a.mode == b.mode && a.ty == b.ty);
                    let by_ref = |d: Option<&Decl>| match d.map(|d| &d.kind) {
                        Some(DeclKind::Fn(f)) => ref_kind(f),
                        _ => None,
                    };
                    let wanted = self.traits[t].refs.iter().find(|r| r.0 == name).map(|r| r.1);
                    if !same || by_ref(method_decl(self.file, m)) != wanted {
                        self.err(span, format!("method `{name}` of `{key}` does not have the signature `{tname}` declares"));
                    }
                }
            }
        }
        let info = ImplInfo { targs: bound.1.clone(), self_ty: ty.clone(), tparams: tparams.to_vec() };
        self.trait_impls.entry((t, key.to_string())).or_default().push(info);
        Some(bound)
    }

    /// What a type implements of a trait: for each `impl`, the trait with
    /// the arguments the `impl` gives it.
    fn implementations(&self, t: &Ty, tr: usize) -> Vec<(Bound, HashMap<String, Ty>, ImplInfo)> {
        let Some(infos) = self.trait_impls.get(&(tr, self.key(t))) else { return Vec::new() };
        infos
            .iter()
            .map(|info| {
                let mut env = HashMap::new();
                bind(&info.self_ty, t, &mut env);
                let targs = info.targs.iter().map(|a| subst(a, &env)).collect();
                ((tr, targs), env, info.clone())
            })
            .collect()
    }

    /// Chooses, among the methods of one name that impls of generic traits
    /// give a type, the one a call means. `Ok(None)` when there are none;
    /// `Err` when the call does not say which, which is reported.
    fn pick(&mut self, key: &str, name: &str, hint: &Option<Ty>, args: &[Arg], span: Span) -> Result<Option<String>, ()> {
        let Some(all) = self.overloads.get(&(key.to_string(), name.to_string())).cloned() else { return Ok(None) };
        if let [(only, _)] = all.as_slice() {
            return Ok(Some(only.clone()));
        }
        let sig_of = |c: &Self, m: &str| c.methods.get(&(key.to_string(), m.to_string())).map(|m| m.sig.clone());
        // The type the call must produce decides first.
        let mut left = all.clone();
        if let Some(h) = hint.as_ref().filter(|h| concrete(h)) {
            let fits: Vec<_> = left.iter().filter(|(m, _)| sig_of(self, m).is_some_and(|s| assignable(h, &s.ret))).cloned().collect();
            if !fits.is_empty() {
                left = fits;
            }
        }
        // Inside a generic function: what its bounds ask of the type.
        if left.len() > 1 {
            let wanted: Vec<Bound> = self
                .active
                .iter()
                .filter(|(t, _)| self.key(t) == key)
                .flat_map(|(_, b)| with_supers(&self.traits, b))
                .collect();
            let fits: Vec<_> = left.iter().filter(|(_, b)| wanted.contains(b)).cloned().collect();
            if !fits.is_empty() {
                left = fits;
            }
        }
        // Then the types of the arguments.
        if left.len() > 1 {
            let (tys, logs) = self.arg_types(args);
            let fits: Vec<_> = left
                .iter()
                .filter(|(m, _)| {
                    sig_of(self, m).is_some_and(|s| s.params.len() >= tys.len() && s.params.iter().zip(&tys).all(|(p, t)| assignable(&p.ty, t)))
                })
                .cloned()
                .collect();
            self.pre_args = Some((tys, logs));
            if !fits.is_empty() {
                left = fits;
            }
        }
        if let [(only, _)] = left.as_slice() {
            return Ok(Some(only.clone()));
        }
        self.pre_args = None;
        let traits: Vec<String> = left.iter().map(|(_, b)| format!("`{}`", self.show_bound(b))).collect();
        self.err(span, format!("`{name}` could be the one of {}: give the result a type, as in `let x: Type = ...`", traits.join(" or ")));
        Err(())
    }

    /// The instance of a generic function or method for the given type
    /// arguments, registered so that its body is checked and compiled.
    fn instance(&mut self, target: Target, targs: Vec<Ty>) -> usize {
        let shown = targs.iter().map(|t| self.show(t)).collect::<Vec<_>>().join(", ");
        let next = self.instances.len();
        let k = *self.instance_ids.entry((target.clone(), shown.clone())).or_insert(next);
        if k == next {
            self.instances.push(Instance { f: target, targs, shown, sig: Sig { params: Vec::new(), ret: Ty::Unit } });
        }
        k
    }

    /// The methods a value of type `ty` carries as a `dyn` of trait `t`.
    fn dyn_table(&mut self, ty: &Ty, bound: &Bound) -> Vec<Res> {
        let key = self.key(ty);
        let mut table = Vec::new();
        let supers = with_supers(&self.traits, bound);
        for (name, _, _) in dyn_slots(&self.traits, bound) {
            // The method is the one of the `impl` of the trait that declares it.
            let name = match supers.iter().find(|b| self.traits[b.0].methods.iter().any(|m| m.0 == name)) {
                Some(b) => format!("{name}{}", suffix_of(&self.traits, &self.show_bound(b), b)),
                None => name,
            };
            let Some(m) = self.methods.get(&(key.clone(), name.clone())).cloned() else { continue };
            if m.tparams.is_empty() && m.default.is_none() {
                table.push(Res::Method(key.clone(), name));
                continue;
            }
            let mut env = HashMap::new();
            bind(&m.self_ty, ty, &mut env);
            let targs: Vec<Ty> = m.tparams.iter().map(|(n, _)| env.get(n).cloned().unwrap_or(Ty::Unknown)).collect();
            let k = self.instance(Target::Method(key.clone(), name), targs);
            table.push(Res::Generic(k));
        }
        table
    }

    fn satisfies(&self, t: &Ty, want: &Bound) -> bool {
        let agree = |a: &Vec<Ty>, b: &Vec<Ty>| a.iter().zip(b).all(|(x, y)| x == y || *x == Ty::Unknown || *y == Ty::Unknown);
        match t {
            Ty::Unknown => true,
            Ty::Param(n) => self.bounds.get(n).is_some_and(|bs| {
                bs.iter().any(|b| with_supers(&self.traits, b).iter().any(|s| s.0 == want.0 && agree(&s.1, &want.1)))
            }),
            _ => self.implementations(t, want.0).into_iter().any(|(got, env, info)| {
                agree(&got.1, &want.1)
                    && info.tparams.iter().all(|(n, bs)| {
                        let arg = env.get(n).cloned().unwrap_or(Ty::Unknown);
                        bs.iter().all(|b| self.satisfies(&arg, &(b.0, b.1.iter().map(|t| subst(t, &env)).collect())))
                    })
            }),
        }
    }

    /// The type an expression in type-argument position denotes: `f[int]`.
    fn type_of_expr(&mut self, e: &Expr) -> Ty {
        let path = |name: &str| Type::Path { path: vec![name.to_string()], args: Vec::new() };
        match &e.kind {
            ExprKind::Path(name) => self.resolve(&path(name), e.span),
            ExprKind::Index { base, args } if matches!(base.kind, ExprKind::Path(_)) => {
                let ExprKind::Path(name) = &base.kind else { return Ty::Unknown };
                let targs = args.iter().map(|a| self.type_of_expr(a)).collect();
                self.named(name, targs, e.span)
            }
            ExprKind::Field { .. } if self.in_module(e) => match self.gref(e) {
                Some((m, name)) => self.named_in(m, name, Vec::new(), e.span),
                None => Ty::Unknown,
            },
            ExprKind::Index { base, args } if self.in_module(base) => match self.gref(base) {
                Some((m, name)) => {
                    let targs = args.iter().map(|a| self.type_of_expr(a)).collect();
                    self.named_in(m, name, targs, e.span)
                }
                None => Ty::Unknown,
            },
            ExprKind::Array(items) if items.len() == 1 => Ty::Slice(Box::new(self.type_of_expr(&items[0]))),
            ExprKind::Try(inner) => Ty::Optional(Box::new(self.type_of_expr(inner))),
            ExprKind::Tuple(items) if !items.is_empty() => Ty::Tuple(items.iter().map(|i| self.type_of_expr(i)).collect()),
            _ => {
                self.err(e.span, "expected a type");
                Ty::Unknown
            }
        }
    }

    /// Types of the arguments of a call, with the range of `mut_log` each one filled.
    fn arg_types(&mut self, args: &[Arg]) -> (Vec<Ty>, Vec<(usize, usize)>) {
        let mut logs = Vec::new();
        let mut tys = Vec::new();
        for a in args {
            let start = self.mut_log.len();
            tys.push(self.expr(&a.value));
            logs.push((start, self.mut_log.len()));
        }
        (tys, logs)
    }

    /// The number an expression is, when it is made of numbers, constants
    /// and the `comptime` parameters of the function it is in.
    fn const_int(&self, e: &Expr, depth: usize) -> Option<i128> {
        if depth > 40 {
            return None;
        }
        // Every step has to fit in 64 bits.
        let fits = |v: i128| i64::try_from(v).ok().map(i128::from);
        match &e.kind {
            ExprKind::Int(s) => int_value(s).and_then(|v| i128::try_from(v).ok()).and_then(fits),
            ExprKind::Unary { op: "-", expr } => fits(-self.const_int(expr, depth)?),
            ExprKind::Binary { op, lhs, rhs } => {
                let (a, b) = (self.const_int(lhs, depth)?, self.const_int(rhs, depth)?);
                match *op {
                    "+" => fits(a + b),
                    "-" => fits(a - b),
                    "*" => fits(a * b),
                    "/" if b != 0 => fits(a / b),
                    "%" if b != 0 => fits(a % b),
                    _ => None,
                }
            }
            ExprKind::Path(name) => match self.tenv.get(name) {
                Some(Ty::Value(v)) => Some(*v),
                Some(_) => None,
                None => match self.global(name) {
                    Some(Global::Const(i)) => self.const_int(self.const_exprs[i], depth + 1),
                    _ => None,
                },
            },
            _ => None,
        }
    }

    /// A call to a generic function or method: finds the type arguments,
    /// checks the bounds and records the instance to check and run.
    /// `env` holds what the receiver or an explicit type already decided.
    #[allow(clippy::too_many_arguments)]
    fn generic_call(
        &mut self,
        e: &Expr,
        target: Target,
        mut env: HashMap<String, Ty>,
        explicit: Option<&[Expr]>,
        hint: Option<Ty>,
        receiver: Option<(Access, bool)>,
        args: &[Arg],
        span: Span,
    ) -> Ty {
        let (tparams, sig) = match &target {
            Target::Fn(i) => (self.fns[*i].tparams.clone(), self.fns[*i].sig.clone()),
            Target::Method(t, n) => match self.methods.get(&(t.clone(), n.clone())) {
                Some(m) => (m.tparams.clone(), m.sig.clone()),
                None => return Ty::Unknown,
            },
        };
        // What each `comptime` parameter is given has to be known here:
        // a number, or a type when the parameter is a `type`.
        let known: Vec<(usize, String, bool)> = match &target {
            Target::Fn(i) => match &self.file.decls[self.decl_of_fn[*i]].kind {
                DeclKind::Fn(f) => {
                    f.params.iter().enumerate().filter(|(_, p)| p.comptime).map(|(k, p)| (k, p.name.clone(), matches!(p.ty, Some(Type::TypeType)))).collect()
                }
                _ => Vec::new(),
            },
            Target::Method(..) => Vec::new(),
        };
        let (tys, logs) = match self.pre_args.take() {
            Some(known) => known,
            None if known.iter().any(|k| k.2) => {
                // An argument that is a type is read as one, not as a value.
                let (mut tys, mut logs) = (Vec::new(), Vec::new());
                for (k, a) in args.iter().enumerate() {
                    let start = self.mut_log.len();
                    match known.iter().find(|n| n.0 == k && n.2) {
                        Some((_, name, _)) => {
                            let t = self.type_of_expr(&a.value);
                            env.insert(name.clone(), t);
                            tys.push(Ty::Unknown);
                        }
                        None => tys.push(self.expr(&a.value)),
                    }
                    logs.push((start, self.mut_log.len()));
                }
                (tys, logs)
            }
            None => self.arg_types(args),
        };
        for (k, name, is_type) in &known {
            if *is_type {
                continue;
            }
            let Some(a) = args.get(*k) else { continue };
            match self.const_int(&a.value, 0) {
                Some(v) => {
                    if let Some(Ty::Int(kind)) = sig.params.get(*k).map(|p| &p.ty) {
                        let (lo, hi) = crate::mir::bounds(kind);
                        if (v < lo || v > hi) && !matches!(a.value.kind, ExprKind::Int(_)) {
                            self.err(a.value.span, format!("{v} does not fit in `{kind}`, the type of the `comptime` parameter `{name}`"));
                        }
                    }
                    env.insert(name.clone(), Ty::Value(v));
                }
                // In the body of a function with parameters of its own,
                // the copies of that function say whether it is known.
                None if self.inst == DEFINITION => {
                    env.insert(name.clone(), Ty::Unknown);
                }
                None => {
                    self.err(a.value.span, format!("`{name}` is a `comptime` parameter: what it is given must be a number known when the program is compiled"));
                    env.insert(name.clone(), Ty::Unknown);
                }
            }
        }
        match explicit {
            Some(targs) => {
                if targs.len() != tparams.len() - known.len() {
                    self.err(span, format!("expected {} type arguments, found {}", tparams.len() - known.len(), targs.len()));
                }
                for ((name, _), t) in tparams.iter().zip(targs) {
                    let t = self.type_of_expr(t);
                    env.insert(name.clone(), t);
                }
            }
            None => {
                for (p, t) in sig.params.iter().zip(&tys) {
                    bind(&p.ty, t, &mut env);
                }
                if let Some(h) = &hint {
                    hinted(&sig.ret, h, &mut env);
                }
            }
        }
        // A parameter nothing mentions can follow from a bound: with
        // `[T: Into[U], U]` and `T = int`, `U` is what `int` converts into.
        for (name, bounds) in &tparams {
            let Some(t) = env.get(name).map(concretize) else { continue };
            for b in bounds {
                // Of several impls, the one that agrees with what is known.
                let known: Vec<Ty> = b.1.iter().map(|p| subst(p, &env)).collect();
                let fits: Vec<_> = self
                    .implementations(&t, b.0)
                    .into_iter()
                    .filter(|(got, _, _)| got.1.iter().zip(&known).all(|(g, k)| open(k) || !concrete(k) || g == k))
                    .collect();
                if let [(got, _, _)] = fits.as_slice() {
                    for (pat, actual) in b.1.iter().zip(&got.1) {
                        if open(&subst(pat, &env)) || !concrete(&subst(pat, &env)) {
                            bind(pat, actual, &mut env);
                        }
                    }
                }
            }
        }
        for (name, _) in &tparams {
            let t = match env.get(name) {
                Some(t) => concretize(t),
                None => {
                    self.err(span, format!("cannot infer the type parameter `{name}`: write the type arguments, as in `name[Type](...)`, or give the result a type"));
                    Ty::Unknown
                }
            };
            env.insert(name.clone(), t);
        }
        for (name, bounds) in &tparams {
            let t = env[name].clone();
            for b in bounds {
                let b: Bound = (b.0, b.1.iter().map(|a| subst(a, &env)).collect());
                if !self.satisfies(&t, &b) {
                    let (shown, tname) = (self.show(&t), self.show_bound(&b));
                    self.err(span, format!("type `{shown}` does not implement `{tname}`, which `{name}` requires"));
                }
            }
        }
        self.arg_logs = Some(logs);
        self.receiver = receiver;
        let ret = self.apply_with(&subst_sig(&sig, &env), args, Some(tys), span);
        let targs: Vec<Ty> = tparams.iter().map(|(n, _)| env[n].clone()).collect();
        if self.inst != DEFINITION && targs.iter().all(|t| concrete(t) && !open(t)) {
            let k = self.instance(target, targs);
            self.res.insert((self.inst, id(e)), Res::Generic(k));
        } else if self.inst != DEFINITION && !targs.contains(&Ty::Unknown) {
            self.err(span, "the type arguments of this call are not fully known: write them, as in `name[Type](...)`, or give the values a type");
        }
        ret
    }

    /// The struct or enum an expression names, when it names one: `Tree` or
    /// `Tree[int]`. Without type arguments the type comes back in terms of
    /// its own parameters, and the flag is false.
    fn type_head(&mut self, e: &Expr) -> Option<(Ty, bool)> {
        let ((m, name), targs) = match &e.kind {
            ExprKind::Index { base, args } => (self.gref(base)?, Some(args)),
            _ => (self.gref(e)?, None),
        };
        let (i, is_struct, tparams) = match self.global_in(m, name)? {
            Global::Struct(i) if !self.structs[i].generic => (i, true, self.structs[i].tparams.clone()),
            Global::Enum(i) if !self.enums[i].generic => (i, false, self.enums[i].tparams.clone()),
            _ => return None,
        };
        let (args, explicit) = match targs {
            Some(targs) => {
                let mut tys: Vec<Ty> = targs.iter().map(|t| self.type_of_expr(t)).collect();
                if tys.len() != tparams.len() {
                    self.err(e.span, format!("`{name}` takes {} type arguments, found {}", tparams.len(), tys.len()));
                    tys = vec![Ty::Unknown; tparams.len()];
                }
                (tys, true)
            }
            None => (tparams.iter().map(|n| Ty::Param(n.clone())).collect(), false),
        };
        Some((if is_struct { Ty::Struct(i, args) } else { Ty::Enum(i, args) }, explicit))
    }

    /// Type arguments of a generic type being built: the explicit ones, or
    /// what the expected type and the field values decide. A parameter
    /// nothing decides stays open, like the element type of `[]`.
    fn type_args(
        &mut self,
        tparams: &[String],
        owner: &Ty,
        explicit: Option<Vec<Ty>>,
        hint: Option<Ty>,
        params: &[Ty],
        tys: &[Ty],
    ) -> HashMap<String, Ty> {
        let mut env = HashMap::new();
        match explicit {
            Some(targs) => env = targ_env(tparams, &targs),
            None => {
                for (p, t) in params.iter().zip(tys) {
                    bind(p, t, &mut env);
                }
                if let Some(h) = &hint {
                    hinted(owner, h, &mut env);
                }
            }
        }
        for n in tparams {
            let t = env.get(n).map_or(Ty::Never, concretize);
            env.insert(n.clone(), t);
        }
        env
    }

    /// Builds a struct or an enum variant from its fields.
    #[allow(clippy::too_many_arguments)]
    fn construct(
        &mut self,
        e: &Expr,
        res: Res,
        owner: Ty,
        tparams: &[String],
        fields: Vec<(String, Ty, bool)>,
        explicit: Option<Vec<Ty>>,
        hint: Option<Ty>,
        args: &[Arg],
        span: Span,
    ) -> Ty {
        self.res.insert((self.inst, id(e)), res);
        if tparams.is_empty() {
            let sig = Self::ctor(fields.into_iter(), owner);
            return self.apply(&sig, args, span);
        }
        let (tys, logs) = self.arg_types(args);
        let params: Vec<Ty> = fields.iter().map(|f| f.1.clone()).collect();
        let env = self.type_args(tparams, &owner, explicit, hint, &params, &tys);
        self.arg_logs = Some(logs);
        let sig = subst_sig(&Self::ctor(fields.into_iter(), owner), &env);
        self.apply_with(&sig, args, Some(tys), span)
    }

    fn construct_struct(&mut self, e: &Expr, i: usize, explicit: Option<Vec<Ty>>, hint: Option<Ty>, args: &[Arg], span: Span) -> Ty {
        let tparams = self.structs[i].tparams.clone();
        let fields = self.structs[i].fields.iter().map(|f| (f.name.clone(), f.ty.clone(), f.optional)).collect();
        let owner = Ty::Struct(i, tparams.iter().map(|n| Ty::Param(n.clone())).collect());
        self.construct(e, Res::StructCtor(i), owner, &tparams, fields, explicit, hint, args, span)
    }

    /// `meta.info(T)`: what is known of the type `T`, written where it is asked for.
    fn meta_info(&mut self, e: &Expr, module: usize, args: &[Arg], span: Span) -> Ty {
        let [arg] = args else {
            self.args_unchecked(args);
            self.err(span, "`info` takes one type, as in `meta.info(Config)`");
            return Ty::Unknown;
        };
        let ty = self.type_of_expr(&arg.value);
        self.types.insert((self.inst, id(&arg.value)), ty.clone());
        let Some(Global::Struct(i)) = self.global_in(module, "Info") else { return Ty::Unknown };
        let mut meta = Meta { name: self.show(&ty), kind: "other", ..Meta::default() };
        match &ty {
            Ty::Struct(s, targs) if *s < self.structs.len() => {
                meta.kind = "struct";
                let tys = self.structs[*s].field_tys(targs);
                meta.fields = self.structs[*s].fields.iter().zip(&tys).map(|(f, t)| (f.name.clone(), self.show(t))).collect();
            }
            Ty::Enum(en, targs) => {
                meta.kind = "enum";
                for (v, variant) in self.enums[*en].variants.iter().enumerate() {
                    let tys = self.enums[*en].field_tys(v, targs);
                    let fields = variant.fields.iter().zip(&tys).map(|((n, _), t)| (n.clone().unwrap_or_default(), self.show(t))).collect();
                    meta.variants.push((variant.name.clone(), fields));
                }
            }
            _ => {}
        }
        self.metas.insert((self.inst, id(e)), meta);
        self.res.insert((self.inst, id(e)), Res::Builtin("meta.info"));
        Ty::Struct(i, Vec::new())
    }

    /// `s.spawn(e)`: `e` is what the task runs. It is checked as the body of
    /// a closure, so it reads the variables around it and cannot change them.
    fn spawn(&mut self, e: &Expr, args: &[Arg], span: Span) -> Ty {
        let [arg] = args else {
            self.args_unchecked(args);
            self.err(span, "`spawn` takes one expression: what the task runs");
            return Ty::Unknown;
        };
        self.closures.push(self.scopes.len());
        self.scopes.push(HashMap::new());
        self.ctx.push(FnCtx { ret: Ty::Unknown, err: None, loops: Vec::new(), by_ref: false });
        let t = self.expr(&arg.value);
        self.ctx.pop();
        self.scopes.pop();
        self.closures.pop();
        self.res.insert((self.inst, id(e)), Res::Builtin("task.spawn"));
        Ty::Task(Box::new(match concretize(&t) {
            Ty::Never => Ty::Unit,
            t => t,
        }))
    }

    /// The methods the language itself gives to `[T]`.
    fn array_method(&mut self, e: &Expr, base: &Expr, bt: &Ty, name: &str, args: &[Arg], span: Span) -> Ty {
        let elem = match bt {
            Ty::Slice(t) | Ty::Array(t) => concretize(t),
            _ => Ty::Unknown,
        };
        let name: &'static str = match name {
            "push" => "push",
            "pop" => "pop",
            _ => "get",
        };
        if name != "get" {
            if matches!(bt, Ty::Array(_)) {
                self.args_unchecked(args);
                self.err(span, format!("a fixed-size array cannot change its length: give the variable the type `[{}]`", self.show(&elem)));
                return Ty::Unknown;
            }
            if elem == Ty::Never {
                self.args_unchecked(args);
                self.err(span, "the element type of this array is not known: give the variable a type, as in `var xs: [int] = []`");
                return Ty::Unknown;
            }
            self.require_mut_place(base);
            self.receiver = self.access(base).map(|a| (a, true));
        }
        self.res.insert((self.inst, id(e)), Res::Builtin(name));
        let param = |name: &str, ty: Ty| ParamSig { name: name.into(), mode: Mode::Let, ty, optional: false };
        let optional = Ty::Optional(Box::new(elem.clone()));
        match name {
            "push" => self.apply(&Sig { params: vec![param("item", elem)], ret: Ty::Unit }, args, span),
            "pop" => self.apply(&Sig { params: Vec::new(), ret: optional }, args, span),
            _ => {
                // Any integer type can be an index, as in `xs[i]`.
                let (tys, logs) = self.arg_types(args);
                let index = match tys.first() {
                    Some(t @ Ty::Int(_)) => t.clone(),
                    _ => Ty::Int("usize"),
                };
                self.arg_logs = Some(logs);
                self.apply_with(&Sig { params: vec![param("index", index)], ret: optional }, args, Some(tys), span)
            }
        }
    }

    /// `Type.name(...)`: a variant constructor or an associated function.
    #[allow(clippy::too_many_arguments)]
    fn type_call(&mut self, e: &Expr, ty: Ty, explicit: bool, name: &str, hint: Option<Ty>, args: &[Arg], span: Span) -> Ty {
        let b = self.key(&ty);
        if let Ty::Enum(i, targs) = &ty {
            if let Some(k) = self.enums[*i].variants.iter().position(|v| v.name == name) {
                let tparams = self.enums[*i].tparams.clone();
                let fields = self.enums[*i].variants[k]
                    .fields
                    .iter()
                    .map(|(n, t)| (n.clone().unwrap_or_default(), t.clone(), false))
                    .collect();
                let owner = Ty::Enum(*i, tparams.iter().map(|n| Ty::Param(n.clone())).collect());
                return self.construct(e, Res::Variant(*i, k), owner, &tparams, fields, explicit.then(|| targs.clone()), hint, args, span);
            }
        }
        // A function several impls of a generic trait give the type.
        let mut chosen = name.to_string();
        if !self.methods.contains_key(&(b.clone(), chosen.clone())) {
            match self.pick(&b, name, &hint, args, span) {
                Ok(Some(m)) => chosen = m,
                Ok(None) => {}
                Err(()) => {
                    self.args_unchecked(args);
                    return Ty::Unknown;
                }
            }
        }
        if let Some(m) = self.methods.get(&(b.clone(), chosen.clone())).cloned() {
            self.method_visible(&m, name, span);
        }
        match self.methods.get(&(b.clone(), chosen.clone())).cloned() {
            Some(m) if m.self_mode.is_none() && m.tparams.is_empty() && m.default.is_none() => {
                self.res.insert((self.inst, id(e)), Res::Assoc(b, chosen));
                self.apply(&m.sig, args, span)
            }
            Some(m) if m.self_mode.is_none() => {
                let mut env = HashMap::new();
                if explicit {
                    bind(&m.self_ty, &ty, &mut env);
                }
                self.generic_call(e, Target::Method(b, chosen), env, None, hint, None, args, span)
            }
            Some(_) => {
                self.args_unchecked(args);
                self.err(span, format!("`{name}` takes `self`: call it on a value of type `{b}`"));
                Ty::Unknown
            }
            None => {
                self.args_unchecked(args);
                self.err(span, format!("`{b}` has no variant or function `{name}`"));
                Ty::Unknown
            }
        }
    }

    /// A type named with type arguments already resolved: `List[int]`.
    fn named(&mut self, name: &str, args: Vec<Ty>, span: Span) -> Ty {
        if let Some(t) = self.tenv.get(name).cloned() {
            if !args.is_empty() {
                self.err(span, format!("`{name}` takes no type arguments"));
            }
            return t;
        }
        if let Some(p) = prim(name) {
            if !args.is_empty() {
                self.err(span, format!("`{name}` takes no type arguments"));
            }
            return p;
        }
        let (generic, want) = match self.global(name) {
            Some(Global::Struct(i)) => (self.structs[i].generic, self.structs[i].tparams.len()),
            Some(Global::Enum(i)) => (self.enums[i].generic, self.enums[i].tparams.len()),
            Some(Global::Alias) => return self.alias_with(name, args, span),
            Some(Global::Trait(_)) => {
                self.err(span, format!("`{name}` is a trait, not a type (use `dyn {name}`)"));
                return Ty::Unknown;
            }
            // `Task[T]` is what `spawn` gives, unless the program declares its own.
            _ if name == "Task" && args.len() == 1 => return Ty::Task(Box::new(args[0].clone())),
            // `Chan[T]` is a channel of the language, on the same terms.
            _ if name == "Chan" && args.len() == 1 => return Ty::Chan(Box::new(args[0].clone())),
            _ if name == "Mutex" && args.len() == 1 => return Ty::Mutex(Box::new(args[0].clone())),
            _ => {
                self.err(span, format!("unknown type `{name}`"));
                return Ty::Unknown;
            }
        };
        if generic {
            return self.skip();
        }
        if args.len() != want {
            if want == 0 {
                self.err(span, format!("`{name}` takes no type arguments"));
            } else {
                self.err(span, format!("`{name}` takes {want} type arguments, found {}: write `{name}[...]`", args.len()));
            }
            return Ty::Unknown;
        }
        match self.global(name) {
            Some(Global::Struct(i)) => Ty::Struct(i, args),
            Some(Global::Enum(i)) => Ty::Enum(i, args),
            _ => Ty::Unknown,
        }
    }

    /// A generic alias with its arguments: `Pair[int]` for `type Pair[T] = (T, T)`.
    fn alias_with(&mut self, name: &str, args: Vec<Ty>, span: Span) -> Ty {
        let file = self.file;
        let decl = match self.aliases.get(&(self.module, name.to_string())) {
            Some(Alias::Todo(i)) => Some(*i),
            _ => None,
        };
        let generics = decl.and_then(|i| match &file.decls[i].kind {
            DeclKind::TypeAlias { generics, ty, .. } if !generics.is_empty() => Some((generics, ty, file.decls[i].span)),
            _ => None,
        });
        let Some((generics, ty, at)) = generics else {
            if !args.is_empty() {
                self.err(span, format!("`{name}` takes no type arguments"));
            }
            return self.resolve_alias(name, span);
        };
        let Some(names) = Self::param_names(generics) else { return self.skip() };
        if names.len() != args.len() {
            self.err(span, format!("`{name}` takes {} type arguments, found {}", names.len(), args.len()));
            return Ty::Unknown;
        }
        if self.alias_depth >= 32 {
            self.err(span, format!("type alias `{name}` refers to itself"));
            return Ty::Unknown;
        }
        let saved = std::mem::replace(&mut self.tenv, targ_env(&names, &args));
        self.alias_depth += 1;
        let t = self.resolve(ty, at);
        self.alias_depth -= 1;
        self.tenv = saved;
        t
    }

    fn resolve_alias(&mut self, name: &str, span: Span) -> Ty {
        let key = (self.module, name.to_string());
        match self.aliases.get(&key) {
            Some(Alias::Done(t)) => t.clone(),
            Some(Alias::Busy) => {
                self.err(span, format!("type alias `{name}` refers to itself"));
                Ty::Unknown
            }
            Some(Alias::Todo(i)) => {
                let i = *i;
                let file = self.file;
                let DeclKind::TypeAlias { generics, ty, .. } = &file.decls[i].kind else { return Ty::Unknown };
                if !generics.is_empty() {
                    return self.skip();
                }
                self.aliases.insert(key.clone(), Alias::Busy);
                let t = self.resolve(ty, file.decls[i].span);
                self.aliases.insert(key, Alias::Done(t.clone()));
                t
            }
            None => Ty::Unknown,
        }
    }

    fn resolve(&mut self, t: &Type, span: Span) -> Ty {
        match t {
            Type::Path { path, args } => {
                let name = &path[0];
                if path.len() == 2 {
                    if let Some(Global::Module(m)) = self.global(name) {
                        let mut targs = Vec::new();
                        for a in args {
                            match a {
                                GenArg::Type(t) => targs.push(self.resolve(t, span)),
                                GenArg::Expr(_) => return self.skip(),
                            }
                        }
                        return self.named_in(m, &path[1], targs, span);
                    }
                }
                if path.len() > 1 {
                    if matches!(self.global(name), Some(Global::Import)) {
                        if self.is_io(name) && path.len() == 2 && path[1] == "Error" {
                            return Ty::Builtin("io.Error");
                        }
                        return self.skip();
                    }
                    self.err(span, format!("unknown type `{}`", path.join(".")));
                    return Ty::Unknown;
                }
                let mut targs = Vec::new();
                for a in args {
                    match a {
                        GenArg::Type(t) => targs.push(self.resolve(t, span)),
                        GenArg::Expr(_) => return self.skip(),
                    }
                }
                self.named(name, targs, span)
            }
            Type::Slice(t) | Type::Soa(t) => Ty::Slice(Box::new(self.resolve(t, span))),
            Type::Array(t, _) => {
                // The length is a compile-time expression; it is not evaluated yet.
                self.report.skipped_exprs += 1;
                Ty::Array(Box::new(self.resolve(t, span)))
            }
            Type::Tuple(ts) if ts.is_empty() => Ty::Unit,
            Type::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| self.resolve(t, span)).collect()),
            Type::Fn { params, ret, err } => {
                let params = params
                    .iter()
                    .map(|(mode, t)| ParamSig {
                        name: String::new(),
                        mode: *mode,
                        ty: self.resolve(t, span),
                        optional: false,
                    })
                    .collect();
                let ret = self.ret_type(ret.as_deref(), err.as_deref(), span);
                Ty::Fn(Box::new(Sig { params, ret }))
            }
            // References are second-class: the checker sees the referent.
            Type::Ref { ty, .. } => self.resolve(ty, span),
            Type::Optional(t) => Ty::Optional(Box::new(self.resolve(t, span))),
            Type::SelfType => match self.self_ty.clone() {
                Some(t) => t,
                None => {
                    self.err(span, "`Self` can only be used inside an `impl`");
                    Ty::Unknown
                }
            },
            Type::Dyn(bounds) => match bounds.as_slice() {
                [one] => match self.trait_of(one, span) {
                    Some((t, targs)) => Ty::Dyn(t, targs),
                    None => {
                        self.err(span, "`dyn` must be followed by a trait");
                        Ty::Unknown
                    }
                },
                _ => {
                    self.err(span, "`dyn` takes one trait: declare a trait that requires the others");
                    Ty::Unknown
                }
            },
            Type::TypeType => self.skip(),
        }
    }

    fn ret_type(&mut self, ret: Option<&Type>, err: Option<&Type>, span: Span) -> Ty {
        let ret = ret.map_or(Ty::Unit, |t| self.resolve(t, span));
        match err {
            Some(e) => Ty::Result(Box::new(ret), Box::new(self.resolve(e, span))),
            None => ret,
        }
    }

    fn signature(&mut self, f: &FnDecl, span: Span) -> (Sig, Option<Mode>) {
        let mut params = Vec::new();
        let mut self_mode = None;
        for p in &f.params {
            if p.name == "self" {
                if self.self_ty.is_none() {
                    self.err(span, "`self` can only be used inside an `impl`");
                }
                self_mode = Some(p.mode);
                continue;
            }
            let ty = match &p.ty {
                Some(t) => self.resolve(t, span),
                None => {
                    self.err(span, format!("parameter `{}` of `{}` needs a type", p.name, f.name));
                    Ty::Unknown
                }
            };
            if let (Some(default), false) = (&p.default, self.decls_only) {
                self.expect(default, &ty, "parameter default");
            }
            params.push(ParamSig { name: p.name.clone(), mode: p.mode, ty, optional: p.default.is_some() });
        }
        let ret = self.ret_type(f.ret.as_ref(), f.err.as_ref(), span);
        (Sig { params, ret }, self_mode)
    }

    fn body(&mut self, f: &FnDecl, sig: &Sig, self_mode: Option<Mode>, span: Span) {
        self.scopes.push(HashMap::new());
        if let (Some(mode), Some(ty)) = (self_mode, self.self_ty.clone()) {
            self.declare("self", ty, mode == Mode::Mut);
            self.set_movable("self", mode == Mode::Own);
        }
        for p in &sig.params {
            self.declare(&p.name, p.ty.clone(), p.mode == Mode::Mut);
            self.set_movable(&p.name, p.mode == Mode::Own);
        }
        for w in &f.where_ {
            self.expect(w, &Ty::Bool, "`where` condition");
        }
        let (ret, err) = match &sig.ret {
            Ty::Result(t, e) => ((**t).clone(), Some((**e).clone())),
            other => (other.clone(), None),
        };
        // A function that returns nothing expects nothing of its last expression.
        self.hint = (ret != Ty::Unit).then(|| ret.clone());
        // A result by reference is part of `self`, or of the first parameter.
        let kind = ref_kind(f);
        let mut source = None;
        if let Some(mutable) = kind {
            let first = match self_mode {
                Some(mode) => Some(("self".to_string(), mode == Mode::Mut)),
                None => sig.params.first().map(|p| (p.name.clone(), p.mode == Mode::Mut)),
            };
            let once = self.inst == 0 || self.inst == DEFINITION;
            match first {
                Some((name, is_mut)) => {
                    if mutable && !is_mut && once {
                        self.err(span, format!("`{}` returns `ref mut`, which is part of `{name}`: take `{name}` as `mut`", f.name));
                    }
                    source = self.lookup(&name).map(|v| (v.id, name));
                }
                None if once => self.err(span, format!("`{}` returns `ref` but has no parameter for the result to be part of", f.name)),
                None => {}
            }
            if err.is_some() && once {
                self.err(span, format!("`{}` returns `ref`, so it cannot fail with `!`", f.name));
            }
        }
        self.ctx.push(FnCtx { ret, err, loops: Vec::new(), by_ref: kind.is_some() });
        if f.body.is_none() {
            self.hint = None;
        }
        if let Some(body) = &f.body {
            let t = self.expr(body);
            if let (Some(mutable), Some((var, name))) = (kind, &source) {
                self.ref_tail(body, *var, mutable, name);
            }
            self.returned(Some(body), &t, span, true);
            if self.inst == 0 || self.inst == DEFINITION {
                self.report.checked_fns += 1;
            }
        }
        self.ctx.pop();
        self.scopes.pop();
    }

    // ---- scopes ----

    fn declare(&mut self, name: &str, ty: Ty, mutable: bool) {
        if mutable && undecided(&ty) {
            let shown = self.show(&ty);
            self.err(self.cur, format!("the type of `{name}` is not fully known (`{shown}`): write it, as in `var {name}: Type = ...`"));
        }
        if let Some(scope) = self.scopes.last_mut() {
            self.next_var += 1;
            scope.insert(name.to_string(), Var { ty, mutable, id: self.next_var, movable: true, deferred: false });
        }
    }

    fn set_movable(&mut self, name: &str, movable: bool) {
        if let Some(v) = self.scopes.last_mut().and_then(|s| s.get_mut(name)) {
            v.movable = movable;
        }
    }

    /// The variable, field or element an expression denotes, if it is one.
    fn access(&self, e: &Expr) -> Option<Access> {
        match &e.kind {
            ExprKind::Path(name) => self.lookup(name).map(|v| Access { root: v.id, name: name.clone(), segs: Vec::new() }),
            ExprKind::Field { base, name } => {
                let mut a = self.access(base)?;
                a.segs.push(Seg::Field(name.clone()));
                Some(a)
            }
            ExprKind::Index { base, args } if args.len() == 1 => {
                let mut a = self.access(base)?;
                a.segs.push(Seg::Index(match &args[0].kind {
                    ExprKind::Int(s) => Some(s.replace('_', "")),
                    _ => None,
                }));
                Some(a)
            }
            // The result of a `ref` function is part of what it comes from.
            ExprKind::Call { .. } => self.access(self.ref_src(e)?.0),
            _ => None,
        }
    }

    /// What a call of a function that returns `ref` takes its result
    /// from, and whether the result can be written to.
    fn ref_src<'e>(&self, e: &'e Expr) -> Option<(&'e Expr, bool)> {
        let ExprKind::Call { callee, args } = &e.kind else { return None };
        let (mutable, receiver) = *self.ref_calls.get(&(self.inst, id(e)))?;
        let src = match (&callee.kind, receiver) {
            (ExprKind::Field { base, .. }, true) => &**base,
            (_, true) => return None,
            (_, false) => &args.first()?.value,
        };
        Some((src, mutable))
    }

    /// The variable a place is part of, seeing through `ref mut` results.
    fn root<'e>(&self, e: &'e Expr) -> Option<&'e str> {
        match &e.kind {
            ExprKind::Path(n) => Some(n),
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.root(base),
            ExprKind::Call { .. } => match self.ref_src(e) {
                Some((src, true)) => self.root(src),
                _ => None,
            },
            _ => None,
        }
    }

    /// Notes that a place is written to: the `ref mut` calls it goes
    /// through must say where their result is.
    fn written(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Field { base, .. } | ExprKind::Index { base, .. } => self.written(base),
            ExprKind::Call { .. } => {
                if let Some((src, true)) = self.ref_src(e) {
                    self.ref_writes.insert((self.inst, id(e)));
                    self.written(src);
                }
            }
            _ => {}
        }
    }

    /// Notes that a call just checked is of a function that returns `ref`.
    fn note_ref_call(&mut self, e: &Expr, callee: &Expr) {
        let file = self.file;
        let of_fn = |d: Option<&Decl>| match d.map(|d| &d.kind) {
            Some(DeclKind::Fn(f)) => ref_kind(f),
            _ => None,
        };
        let based = matches!(callee.kind, ExprKind::Field { .. });
        let method = |c: &Self, t: &str, n: &str, receiver: bool| {
            let m = c.methods.get(&(t.to_string(), n.to_string()))?;
            Some((of_fn(method_decl(file, m))?, receiver && based && m.self_mode.is_some()))
        };
        let function = |c: &Self, i: usize| Some((of_fn(file.decls.get(*c.decl_of_fn.get(i)?))?, false));
        let found = match self.res.get(&(self.inst, id(e))) {
            Some(Res::Fn(i)) => function(self, *i),
            Some(Res::Method(t, n)) => method(self, t, n, true),
            Some(Res::Assoc(t, n)) => method(self, t, n, false),
            Some(Res::Generic(k)) => match self.instances.get(*k).map(|i| &i.f) {
                Some(Target::Fn(i)) => function(self, *i),
                Some(Target::Method(t, n)) => method(self, t, n, true),
                None => None,
            },
            _ => None,
        };
        if let Some(found) = found {
            self.ref_calls.insert((self.inst, id(e)), found);
        }
    }

    /// Checks that the result of a function that returns `ref` is part of
    /// the parameter it must come from.
    fn ref_tail(&mut self, e: &Expr, var: usize, mutable: bool, name: &str) {
        let key = (self.inst, id(e));
        let ok = match &e.kind {
            ExprKind::Block(b) => match &b.tail {
                Some(t) => return self.ref_tail(t, var, mutable, name),
                None => false,
            },
            ExprKind::If { then, els: Some(els), .. } => {
                match &then.tail {
                    Some(t) => self.ref_tail(t, var, mutable, name),
                    None => self.err(e.span, format!("this branch gives no result: a function that returns `ref` ends in a part of `{name}`")),
                }
                return self.ref_tail(els, var, mutable, name);
            }
            ExprKind::Path(_) => self.var_of.get(&key) == Some(&var),
            ExprKind::Field { base, .. } if matches!(self.res.get(&key), Some(Res::Field(_))) => return self.ref_tail(base, var, mutable, name),
            ExprKind::Index { base, args } if args.len() == 1 => {
                let array = matches!(self.types.get(&(self.inst, id(base))), Some(Ty::Slice(_) | Ty::Array(_)));
                let range = matches!(self.types.get(&(self.inst, id(&args[0]))), Some(Ty::Range(_)));
                if array && !range {
                    return self.ref_tail(base, var, mutable, name);
                }
                false
            }
            ExprKind::Call { .. } => match self.ref_src(e) {
                Some((src, m)) if m || !mutable => {
                    if mutable {
                        self.ref_writes.insert(key);
                    }
                    return self.ref_tail(src, var, mutable, name);
                }
                _ => false,
            },
            _ => false,
        };
        if !ok {
            self.err(e.span, format!("the result of a function that returns `ref` must be `{name}`, or a field or an item of it"));
        }
    }

    /// Records a write access, which must not touch what a `for` iterates over.
    fn note_mut(&mut self, e: &Expr) {
        let Some(a) = self.access(e) else { return };
        if self.frozen.iter().any(|f| overlap(f, &a)) {
            self.err(e.span, format!("cannot modify `{}` while a `for` iterates over it", a.name));
        }
        self.mut_log.push((a, e.span));
    }

    /// Whether a name belongs to the function around the current closure.
    /// Closures capture by value, so such a name cannot be written to.
    fn captured(&self, name: &str) -> bool {
        let Some(floor) = self.closures.last() else { return false };
        self.scopes.iter().rposition(|s| s.contains_key(name)).is_some_and(|i| i < *floor)
    }

    fn lookup(&self, name: &str) -> Option<Var> {
        self.scopes.iter().rev().find_map(|s| s.get(name)).cloned()
    }

    // ---- statements ----

    fn block(&mut self, b: &Block) -> Ty {
        let hint = self.pending.take();
        self.scopes.push(HashMap::new());
        let mut diverges = false;
        for s in &b.stmts {
            diverges |= self.stmt(s);
        }
        let t = match &b.tail {
            Some(e) => {
                self.hint = hint;
                self.expr(e)
            }
            None if diverges => Ty::Never,
            None => Ty::Unit,
        };
        self.scopes.pop();
        t
    }

    /// Checks a block whose value is thrown away (loop bodies).
    fn block_discarded(&mut self, b: &Block, span: Span) {
        let t = self.block(b);
        self.discard(&t, b.tail.as_ref().map_or(span, |e| e.span));
    }

    fn discard(&mut self, t: &Ty, span: Span) {
        if matches!(t, Ty::Result(..)) {
            self.err(span, "this result may hold an error and is ignored: use `?` or `catch`");
        }
    }

    /// Returns whether the statement never completes.
    fn stmt(&mut self, s: &Stmt) -> bool {
        match s {
            Stmt::Let { mutable, pat, ty, value } => {
                let mut deferred = false;
                let span = value.as_ref().map_or(self.cur, |v| v.span);
                let declared = ty.as_ref().map(|t| self.resolve(t, span));
                self.hint = declared.clone();
                let actual = value.as_ref().map(|v| (v, self.expr(v)));
                self.hint = None;
                let t = match (declared, actual) {
                    (Some(d), Some((v, a))) => {
                        self.coerce(v, &d, &a, "initializer");
                        d
                    }
                    (Some(d), None) => {
                        deferred = !*mutable;
                        d
                    }
                    // A variable that starts as a list of items can grow.
                    (None, Some((v, a))) => match (concretize(&a), &v.kind) {
                        (Ty::Array(item), ExprKind::Array(_) | ExprKind::ArrayRepeat(..)) if *mutable => Ty::Slice(item),
                        (t, _) => t,
                    },
                    (None, None) => {
                        self.err(span, "a `let` without a value needs a type");
                        Ty::Unknown
                    }
                };
                if !self.irrefutable(pat, &t) {
                    self.err(span, "this pattern may not match: use `if let` or `match`");
                }
                self.pattern(pat, &t, *mutable, span);
                if let (true, Pattern::Bind { name, .. }) = (deferred, pat) {
                    if let Some(v) = self.scopes.last_mut().and_then(|s| s.get_mut(name)) {
                        v.deferred = true;
                    }
                }
                false
            }
            Stmt::Assign { target, op, value } => {
                let tt = self.place(target);
                if *op == "=" {
                    self.expect(value, &tt, "assignment");
                } else {
                    let vt = self.expr(value);
                    let rt = self.binop(&op[..op.len() - 1], &tt, &vt, target.span);
                    if !assignable(&tt, &rt) {
                        let (a, b) = (self.show(&tt), self.show(&rt));
                        self.err(target.span, format!("assignment: expected `{a}`, found `{b}`"));
                    }
                }
                false
            }
            Stmt::Defer(e) => {
                self.expr(e);
                false
            }
            Stmt::Expr(e) => {
                let t = self.expr(e);
                self.discard(&t, e.span);
                t == Ty::Never
            }
            // Checked with the declarations of the module, where the loader put it.
            Stmt::Decl(_) => false,
        }
    }

    /// Type of an assignment target, checking that it can be written to.
    fn place(&mut self, target: &Expr) -> Ty {
        match &target.kind {
            ExprKind::Path(_) | ExprKind::Field { .. } | ExprKind::Index { .. } | ExprKind::Call { .. } => {
                let t = self.expr(target);
                if self.root(target).is_none() && t != Ty::Unknown {
                    self.err(target.span, "this expression cannot be assigned to: it is not a variable, or a field or an item of one");
                }
                self.written(target);
                if let Some(name) = self.root(target) {
                    if self.captured(name) {
                        self.err(target.span, format!("cannot assign to `{name}`: a closure captures it by value"));
                    }
                    self.note_mut(target);
                    match self.lookup(name) {
                        // The one assignment that gives a deferred `let` its value.
                        Some(v) if v.deferred && matches!(target.kind, ExprKind::Path(_)) => {}
                        Some(v) if !v.mutable => self.err(
                            target.span,
                            format!("cannot assign to `{name}`: it is immutable (declare it with `var` or take it as `mut`)"),
                        ),
                        Some(_) => {}
                        None if matches!(self.global(name), Some(Global::Const(_))) => {
                            self.err(target.span, format!("cannot assign to constant `{name}`"))
                        }
                        None => {}
                    }
                }
                t
            }
            _ => {
                self.expr(target);
                self.err(target.span, "this expression cannot be assigned to");
                Ty::Unknown
            }
        }
    }

    fn require_mut_place(&mut self, e: &Expr) {
        self.written(e);
        match self.root(e) {
            Some(name) => {
                if self.captured(name) {
                    self.err(e.span, format!("cannot use `{name}` as `mut`: a closure captures it by value"));
                }
                if let Some(Var { mutable: false, .. }) = self.lookup(name) {
                    self.err(e.span, format!("cannot use `{name}` as `mut`: it is not mutable"));
                }
                self.note_mut(e);
            }
            None => self.err(e.span, "a `mut` access needs a variable, field or element"),
        }
    }

    // ---- patterns ----

    fn irrefutable(&self, p: &Pattern, ty: &Ty) -> bool {
        match p {
            Pattern::Wildcard | Pattern::Bind { .. } | Pattern::Rest => true,
            Pattern::Tuple(ps) => match ty {
                Ty::Tuple(ts) if ts.len() == ps.len() => ps.iter().zip(ts).all(|(p, t)| self.irrefutable(p, t)),
                _ => ps.iter().all(|p| self.irrefutable(p, &Ty::Unknown)),
            },
            Pattern::Ctor { args, .. } => match ty {
                Ty::Struct(i, targs) => {
                    let fields = self.structs[*i].field_tys(targs);
                    args.len() == fields.len() && args.iter().zip(&fields).all(|(p, t)| self.irrefutable(p, t))
                }
                _ => false,
            },
            Pattern::Struct { fields, .. } => match ty {
                Ty::Struct(i, targs) => fields.iter().all(|(name, sub)| match sub {
                    None => true,
                    Some(p) => {
                        let tys = self.structs[*i].field_tys(targs);
                        let k = self.structs[*i].fields.iter().position(|f| f.name == *name);
                        self.irrefutable(p, k.map_or(&Ty::Unknown, |k| &tys[k]))
                    }
                }),
                _ => false,
            },
            Pattern::Or(alts) => alts.iter().any(|p| self.irrefutable(p, ty)),
            Pattern::Literal(_) | Pattern::Range { .. } | Pattern::Slice(_) => false,
        }
    }

    fn patterns(&mut self, ps: &[Pattern], tys: &[Ty], mutable: bool, span: Span) {
        for (i, p) in ps.iter().enumerate() {
            let t = tys.get(i).cloned().unwrap_or(Ty::Unknown);
            self.pattern(p, &t, mutable, span);
        }
    }

    /// Checks a pattern against a type and declares its bindings.
    fn pattern(&mut self, p: &Pattern, ty: &Ty, mutable: bool, span: Span) {
        match p {
            Pattern::Wildcard | Pattern::Rest => {}
            Pattern::Bind { mutable: m, name } => self.declare(name, concretize(ty), *m || mutable),
            Pattern::Literal(e) => {
                let t = self.expr(e);
                if join(ty, &t).is_none() {
                    let (a, b) = (self.show(&t), self.show(ty));
                    self.err(e.span, format!("a pattern of type `{a}` cannot match a value of type `{b}`"));
                }
                self.lit_range(e, ty);
            }
            Pattern::Range { lo, hi, .. } => {
                for e in std::iter::once(lo).chain(hi) {
                    let t = self.expr(e);
                    if join(ty, &t).is_none() {
                        let (a, b) = (self.show(&t), self.show(ty));
                        self.err(e.span, format!("a pattern of type `{a}` cannot match a value of type `{b}`"));
                    }
                }
            }
            Pattern::Ctor { path, args } => {
                let name = path.last().cloned().unwrap_or_default();
                let shown = self.show(ty);
                let fields: Option<Vec<Ty>> = match ty {
                    Ty::Unknown => None,
                    Ty::Optional(t) if path.len() == 1 && name == "Some" => Some(vec![(**t).clone()]),
                    Ty::Optional(_) if path.len() == 1 && name == "None" => Some(Vec::new()),
                    Ty::Enum(i, targs) => {
                        let en = &self.enums[*i];
                        if path.len() == 2 && path[0] != en.name {
                            let msg = format!("pattern `{}` cannot match a value of type `{shown}`", path.join("."));
                            self.err(span, msg);
                            None
                        } else {
                            match en.variants.iter().position(|v| v.name == name) {
                                Some(v) => Some(en.field_tys(v, targs)),
                                None => {
                                    self.err(span, format!("`{shown}` has no variant `{name}`"));
                                    None
                                }
                            }
                        }
                    }
                    Ty::Struct(i, targs) if path.len() == 1 && self.structs[*i].name == name => {
                        Some(self.structs[*i].field_tys(targs))
                    }
                    _ => {
                        let msg = format!("pattern `{}` cannot match a value of type `{shown}`", path.join("."));
                        self.err(span, msg);
                        None
                    }
                };
                match fields {
                    Some(fields) => {
                        if fields.len() != args.len() {
                            let msg = format!("`{name}` has {} fields, the pattern has {}", fields.len(), args.len());
                            self.err(span, msg);
                        }
                        self.patterns(args, &fields, mutable, span);
                    }
                    None => self.patterns(args, &[], mutable, span),
                }
            }
            Pattern::Struct { path, fields, rest } => {
                let name = path.last().cloned().unwrap_or_default();
                let known: Option<Vec<(String, Ty)>> = match ty {
                    Ty::Struct(i, targs) if self.structs[*i].name == name => {
                        let st = &self.structs[*i];
                        Some(st.fields.iter().map(|f| f.name.clone()).zip(st.field_tys(targs)).collect())
                    }
                    Ty::Enum(i, targs) => self.enums[*i].variants.iter().position(|v| v.name == name).map(|v| {
                        let en = &self.enums[*i];
                        en.variants[v].fields.iter().map(|(n, _)| n.clone().unwrap_or_default()).zip(en.field_tys(v, targs)).collect()
                    }),
                    _ => None,
                };
                if known.is_none() && *ty != Ty::Unknown {
                    let shown = self.show(ty);
                    self.err(span, format!("pattern `{}` cannot match a value of type `{shown}`", path.join(".")));
                }
                for (fname, sub) in fields {
                    let ft = match &known {
                        Some(k) => match k.iter().find(|(n, _)| n == fname) {
                            Some((_, t)) => t.clone(),
                            None => {
                                self.err(span, format!("`{name}` has no field `{fname}`"));
                                Ty::Unknown
                            }
                        },
                        None => Ty::Unknown,
                    };
                    match sub {
                        Some(p) => self.pattern(p, &ft, mutable, span),
                        None => self.declare(fname, ft, mutable),
                    }
                }
                if let Some(k) = &known {
                    if !rest && fields.len() < k.len() {
                        self.err(span, format!("pattern does not mention every field of `{name}`: add `..`"));
                    }
                }
            }
            Pattern::Tuple(ps) => match ty {
                Ty::Tuple(ts) => {
                    if ts.len() != ps.len() && !ps.iter().any(|p| matches!(p, Pattern::Rest)) {
                        let msg = format!("the tuple has {} elements, the pattern has {}", ts.len(), ps.len());
                        self.err(span, msg);
                    }
                    self.patterns(ps, ts, mutable, span);
                }
                Ty::Unknown => self.patterns(ps, &[], mutable, span),
                _ => {
                    let shown = self.show(ty);
                    self.err(span, format!("a tuple pattern cannot match a value of type `{shown}`"));
                    self.patterns(ps, &[], mutable, span);
                }
            },
            Pattern::Slice(ps) => {
                let elem = match ty {
                    Ty::Slice(t) | Ty::Array(t) => (**t).clone(),
                    Ty::Unknown => Ty::Unknown,
                    _ => {
                        let shown = self.show(ty);
                        self.err(span, format!("a slice pattern cannot match a value of type `{shown}`"));
                        Ty::Unknown
                    }
                };
                for p in ps {
                    self.pattern(p, &elem, mutable, span);
                }
            }
            Pattern::Or(alts) => {
                let mut first: Option<HashMap<String, Var>> = None;
                for alt in alts {
                    self.scopes.push(HashMap::new());
                    self.pattern(alt, ty, mutable, span);
                    let bound = self.scopes.pop().unwrap_or_default();
                    match &first {
                        None => first = Some(bound),
                        Some(f) => {
                            if f.len() != bound.len() || !f.keys().all(|k| bound.contains_key(k)) {
                                self.err(span, "every alternative of an `|` pattern must bind the same names");
                            }
                        }
                    }
                }
                for (name, var) in first.unwrap_or_default() {
                    self.declare(&name, var.ty, var.mutable);
                }
            }
        }
    }

    fn exhaustive(&mut self, ty: &Ty, arms: &[Arm], span: Span) {
        let mut tops: Vec<&Pattern> = Vec::new();
        for arm in arms.iter().filter(|a| a.guard.is_none()) {
            match &arm.pat {
                Pattern::Or(alts) => tops.extend(alts),
                p => tops.push(p),
            }
        }
        if *ty == Ty::Unknown || tops.iter().any(|p| self.irrefutable(p, ty)) {
            return;
        }
        let covers = |name: &str, fields: &[Ty]| {
            tops.iter().any(|p| match p {
                Pattern::Ctor { path, args } => {
                    path.last().is_some_and(|n| n == name)
                        && args.iter().enumerate().all(|(i, a)| self.irrefutable(a, fields.get(i).unwrap_or(&Ty::Unknown)))
                }
                Pattern::Struct { path, fields: fs, .. } => {
                    path.last().is_some_and(|n| n == name)
                        && fs.iter().all(|(_, sub)| sub.as_ref().is_none_or(|p| self.irrefutable(p, &Ty::Unknown)))
                }
                _ => false,
            })
        };
        let missing: Vec<String> = match ty {
            Ty::Enum(i, targs) => self.enums[*i]
                .variants
                .iter()
                .enumerate()
                .filter(|(k, v)| !covers(&v.name, &self.enums[*i].field_tys(*k, targs)))
                .map(|(_, v)| v.name.clone())
                .collect(),
            Ty::Optional(t) => [("Some", vec![(**t).clone()]), ("None", Vec::new())]
                .iter()
                .filter(|(n, f)| !covers(n, f))
                .map(|(n, _)| n.to_string())
                .collect(),
            Ty::Bool => [true, false]
                .iter()
                .filter(|b| {
                    !tops.iter().any(|p| matches!(p, Pattern::Literal(e) if matches!(e.kind, ExprKind::Bool(x) if x == **b)))
                })
                .map(|b| b.to_string())
                .collect(),
            _ => {
                let shown = self.show(ty);
                self.err(span, format!("this `match` on `{shown}` does not cover every value: add a `_` arm"));
                return;
            }
        };
        if !missing.is_empty() {
            let list: Vec<String> = missing.iter().map(|m| format!("`{m}`")).collect();
            self.err(span, format!("this `match` does not cover every value: missing {}", list.join(", ")));
        }
    }

    // ---- expressions ----

    fn expect(&mut self, e: &Expr, want: &Ty, what: &str) -> Ty {
        self.hint = Some(want.clone());
        let got = self.expr(e);
        self.coerce(e, want, &got, what);
        got
    }

    fn coerce(&mut self, e: &Expr, want: &Ty, got: &Ty, what: &str) {
        if assignable(want, got) {
            self.wants.insert((self.inst, id(e)), want.clone());
            self.lit_range(e, want);
        } else {
            let (a, b) = (self.show(want), self.show(got));
            self.err(tail_of(e).span, format!("{what}: expected `{a}`, found `{b}`"));
        }
    }

    /// Reports an integer literal that does not fit the type it is used as.
    fn lit_range(&mut self, e: &Expr, want: &Ty) {
        let Ty::Int(kind) = want else { return };
        let e = tail_of(e);
        let (negative, digits) = match &e.kind {
            ExprKind::Int(s) => (false, s),
            ExprKind::Unary { op: "-", expr } => match &expr.kind {
                ExprKind::Int(s) => (true, s),
                _ => return,
            },
            _ => return,
        };
        if let Some(v) = int_value(digits) {
            if !fits(kind, negative, v) {
                let sign = if negative { "-" } else { "" };
                self.err(e.span, format!("literal `{sign}{digits}` is out of range for `{kind}`"));
            }
        }
    }

    fn returned(&mut self, e: Option<&Expr>, t: &Ty, span: Span, tail: bool) {
        let Some(ctx) = self.ctx.last() else {
            self.err(span, "`return` outside of a function");
            return;
        };
        let (ret, err) = (ctx.ret.clone(), ctx.err.clone());
        if assignable(&ret, t) {
            if let Some(e) = e {
                self.lit_range(e, &ret);
            }
            return;
        }
        // A value of the declared error type is an error return.
        if err.as_ref().is_some_and(|e| *e != Ty::Unknown && assignable(e, t)) {
            return;
        }
        let at = e.map_or(span, |e| tail_of(e).span);
        if tail && ret == Ty::Unit {
            self.discard(t, at);
            return;
        }
        let (a, b) = (self.show(&ret), self.show(t));
        self.err(at, format!("expected return type `{a}`, found `{b}`"));
    }

    fn cond(&mut self, c: &Cond) {
        match c {
            Cond::Expr(e) => {
                self.expect(e, &Ty::Bool, "condition");
            }
            Cond::Let(pat, e) => {
                let t = self.expr(e);
                self.pattern(pat, &t, false, e.span);
            }
        }
    }

    fn expr(&mut self, e: &Expr) -> Ty {
        self.pending = self.hint.take();
        let t = self.expr_inner(e);
        self.types.insert((self.inst, id(e)), t.clone());
        t
    }

    fn expr_inner(&mut self, e: &Expr) -> Ty {
        let span = e.span;
        self.cur = span;
        match &e.kind {
            ExprKind::Int(s) => {
                if int_value(s).is_none() {
                    self.err(span, format!("invalid integer literal `{s}`"));
                }
                Ty::IntLit
            }
            ExprKind::Float(_) => Ty::FloatLit,
            ExprKind::Char(_) => Ty::Char,
            ExprKind::Bool(_) => Ty::Bool,
            ExprKind::Str(parts) => {
                for part in parts {
                    if let StrPart::Expr(inner) = part {
                        let t = self.expr(inner);
                        self.discard(&t, inner.span);
                    }
                }
                Ty::Str
            }
            ExprKind::Path(name) => self.name(e, name, span),
            ExprKind::Tuple(items) if items.is_empty() => Ty::Unit,
            ExprKind::Tuple(items) => Ty::Tuple(items.iter().map(|i| self.expr(i)).collect()),
            ExprKind::Array(items) => {
                let mut elem = Ty::Never;
                for item in items {
                    let t = self.expr(item);
                    match join(&elem, &t) {
                        Some(j) => elem = j,
                        None => {
                            let (a, b) = (self.show(&elem), self.show(&t));
                            self.err(item.span, format!("array elements have different types: `{a}` and `{b}`"));
                        }
                    }
                }
                Ty::Array(Box::new(elem))
            }
            ExprKind::ArrayRepeat(value, len) => {
                let t = self.expr(value);
                self.expect(len, &Ty::Int("usize"), "array length");
                Ty::Array(Box::new(t))
            }
            ExprKind::Block(b) => self.block(b),
            ExprKind::If { cond, then, els } => {
                let hint = self.pending.take();
                self.scopes.push(HashMap::new());
                self.cond(cond);
                self.pending = hint.clone();
                let tt = self.block(then);
                self.scopes.pop();
                match els {
                    None => {
                        self.discard(&tt, span);
                        Ty::Unit
                    }
                    Some(els) => {
                        self.hint = hint;
                        let et = self.expr(els);
                        join(&tt, &et).unwrap_or_else(|| {
                            let (a, b) = (self.show(&tt), self.show(&et));
                            self.err(span, format!("`if` branches have different types: `{a}` and `{b}`"));
                            Ty::Unknown
                        })
                    }
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let hint = self.pending.take();
                let st = self.expr(scrutinee);
                let mut result = Ty::Never;
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    self.pattern(&arm.pat, &st, false, arm.body.span);
                    if let Some(g) = &arm.guard {
                        self.expect(g, &Ty::Bool, "guard");
                    }
                    self.hint = hint.clone();
                    let bt = self.expr(&arm.body);
                    self.scopes.pop();
                    match join(&result, &bt) {
                        Some(j) => result = j,
                        None => {
                            let (a, b) = (self.show(&result), self.show(&bt));
                            self.err(arm.body.span, format!("`match` arms have different types: `{a}` and `{b}`"));
                        }
                    }
                }
                self.exhaustive(&st, arms, span);
                result
            }
            ExprKind::For { pat, iter, body } => {
                let it = self.expr(iter);
                let elem = match &it {
                    Ty::Slice(t) | Ty::Array(t) | Ty::Range(t) => concretize(t),
                    Ty::Str => Ty::Char,
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        let shown = self.show(other);
                        self.err(iter.span, format!("cannot iterate over a value of type `{shown}`"));
                        Ty::Unknown
                    }
                };
                self.scopes.push(HashMap::new());
                if !self.irrefutable(pat, &elem) {
                    self.err(iter.span, "the pattern of a `for` must always match");
                }
                self.pattern(pat, &elem, false, iter.span);
                let frozen = self.access(iter).filter(|_| !matches!(it, Ty::Range(_)));
                let depth = self.frozen.len();
                self.frozen.extend(frozen);
                self.looped(body, false, span);
                self.frozen.truncate(depth);
                self.scopes.pop();
                Ty::Unit
            }
            ExprKind::While { cond, body } => {
                self.scopes.push(HashMap::new());
                self.cond(cond);
                self.looped(body, false, span);
                self.scopes.pop();
                Ty::Unit
            }
            ExprKind::Loop(body) => self.looped(body, true, span).unwrap_or(Ty::Never),
            ExprKind::Break(value) => {
                let t = value.as_ref().map_or(Ty::Unit, |v| self.expr(v));
                let shown = self.show(&t);
                match self.ctx.last_mut().and_then(|c| c.loops.last_mut()) {
                    None => self.err(span, "`break` outside of a loop"),
                    Some(l) if value.is_some() && !l.valued => {
                        self.err(span, "only `loop` can `break` with a value")
                    }
                    Some(l) => match join(l.ty.as_ref().unwrap_or(&Ty::Never), &t) {
                        Some(j) => l.ty = Some(j),
                        None => self.err(span, format!("`break` values have different types (found `{shown}`)")),
                    },
                }
                Ty::Never
            }
            ExprKind::Continue => {
                if self.ctx.last().is_none_or(|c| c.loops.is_empty()) {
                    self.err(span, "`continue` outside of a loop");
                }
                Ty::Never
            }
            ExprKind::Return(value) => {
                if self.ctx.last().is_some_and(|c| c.by_ref) {
                    self.err(span, "a function that returns `ref` gives its result as the last expression of its body, not with `return`");
                }
                self.hint = value.as_ref().and_then(|_| self.ctx.last().map(|c| c.ret.clone())).filter(|t| *t != Ty::Unit);
                let t = value.as_ref().map_or(Ty::Unit, |v| self.expr(v));
                self.returned(value.as_deref(), &t, span, false);
                Ty::Never
            }
            ExprKind::Closure { params, ret, err, body } => {
                self.closures.push(self.scopes.len());
                self.scopes.push(HashMap::new());
                let mut ps = Vec::new();
                for p in params {
                    let ty = match &p.ty {
                        Some(t) => self.resolve(t, span),
                        None => self.skip(),
                    };
                    self.declare(&p.name, ty.clone(), p.mode == Mode::Mut);
                    self.set_movable(&p.name, p.mode == Mode::Own);
                    ps.push(ParamSig { name: p.name.clone(), mode: p.mode, ty, optional: false });
                }
                let declared = ret.as_ref().map(|t| self.resolve(t, span));
                let err = err.as_ref().map(|t| self.resolve(t, span));
                self.ctx.push(FnCtx {
                    ret: declared.clone().unwrap_or(Ty::Unknown),
                    err: err.clone(),
                    loops: Vec::new(),
                    by_ref: false,
                });
                let bt = self.block(body);
                if declared.is_some() {
                    self.returned(None, &bt, body.tail.as_ref().map_or(span, |t| t.span), true);
                }
                self.ctx.pop();
                self.scopes.pop();
                self.closures.pop();
                let ret = declared.unwrap_or_else(|| match concretize(&bt) {
                    Ty::Never => Ty::Unit,
                    t => t,
                });
                let ret = match err {
                    Some(e) => Ty::Result(Box::new(ret), Box::new(e)),
                    None => ret,
                };
                Ty::Fn(Box::new(Sig { params: ps, ret }))
            }
            ExprKind::Comptime(inner) => self.expr(inner),
            ExprKind::Unsafe(b) => {
                if !self.in_system() {
                    self.err(e.span, "`unsafe` is only allowed in a module that starts with `@profile(system)`");
                }
                self.unsafe_depth += 1;
                let t = self.block(b);
                self.unsafe_depth -= 1;
                t
            }
            ExprKind::Scope { name, body } => {
                self.scopes.push(HashMap::new());
                self.declare(name, Ty::Builtin("scope"), false);
                let t = self.block(body);
                self.scopes.pop();
                t
            }
            ExprKind::With { bindings, body } => {
                // The contexts there are: where memory is asked from, and
                // where what is printed goes.
                let arena = match bindings.as_slice() {
                    [(name, value)] => name == "alloc" && matches!(&value.kind, ExprKind::Path(n) if n == "arena" && self.lookup(n).is_none()),
                    _ => false,
                };
                match bindings.as_slice() {
                    [(name, value)] if name == "io" => {
                        let t = self.expr(value);
                        if !assignable(&Ty::Slice(Box::new(Ty::Str)), &t) {
                            let shown = self.show(&t);
                            self.err(value.span, format!("`with io = ...` takes a variable of type `[str]`, where the lines the block prints are kept, and this is `{shown}`"));
                        }
                        self.require_mut_place(value);
                    }
                    _ if !arena => self.err(e.span, "the contexts there are for now are `with alloc = arena { }` and `with io = lines { }`"),
                    _ => {}
                }
                self.scopes.push(HashMap::new());
                let t = self.block(body);
                self.scopes.pop();
                t
            }
            ExprKind::Unary { op, expr } => {
                let t = self.expr(expr);
                let ok = match *op {
                    "-" => is_num(&t) && !matches!(t, Ty::Int(k) if k.starts_with('u')),
                    "!" => matches!(t, Ty::Bool | Ty::Unknown),
                    _ => is_int(&t),
                };
                if !ok {
                    let shown = self.show(&t);
                    self.err(span, format!("operator `{op}` cannot be applied to `{shown}`"));
                    return Ty::Unknown;
                }
                t
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let lt = self.expr(lhs);
                let rt = self.expr(rhs);
                self.binop(op, &lt, &rt, span)
            }
            ExprKind::Range { lo, hi, .. } => {
                let mut t = Ty::IntLit;
                let mut first = true;
                for bound in [lo, hi].into_iter().flatten() {
                    let bt = self.expr(bound);
                    let joined = if first { Some(bt.clone()) } else { join(&t, &bt) };
                    first = false;
                    match joined {
                        Some(j) if is_int(&j) || j == Ty::Char => t = j,
                        _ => {
                            let shown = self.show(&bt);
                            self.err(bound.span, format!("a range bound cannot have type `{shown}`"));
                            t = Ty::Unknown;
                        }
                    }
                }
                Ty::Range(Box::new(t))
            }
            ExprKind::Cast { expr, ty } => {
                let from = self.expr(expr);
                let to = self.resolve(ty, span);
                if let Ty::Dyn(t, targs) = &to {
                    let from = concretize(&from);
                    let (shown, tname) = (self.show(&from), self.show_bound(&(*t, targs.clone())));
                    match &from {
                        Ty::Unknown => {}
                        Ty::Dyn(u, uargs) if u == t && uargs == targs => {}
                        Ty::Dyn(..) => self.err(span, format!("cannot cast `{shown}` to `dyn {tname}`: a `dyn` value only keeps the methods of its own trait")),
                        _ if !self.satisfies(&from, &(*t, targs.clone())) => self.err(span, format!("cannot cast `{shown}` to `dyn {tname}`: `{shown}` does not implement `{tname}`")),
                        _ if self.inst != DEFINITION && concrete(&from) => {
                            let table = self.dyn_table(&from, &(*t, targs.clone()));
                            self.dyns.insert((self.inst, id(e)), table);
                        }
                        _ => {}
                    }
                    return to;
                }
                let ok = match (&from, &to) {
                    (Ty::Unknown, _) | (_, Ty::Unknown) => true,
                    (f, t) if is_num(f) && is_num(t) => true,
                    (Ty::Char | Ty::Bool, Ty::Int(_)) => true,
                    // A whole number is a character when it is the code of one:
                    // checked when the program runs.
                    (f, Ty::Char) if is_int(f) => true,
                    _ => false,
                };
                if !ok {
                    let (a, b) = (self.show(&from), self.show(&to));
                    self.err(span, format!("cannot cast `{a}` to `{b}`"));
                }
                to
            }
            ExprKind::Call { callee, args } => {
                let t = self.call(e, callee, args, span);
                self.note_ref_call(e, callee);
                t
            }
            ExprKind::Index { base, args } => {
                if let ExprKind::Path(n) = &base.kind {
                    if self.lookup(n).is_none() {
                        let generic = match self.global(n) {
                            Some(Global::Fn(i)) => self.fns[i].generic || !self.fns[i].tparams.is_empty(),
                            Some(Global::Struct(i)) => self.structs[i].generic,
                            Some(Global::Enum(i)) => self.enums[i].generic,
                            Some(Global::Alias) => true,
                            _ => false,
                        };
                        if generic {
                            return self.skip();
                        }
                    }
                }
                let bt = self.expr(base);
                let its: Vec<Ty> = args.iter().map(|a| self.expr(a)).collect();
                if its.len() != 1 {
                    self.err(span, "an index takes exactly one value");
                    return Ty::Unknown;
                }
                let ranged = match &its[0] {
                    Ty::Range(r) if is_int(r) => true,
                    t if is_int(t) => false,
                    other => {
                        let shown = self.show(other);
                        self.err(args[0].span, format!("an index must be an integer or a range, found `{shown}`"));
                        return Ty::Unknown;
                    }
                };
                match bt {
                    Ty::Slice(t) | Ty::Array(t) if ranged => Ty::Slice(t),
                    Ty::Slice(t) | Ty::Array(t) => *t,
                    Ty::Str if ranged => Ty::Str,
                    Ty::Str => {
                        self.err(span, "a `str` is UTF-8 and can only be indexed with a range");
                        Ty::Unknown
                    }
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        let shown = self.show(&other);
                        self.err(span, format!("a value of type `{shown}` cannot be indexed"));
                        Ty::Unknown
                    }
                }
            }
            ExprKind::Field { base, name } => self.field(e, base, name, span),
            ExprKind::Try(inner) => {
                let t = self.expr(inner);
                let (ret, err) = match self.ctx.last() {
                    Some(c) => (c.ret.clone(), c.err.clone()),
                    None => (Ty::Unit, None),
                };
                match t {
                    Ty::Result(ok, found) => {
                        match &err {
                            None => self.err(span, "`?` on a result needs a function that declares an error (`! E`)"),
                            Some(declared) if !assignable(declared, &found) => {
                                let (a, b) = (self.show(&found), self.show(declared));
                                self.err(span, format!("`?` would return an error of type `{a}`, but the function declares `{b}`"));
                            }
                            Some(_) => {}
                        }
                        *ok
                    }
                    Ty::Optional(some) => {
                        if !matches!(ret, Ty::Optional(_) | Ty::Unknown) {
                            self.err(span, "`?` on an optional needs a function that returns an optional");
                        }
                        *some
                    }
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        let shown = self.show(&other);
                        self.err(span, format!("`?` needs a result or an optional, found `{shown}`"));
                        Ty::Unknown
                    }
                }
            }
            ExprKind::Catch { expr, binding, handler } => {
                let t = self.expr(expr);
                let (ok, err) = match t {
                    Ty::Result(ok, err) => (*ok, *err),
                    Ty::Unknown => (Ty::Unknown, Ty::Unknown),
                    other => {
                        let shown = self.show(&other);
                        self.err(span, format!("`catch` needs a result with an error, found `{shown}`"));
                        (Ty::Unknown, Ty::Unknown)
                    }
                };
                self.scopes.push(HashMap::new());
                if let Some(b) = binding {
                    self.declare(b, err, false);
                }
                let ht = self.block(handler);
                self.scopes.pop();
                join(&ok, &ht).unwrap_or_else(|| {
                    let (a, b) = (self.show(&ok), self.show(&ht));
                    self.err(span, format!("the `catch` handler must produce `{a}`, found `{b}`"));
                    Ty::Unknown
                })
            }
        }
    }

    /// Checks a loop body. Returns the type of its `break` values, if any.
    fn looped(&mut self, body: &Block, valued: bool, span: Span) -> Option<Ty> {
        match self.ctx.last_mut() {
            Some(c) => c.loops.push(Loop { ty: None, valued }),
            None => {
                self.err(span, "a loop cannot appear outside of a function");
                return Some(Ty::Unknown);
            }
        }
        self.block_discarded(body, span);
        self.ctx.last_mut().and_then(|c| c.loops.pop()).and_then(|l| l.ty)
    }

    fn name(&mut self, e: &Expr, name: &str, span: Span) -> Ty {
        if let Some(v) = self.lookup(name) {
            self.res.insert((self.inst, id(e)), Res::Local);
            self.var_of.insert((self.inst, id(e)), v.id);
            return v.ty;
        }
        if name == "None" {
            self.res.insert((self.inst, id(e)), Res::NoneValue);
            return Ty::Optional(Box::new(Ty::Never));
        }
        self.global_value(e, self.module, name, span)
    }

    /// A top-level name of a module used as a value.
    fn global_value(&mut self, e: &Expr, module: usize, name: &str, span: Span) -> Ty {
        match self.global_in(module, name) {
            Some(Global::Fn(i)) if self.fns[i].generic || !self.fns[i].tparams.is_empty() => self.skip(),
            Some(Global::Fn(i)) => {
                self.res.insert((self.inst, id(e)), Res::Fn(i));
                Ty::Fn(Box::new(self.fns[i].sig.clone()))
            }
            Some(Global::Const(i)) => match self.consts[i].clone() {
                Some(t) => {
                    self.res.insert((self.inst, id(e)), Res::Const(i));
                    t
                }
                None => {
                    self.err(span, format!("constant `{name}` is used before its definition"));
                    Ty::Unknown
                }
            },
            Some(Global::Import) => self.skip(),
            Some(Global::Module(_)) => {
                self.err(span, format!("`{name}` is a module, not a value"));
                Ty::Unknown
            }
            Some(_) => {
                self.err(span, format!("`{name}` is a type, not a value"));
                Ty::Unknown
            }
            None => {
                self.err(span, format!("undefined name `{name}`"));
                Ty::Unknown
            }
        }
    }

    fn field(&mut self, e: &Expr, base: &Expr, name: &str, span: Span) -> Ty {
        let hint = self.pending.take();
        if self.in_module(e) {
            return match self.gref(e) {
                Some((m, name)) => self.global_value(e, m, name, span),
                None => Ty::Unknown,
            };
        }
        if let ExprKind::Path(b) = &base.kind {
            if self.lookup(b).is_none() {
                match self.global(b) {
                    Some(Global::Import) => return self.skip(),
                    Some(Global::Enum(i)) if self.enums[i].generic => return self.skip(),
                    _ => {}
                }
            }
        }
        if let Some((Ty::Enum(i, targs), explicit)) = self.type_head(base) {
            let b = self.enums[i].name.clone();
            return match self.enums[i].variants.iter().position(|v| v.name == name) {
                Some(v) if self.enums[i].variants[v].fields.is_empty() => {
                    self.res.insert((self.inst, id(e)), Res::Variant(i, v));
                    let tparams = self.enums[i].tparams.clone();
                    let args = if explicit {
                        targs
                    } else {
                        let env = self.type_args(&tparams, &Ty::Enum(i, targs), None, hint, &[], &[]);
                        tparams.iter().map(|n| env[n].clone()).collect()
                    };
                    Ty::Enum(i, args)
                }
                Some(_) => {
                    self.err(span, format!("variant `{b}.{name}` needs arguments"));
                    Ty::Unknown
                }
                None => {
                    self.err(span, format!("`{b}` has no variant `{name}`"));
                    Ty::Unknown
                }
            };
        }
        let bt = self.expr(base);
        let found = match &bt {
            Ty::Unknown => return Ty::Unknown,
            Ty::Struct(i, targs) => self.structs[*i].fields.iter().position(|f| f.name == name).map(|k| {
                self.res.insert((self.inst, id(e)), Res::Field(k));
                subst(&self.structs[*i].fields[k].ty, &targ_env(&self.structs[*i].tparams, targs))
            }),
            Ty::Tuple(ts) => name.parse::<usize>().ok().and_then(|i| {
                self.res.insert((self.inst, id(e)), Res::Field(i));
                ts.get(i).cloned()
            }),
            Ty::Slice(_) | Ty::Array(_) | Ty::Str if name == "len" => {
                self.res.insert((self.inst, id(e)), Res::Len);
                Some(Ty::Int("usize"))
            }
            _ => None,
        };
        found.unwrap_or_else(|| {
            let shown = self.show(&bt);
            self.err(span, format!("type `{shown}` has no field `{name}`"));
            Ty::Unknown
        })
    }

    fn binop(&mut self, op: &str, l: &Ty, r: &Ty, span: Span) -> Ty {
        let joined = join(l, r);
        let result = match op {
            "&&" | "||" => (matches!(l, Ty::Bool | Ty::Unknown) && matches!(r, Ty::Bool | Ty::Unknown)).then_some(Ty::Bool),
            "==" | "!=" => joined.map(|_| Ty::Bool),
            "<" | "<=" | ">" | ">=" => {
                joined.filter(|j| is_num(j) || matches!(j, Ty::Char | Ty::Str)).map(|_| Ty::Bool)
            }
            "<<" | ">>" => (is_int(l) && is_int(r)).then(|| l.clone()),
            "&" | "|" | "^" => joined.filter(|j| is_int(j) || *j == Ty::Bool),
            "+" | "-" | "*" | "/" | "%" => joined.filter(is_num),
            // Wrapping and saturating arithmetic only exists for integers.
            _ => joined.filter(is_int),
        };
        result.unwrap_or_else(|| {
            let (a, b) = (self.show(l), self.show(r));
            self.err(span, format!("operator `{op}` cannot be applied to `{a}` and `{b}`"));
            Ty::Unknown
        })
    }

    // ---- calls ----

    /// Checks arguments when the callee's signature is not known.
    fn args_unchecked(&mut self, args: &[Arg]) {
        // Choosing what the call means already looked at them.
        if self.pre_args.take().is_some() {
            return;
        }
        for a in args {
            let t = self.expr(&a.value);
            self.discard(&t, a.value.span);
            if a.mode == Mode::Mut {
                self.require_mut_place(&a.value);
            }
        }
    }

    fn apply(&mut self, sig: &Sig, args: &[Arg], span: Span) -> Ty {
        self.apply_with(sig, args, None, span)
    }

    /// Checks the arguments of a call. `tys` holds their types when the
    /// caller has already computed them.
    fn apply_with(&mut self, sig: &Sig, args: &[Arg], tys: Option<Vec<Ty>>, span: Span) -> Ty {
        let tys = match (tys, self.pre_args.take()) {
            (Some(tys), _) => Some(tys),
            (None, Some((tys, logs))) => {
                self.arg_logs = Some(logs);
                Some(tys)
            }
            (None, None) => None,
        };
        let required = sig.params.iter().filter(|p| !p.optional).count();
        if args.len() < required || args.len() > sig.params.len() {
            let expected = if required == sig.params.len() {
                required.to_string()
            } else {
                format!("{required} to {}", sig.params.len())
            };
            self.err(span, format!("expected {expected} arguments, found {}", args.len()));
        }
        // What each argument borrows for the duration of the call, starting
        // with the receiver of a method.
        let mut borrows: Vec<(Access, bool, Span)> = Vec::new();
        if let Some((a, exclusive)) = self.receiver.take() {
            borrows.push((a, exclusive, span));
        }
        let logs = self.arg_logs.take();
        for (i, a) in args.iter().enumerate() {
            let start = self.mut_log.len();
            let t = match &tys {
                Some(tys) => tys[i].clone(),
                None => {
                    // A call learns from the parameter what it must produce.
                    if let (ExprKind::Call { .. }, Some(p)) = (&a.value.kind, sig.params.get(i)) {
                        self.hint = Some(p.ty.clone()).filter(concrete);
                    }
                    self.expr(&a.value)
                }
            };
            let (start, end) = logs.as_ref().and_then(|l| l.get(i).copied()).unwrap_or((start, self.mut_log.len()));
            // An argument that modifies what an earlier one borrows.
            for k in start..end {
                let (m, at) = self.mut_log[k].clone();
                if borrows.iter().any(|(b, _, _)| overlap(b, &m)) {
                    self.err(at, format!("`{}` is modified here while another argument of the same call borrows it", m.name));
                }
            }
            let Some(p) = sig.params.get(i) else { continue };
            let borrow = match (p.mode, a.mode) {
                (Mode::Mut, Mode::Mut) => self.access(&a.value).map(|x| (x, true, a.value.span)),
                // Scalars are copied before the call: they borrow nothing.
                (Mode::Let, _) if !scalar(&t) => self.access(&a.value).map(|x| (x, false, a.value.span)),
                _ => None,
            };
            if let Some((x, exclusive, at)) = borrow {
                if borrows.iter().any(|(b, other, _)| (exclusive || *other) && overlap(b, &x)) {
                    self.err(at, format!("`{}` is accessed twice by this call and one access is `mut`: exclusivity requires they do not overlap", x.name));
                }
                borrows.push((x, exclusive, at));
            }
            if (p.mode, a.mode) == (Mode::Own, Mode::Own) {
                self.movable(&a.value);
            }
            if let Some(label) = &a.label {
                if !p.name.is_empty() && *label != p.name {
                    self.err(a.value.span, format!("argument {} is named `{}`, not `{label}`", i + 1, p.name));
                }
            }
            self.coerce(&a.value, &p.ty, &t, "argument");
            match (p.mode, a.mode) {
                (Mode::Mut, Mode::Mut) => {
                    self.require_mut_place(&a.value);
                    // The callee writes through the argument, so the types must be the same.
                    if assignable(&p.ty, &t) && !assignable(&t, &p.ty) {
                        let (x, y) = (self.show(&p.ty), self.show(&t));
                        self.err(a.value.span, format!("a `mut` argument must have exactly the type `{x}`, found `{y}`"));
                    }
                }
                (Mode::Mut, _) => {
                    let hint = root_name(&a.value).unwrap_or("value");
                    self.err(a.value.span, format!("this parameter is `mut`: write `mut {hint}` at the call"));
                }
                (Mode::Let, Mode::Mut) | (Mode::Own, Mode::Mut) => {
                    self.err(a.value.span, "argument is marked `mut` but the parameter is not `mut`")
                }
                (Mode::Let, Mode::Own) => {
                    self.err(a.value.span, "argument is marked `own` but the parameter only borrows")
                }
                _ => {}
            }
        }
        sig.ret.clone()
    }

    /// Checks that an `own` argument is a variable that can be moved.
    fn movable(&mut self, e: &Expr) {
        let ExprKind::Path(name) = &e.kind else {
            self.err(e.span, "`own` moves a whole variable: this expression is not one");
            return;
        };
        let Some(v) = self.lookup(name) else { return };
        if self.captured(name) {
            self.err(e.span, format!("cannot move `{name}`: a closure captures it by value"));
        } else if !v.movable {
            self.err(e.span, format!("cannot move `{name}`: it is borrowed (take the parameter as `own`)"));
        } else if self.frozen.iter().any(|f| f.root == v.id) {
            self.err(e.span, format!("cannot move `{name}` while a `for` iterates over it"));
        }
    }

    fn ctor(fields: impl Iterator<Item = (String, Ty, bool)>, ret: Ty) -> Sig {
        let params = fields.map(|(name, ty, optional)| ParamSig { name, mode: Mode::Own, ty, optional }).collect();
        Sig { params, ret }
    }

    fn call(&mut self, e: &Expr, callee: &Expr, args: &[Arg], span: Span) -> Ty {
        let hint = self.pending.take();
        let target = self.gref(callee);
        if target.is_none() && self.in_module(callee) {
            // The name is missing or private: already reported.
            self.args_unchecked(args);
            return Ty::Unknown;
        }
        if let Some((m, "info")) = target {
            if self.mods.get(m).is_some_and(|n| n == "std.meta") && matches!(callee.kind, ExprKind::Field { .. }) {
                return self.meta_info(e, m, args, span);
            }
        }
        if let Some((m, name)) = target {
                if name == "panic" && self.global_in(m, name).is_none() && matches!(callee.kind, ExprKind::Path(_)) {
                    let message = ParamSig { name: "message".into(), mode: Mode::Let, ty: Ty::Str, optional: false };
                    self.res.insert((self.inst, id(e)), Res::Builtin("panic"));
                    self.apply(&Sig { params: vec![message], ret: Ty::Never }, args, span);
                    return Ty::Never;
                }
                if name == "assert" && self.global_in(m, name).is_none() && matches!(callee.kind, ExprKind::Path(_)) {
                    let condition = ParamSig { name: "condition".into(), mode: Mode::Let, ty: Ty::Bool, optional: false };
                    let message = ParamSig { name: "message".into(), mode: Mode::Let, ty: Ty::Str, optional: true };
                    self.res.insert((self.inst, id(e)), Res::Builtin("assert"));
                    return self.apply(&Sig { params: vec![condition, message], ret: Ty::Unit }, args, span);
                }
                if name == "Some" && m == self.module && matches!(callee.kind, ExprKind::Path(_)) {
                    if args.len() != 1 {
                        self.err(span, format!("`Some` takes 1 argument, found {}", args.len()));
                    }
                    let inner = args.first().map_or(Ty::Unknown, |a| self.expr(&a.value));
                    self.res.insert((self.inst, id(e)), Res::SomeCtor);
                    return Ty::Optional(Box::new(inner));
                }
                match self.global_in(m, name) {
                    Some(Global::Fn(i)) if self.fns[i].generic => {
                        self.args_unchecked(args);
                        return self.skip();
                    }
                    Some(Global::Fn(i)) if !self.fns[i].tparams.is_empty() => {
                        return self.generic_call(e, Target::Fn(i), HashMap::new(), None, hint, None, args, span);
                    }
                    Some(Global::Fn(i)) => {
                        let sig = self.fns[i].sig.clone();
                        self.res.insert((self.inst, id(e)), Res::Fn(i));
                        return self.apply(&sig, args, span);
                    }
                    Some(Global::Struct(i)) if self.structs[i].generic => {
                        self.args_unchecked(args);
                        return self.skip();
                    }
                    Some(Global::Struct(i)) => return self.construct_struct(e, i, None, hint, args, span),
                    _ => {}
                }
        }
        match &callee.kind {
            ExprKind::Index { base, args: targs } => {
                if let Some((m, name)) = self.gref(base) {
                    {
                        match self.global_in(m, name) {
                            Some(Global::Fn(i)) if !self.fns[i].generic && !self.fns[i].tparams.is_empty() => {
                                return self.generic_call(e, Target::Fn(i), HashMap::new(), Some(targs), None, None, args, span);
                            }
                            Some(Global::Struct(i)) if !self.structs[i].generic => {
                                if let Some((Ty::Struct(_, targs), _)) = self.type_head(callee) {
                                    return self.construct_struct(e, i, Some(targs), None, args, span);
                                }
                            }
                            // `Chan[T](n)`: a channel that holds up to `n` values.
                            None if name == "Chan" && targs.len() == 1 && matches!(base.kind, ExprKind::Path(_)) => {
                                let item = self.type_of_expr(&targs[0]);
                                self.res.insert((self.inst, id(e)), Res::Builtin("chan.new"));
                                let ret = Ty::Chan(Box::new(item));
                                return self.apply(&Sig { params: vec![plain_param("capacity", Ty::Int("usize"))], ret }, args, span);
                            }
                            // `Mutex[T](v)`: a value tasks share, starting as `v`.
                            None if name == "Mutex" && targs.len() == 1 && matches!(base.kind, ExprKind::Path(_)) => {
                                let item = self.type_of_expr(&targs[0]);
                                self.res.insert((self.inst, id(e)), Res::Builtin("mutex.new"));
                                let ret = Ty::Mutex(Box::new(item.clone()));
                                return self.apply(&Sig { params: vec![plain_param("value", item)], ret }, args, span);
                            }
                            _ => {}
                        }
                    }
                }
            }
            ExprKind::Field { base, name } => {
                if let ExprKind::Path(b) = &base.kind {
                    if self.lookup(b).is_none() {
                        if let Some(&x) = self.extern_ids.get(&(self.module, b.clone(), name.clone())) {
                            if self.unsafe_depth == 0 {
                                self.err(span, format!("`{b}.{name}` is a C function, which the compiler cannot check: call it inside `unsafe {{ }}`"));
                            }
                            self.res.insert((self.inst, id(e)), Res::Extern(x));
                            let sig = self.externs[x].sig.clone();
                            let ret = self.apply(&sig, args, span);
                            // What C is given to call is a function, by its name.
                            for (a, p) in args.iter().zip(&sig.params) {
                                let named = matches!(self.res.get(&(self.inst, id(&a.value))), Some(Res::Fn(_)));
                                if matches!(p.ty, Ty::Fn(_)) && !named {
                                    self.err(a.value.span, "a C function is given a function by its name: a closure or a variable cannot go to C");
                                }
                            }
                            return ret;
                        }
                        let module = self.imports.get(&(self.module, b.clone())).cloned();
                        if let Some((native, sig)) = module.and_then(|m| native(&m, name)) {
                            self.res.insert((self.inst, id(e)), Res::Builtin(native));
                            return self.apply(&sig, args, span);
                        }
                        let skipped = match self.global(b) {
                            Some(Global::Import) => true,
                            Some(Global::Enum(i)) => self.enums[i].generic,
                            Some(Global::Struct(i)) => self.structs[i].generic,
                            _ => false,
                        };
                        if skipped {
                            self.args_unchecked(args);
                            return self.skip();
                        }
                    }
                }
                // A function of a type parameter: `T.from(x)`.
                if let ExprKind::Path(b) = &base.kind {
                    if self.lookup(b).is_none() {
                        match self.tenv.get(b).cloned() {
                            Some(Ty::Param(n)) => {
                                let traits: Vec<Bound> =
                                    self.bounds.get(&n).cloned().unwrap_or_default().iter().flat_map(|b| with_supers(&self.traits, b)).collect();
                                let found = traits.iter().find_map(|b| {
                                    self.traits[b.0].methods.iter().find(|m| m.0 == *name && m.2.is_none()).cloned().map(|m| (b.clone(), m))
                                });
                                return match found {
                                    Some((bound, (_, sig, _))) => {
                                        let mut env = trait_env(&self.traits, &bound);
                                        env.insert("Self".to_string(), Ty::Param(n));
                                        self.apply(&subst_sig(&sig, &env), args, span)
                                    }
                                    None => {
                                        self.args_unchecked(args);
                                        self.err(span, format!("no bound of `{n}` has a function `{name}` without `self`"));
                                        Ty::Unknown
                                    }
                                };
                            }
                            Some(ty) if ty != Ty::Unknown => return self.type_call(e, ty, true, name, hint, args, span),
                            _ => {}
                        }
                    }
                }
                if let Some((ty, explicit)) = self.type_head(base) {
                    return self.type_call(e, ty, explicit, name, hint, args, span);
                }
                let bt = self.expr(base);
                if let Ty::Dyn(t, targs) = &bt {
                    let bound = (*t, targs.clone());
                    let slots = dyn_slots(&self.traits, &bound);
                    let tname = self.show_bound(&bound);
                    return match slots.iter().position(|m| m.0 == *name) {
                        Some(k) => {
                            let is_mut = slots[k].2 == Some(Mode::Mut);
                            if is_mut {
                                self.require_mut_place(base);
                            }
                            self.res.insert((self.inst, id(e)), Res::Dyn(k, is_mut));
                            self.apply(&slots[k].1, args, span)
                        }
                        None => {
                            self.args_unchecked(args);
                            let declared = with_supers(&self.traits, &bound).iter().any(|b| self.traits[b.0].methods.iter().any(|m| m.0 == *name));
                            if declared {
                                self.err(span, format!("`{name}` cannot be called on a `dyn {tname}`: it has no `self`, names `Self` or has type parameters, so it needs the real type"));
                            } else {
                                self.err(span, format!("`dyn {tname}` has no method `{name}`"));
                            }
                            Ty::Unknown
                        }
                    };
                }
                let mut key = (self.key(&concretize(&bt)), name.clone());
                // A method several impls of a generic trait give the type.
                if !self.methods.contains_key(&key) {
                    match self.pick(&key.0, name, &hint, args, span) {
                        Ok(Some(chosen)) => key.1 = chosen,
                        Ok(None) => {}
                        Err(()) => {
                            self.args_unchecked(args);
                            return Ty::Unknown;
                        }
                    }
                }
                if let Some(m) = self.methods.get(&key).cloned() {
                    self.method_visible(&m, name, span);
                    let receiver = match m.self_mode {
                        Some(Mode::Mut) => self.access(base).map(|a| (a, true)),
                        Some(Mode::Let) if !scalar(&bt) => self.access(base).map(|a| (a, false)),
                        _ => None,
                    };
                    match m.self_mode {
                        Some(Mode::Mut) => self.require_mut_place(base),
                        Some(_) => {}
                        None => self.err(span, format!("`{name}` has no `self`: call it as `{}.{name}(...)`", key.0)),
                    }
                    if m.tparams.is_empty() && m.default.is_none() {
                        self.res.insert((self.inst, id(e)), Res::Method(key.0.clone(), key.1.clone()));
                        self.receiver = receiver;
                        return self.apply(&m.sig, args, span);
                    }
                    let mut env = HashMap::new();
                    bind(&m.self_ty, &concretize(&bt), &mut env);
                    return self.generic_call(e, Target::Method(key.0, key.1), env, None, hint, receiver, args, span);
                }
                if let Ty::Param(n) = &bt {
                    let traits: Vec<Bound> =
                        self.bounds.get(n).cloned().unwrap_or_default().iter().flat_map(|b| with_supers(&self.traits, b)).collect();
                    let mut all: Vec<_> = traits
                        .iter()
                        .filter_map(|b| self.traits[b.0].methods.iter().find(|m| m.0 == *name).cloned().map(|m| (b.clone(), m)))
                        .collect();
                    // Of several bounds with the method, the one that
                    // produces what the call must.
                    if let (true, Some(h)) = (all.len() > 1, &hint) {
                        let fits = all.iter().position(|(b, m)| assignable(h, &subst(&m.1.ret, &trait_env(&self.traits, b))));
                        if let Some(k) = fits {
                            all.swap(0, k);
                        }
                    }
                    let found = all.into_iter().next();
                    return match found {
                        Some((bound, (_, sig, self_mode))) => {
                            if self_mode == Some(Mode::Mut) {
                                self.require_mut_place(base);
                            }
                            let mut env = trait_env(&self.traits, &bound);
                            env.insert("Self".to_string(), bt.clone());
                            if let Some(r) = self.traits[bound.0].refs.iter().find(|r| r.0 == *name) {
                                self.ref_calls.insert((self.inst, id(e)), (r.1, self_mode.is_some()));
                            }
                            let own = self.traits[bound.0].own.iter().find(|o| o.0 == *name).map(|o| o.1.clone()).unwrap_or_default();
                            if own.is_empty() {
                                return self.apply(&subst_sig(&sig, &env), args, span);
                            }
                            // The method's own type parameters follow from
                            // the arguments and from what the call must
                            // produce. They get names nothing else has.
                            let fresh = |n: &str| format!("{n}'");
                            for (n, _) in &own {
                                env.insert(n.clone(), Ty::Param(fresh(n)));
                            }
                            let open_sig = subst_sig(&sig, &env);
                            let (tys, logs) = self.arg_types(args);
                            let mut found = HashMap::new();
                            for (p, t) in open_sig.params.iter().zip(&tys) {
                                bind(&p.ty, t, &mut found);
                            }
                            if let Some(h) = &hint {
                                hinted(&open_sig.ret, h, &mut found);
                            }
                            let mut decided = HashMap::new();
                            for (n, _) in &own {
                                match found.get(&fresh(n)) {
                                    Some(t) => {
                                        decided.insert(fresh(n), concretize(t));
                                    }
                                    None => {
                                        self.err(span, format!("cannot infer the type parameter `{n}`: give the result a type"));
                                        decided.insert(fresh(n), Ty::Unknown);
                                    }
                                }
                            }
                            for (n, bs) in &own {
                                let t = decided[&fresh(n)].clone();
                                for b in bs {
                                    let b: Bound = (b.0, b.1.iter().map(|a| subst(&subst(a, &env), &decided)).collect());
                                    if !self.satisfies(&t, &b) {
                                        let (shown, tname) = (self.show(&t), self.show_bound(&b));
                                        self.err(span, format!("type `{shown}` does not implement `{tname}`, which `{n}` requires"));
                                    }
                                }
                            }
                            self.arg_logs = Some(logs);
                            self.apply_with(&subst_sig(&open_sig, &decided), args, Some(tys), span)
                        }
                        None => {
                            self.args_unchecked(args);
                            self.err(span, format!("type `{n}` has no method `{name}`: add a trait bound that provides it"));
                            Ty::Unknown
                        }
                    };
                }
                return match &bt {
                    Ty::Struct(i, targs) => {
                        let env = targ_env(&self.structs[*i].tparams, targs);
                        let field = self.structs[*i].fields.iter().find(|f| f.name == *name).map(|f| subst(&f.ty, &env));
                        match field {
                            Some(Ty::Fn(sig)) => self.apply(&sig, args, span),
                            _ => {
                                self.args_unchecked(args);
                                self.err(span, format!("type `{}` has no method `{name}`", key.0));
                                Ty::Unknown
                            }
                        }
                    }
                    Ty::Enum(..) => {
                        self.args_unchecked(args);
                        self.err(span, format!("type `{}` has no method `{name}`", key.0));
                        Ty::Unknown
                    }
                    Ty::Unknown => {
                        self.args_unchecked(args);
                        Ty::Unknown
                    }
                    Ty::Slice(_) | Ty::Array(_) if matches!(name.as_str(), "push" | "pop" | "get") => {
                        self.array_method(e, base, &bt, name, args, span)
                    }
                    Ty::Builtin("scope") if name == "spawn" => self.spawn(e, args, span),
                    Ty::Task(_) if name == "cancel" => {
                        self.res.insert((self.inst, id(e)), Res::Builtin("task.cancel"));
                        self.apply(&Sig { params: Vec::new(), ret: Ty::Unit }, args, span)
                    }
                    Ty::Task(t) if name == "wait" => {
                        self.res.insert((self.inst, id(e)), Res::Builtin("task.wait"));
                        self.apply(&Sig { params: Vec::new(), ret: (**t).clone() }, args, span)
                    }
                    Ty::Chan(t) if matches!(name.as_str(), "send" | "recv" | "close") => {
                        let item = (**t).clone();
                        let (native, sig) = match name.as_str() {
                            "send" => ("chan.send", Sig { params: vec![plain_param("value", item)], ret: Ty::Unit }),
                            "recv" => ("chan.recv", Sig { params: Vec::new(), ret: Ty::Optional(Box::new(item)) }),
                            _ => ("chan.close", Sig { params: Vec::new(), ret: Ty::Unit }),
                        };
                        self.res.insert((self.inst, id(e)), Res::Builtin(native));
                        self.apply(&sig, args, span)
                    }
                    Ty::Mutex(t) if matches!(name.as_str(), "get" | "set" | "update") => {
                        let item = (**t).clone();
                        let (native, sig) = match name.as_str() {
                            "get" => ("mutex.get", Sig { params: Vec::new(), ret: item }),
                            "set" => ("mutex.set", Sig { params: vec![plain_param("value", item)], ret: Ty::Unit }),
                            _ => {
                                let change = Ty::Fn(Box::new(Sig { params: vec![plain_param("", item.clone())], ret: item }));
                                ("mutex.update", Sig { params: vec![plain_param("change", change)], ret: Ty::Unit })
                            }
                        };
                        self.res.insert((self.inst, id(e)), Res::Builtin(native));
                        self.apply(&sig, args, span)
                    }
                    Ty::Builtin("scope") | Ty::Task(_) | Ty::Chan(_) | Ty::Mutex(_) => {
                        self.args_unchecked(args);
                        self.err(span, format!("type `{}` has no method `{name}`", self.show(&bt)));
                        Ty::Unknown
                    }
                    // Other methods of built-in types are not defined yet.
                    _ => {
                        self.args_unchecked(args);
                        self.skip()
                    }
                };
            }
            _ => {}
        }
        match self.expr(callee) {
            Ty::Fn(sig) => self.apply(&sig, args, span),
            Ty::Unknown => {
                self.args_unchecked(args);
                Ty::Unknown
            }
            other => {
                self.args_unchecked(args);
                let shown = self.show(&other);
                self.err(span, format!("a value of type `{shown}` cannot be called"));
                Ty::Unknown
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_file;

    fn errors(src: &str) -> Vec<String> {
        let file = parse_file(src).unwrap_or_else(|e| panic!("parse error: {} at {:?}", e.msg, e.span));
        match check(&file) {
            Ok(_) => Vec::new(),
            Err(es) => es.into_iter().map(|e| e.msg).collect(),
        }
    }

    fn ok(src: &str) {
        let es = errors(src);
        assert!(es.is_empty(), "unexpected errors: {es:#?}");
    }

    fn bad(src: &str, needle: &str) {
        let es = errors(src);
        assert!(es.iter().any(|e| e.contains(needle)), "expected an error with {needle:?}, got {es:#?}");
    }

    #[test]
    fn examples_check() {
        ok(include_str!("../examples/shapes.mou"));
        ok(include_str!("../examples/core.mou"));
    }

    #[test]
    fn literals_and_arithmetic() {
        ok("fn f(a: f64) -> f64 { 2 * a + 1.5 }");
        ok("fn f() -> u8 { 255 }");
        bad("fn f() -> u8 { 256 }", "out of range");
        bad("fn f() -> i8 { -129 }", "out of range");
        // An integer converts by itself to a type that holds all its values.
        ok("fn f(a: i32, b: i64) -> i64 { a + b }");
        ok("fn f(a: u8) -> int {\n let n: int = a\n n + a\n}");
        ok("fn g(n: int) -> int { n }\nfn f(a: u16, b: i8) -> bool { g(a) < g(b) && a < 70000 }");
        bad("fn f(a: i64, b: u64) -> i64 { a + b }", "cannot be applied");
        bad("fn f(a: int) -> u8 { a }", "expected return type `u8`");
        bad("fn f(a: i8) -> u64 { a }", "expected return type `u64`");
        bad("fn f(a: usize) -> int { a }", "expected return type");
        ok("fn grow(xs: mut [int]) { xs.push(1) }\nfn f() {\n var xs = [1, 2]\n grow(mut xs)\n}");
        bad("fn grow(xs: mut [int]) { xs.push(1) }\nfn f() {\n let xs = [1, 2]\n grow(mut xs)\n}", "");
        bad("fn f(a: f64) -> f64 { a +% 1.0 }", "cannot be applied");
        bad("fn f(a: u32) -> u32 { -a }", "cannot be applied");
        ok("fn f(a: i32) -> i64 { a as i64 + 1 }");
        bad("fn f() -> int { \"a\" }", "expected return type `i64`, found `str`");
    }

    #[test]
    fn mutability_and_modes() {
        bad("fn f() {\n let x = 1\n x = 2\n}", "immutable");
        ok("fn f() -> int {\n var x = 1\n x += 2\n x\n}");
        let g = "fn g(p: mut int) { p += 1 }\n";
        ok(&format!("{g}fn f() {{\n var a = 1\n g(mut a)\n}}"));
        bad(&format!("{g}fn f() {{\n var a = 1\n g(a)\n}}"), "write `mut a`");
        bad(&format!("{g}fn f() {{\n let a = 1\n g(mut a)\n}}"), "not mutable");
        bad("fn g(p: int) {}\nfn f() {\n var a = 1\n g(mut a)\n}", "parameter is not `mut`");
        bad("fn g(p: int) { p = 2 }", "immutable");
    }

    #[test]
    fn structs_enums_and_match() {
        let decls = "struct P { x: int, y: int }\nenum E { A, B(n: int) }\n";
        ok(&format!("{decls}fn f(e: E) -> int {{ match e {{ A => 0, B(n) => n }} }}"));
        bad(&format!("{decls}fn f(e: E) -> int {{ match e {{ A => 0 }} }}"), "missing `B`");
        bad(&format!("{decls}fn f(e: E) -> int {{ match e {{ A => 0, B(1) => 1 }} }}"), "missing `B`");
        bad(&format!("{decls}fn f(e: E) -> int {{ match e {{ A => 0, B(n) => \"x\" }} }}"), "different types");
        bad(&format!("{decls}fn f(p: P) -> int {{ p.z }}"), "no field `z`");
        bad(&format!("{decls}fn f() -> P {{ P(1) }}"), "expected 2 arguments");
        bad(&format!("{decls}fn f() -> P {{ P(1, true) }}"), "expected `i64`, found `bool`");
        bad(&format!("{decls}fn f() -> E {{ E.C }}"), "no variant `C`");
        ok(&format!("{decls}fn f(p: P) -> int {{\n let P(a, b) = p\n a + b\n}}"));
        bad(&format!("{decls}fn f(e: E) {{\n let B(n) = e\n}}"), "may not match");
        bad("fn f(n: int) -> int { match n { 0 => 1 } }", "add a `_` arm");
        ok("fn f(b: bool) -> int { match b { true => 1, false => 0 } }");
    }

    #[test]
    fn optionals_and_errors() {
        ok("fn f(x: int?) -> int { if let Some(v) = x { v } else { 0 } }");
        ok("fn f(x: int) -> int? { if x > 0 { x } else { None } }");
        ok("fn f(x: int?) -> int { match x { Some(v) => v, None => 0 } }");
        bad("fn f(x: int?) -> int { match x { Some(v) => v } }", "missing `None`");
        bad("fn f(x: int?) -> int { x }", "expected return type");
        let g = "enum E { Bad }\nfn g(x: int) -> int ! E {\n if x < 0 { return E.Bad }\n x\n}\n";
        ok(&format!("{g}fn f() -> int ! E {{\n let v = g(1)?\n v + 1\n}}"));
        ok(&format!("{g}fn f() -> int {{ g(1) catch e {{ 0 }} }}"));
        bad(&format!("{g}fn f() -> int {{ g(1)? }}"), "declares an error");
        bad(&format!("{g}fn f() {{\n g(1)\n let x = 1\n}}"), "is ignored");
        bad(&format!("{g}fn f() -> int {{ g(1) }}"), "expected return type");
        bad(&format!("{g}fn f() -> int {{ g(1) catch e {{ \"no\" }} }}"), "handler must produce");
    }

    #[test]
    fn methods() {
        let p = "struct P { x: int }\nimpl P {\n fn new() -> P { P(0) }\n fn get(self) -> int { self.x }\n fn inc(mut self) { self.x += 1 }\n}\n";
        ok(&format!("{p}fn f() -> int {{\n var p = P.new()\n p.inc()\n p.get()\n}}"));
        bad(&format!("{p}fn f() {{\n let p = P.new()\n p.inc()\n}}"), "not mutable");
        bad(&format!("{p}fn f(p: P) {{ p.nope() }}"), "no method `nope`");
        bad(&format!("{p}impl P {{ fn bad(self) {{ self.x = 1 }} }}"), "immutable");
    }

    #[test]
    fn names_loops_and_control_flow() {
        bad("fn f() -> int { y }", "undefined name `y`");
        bad("fn f(x: Nope) {}", "unknown type `Nope`");
        bad("fn f() {}\nfn f() {}", "already defined");
        bad("fn f() { break }", "outside of a loop");
        bad("fn f(c: bool) -> int { if c { 1 } else { \"a\" } }", "different types");
        bad("fn f() { if 1 { } }", "condition: expected `bool`");
        ok("fn f(xs: [int]) -> int {\n var t = 0\n for x in xs { t += x }\n for i in 0..10 { t += i }\n t\n}");
        ok("fn f() -> int { loop { break 3 } }");
        bad("fn f() { while true { break 3 } }", "only `loop`");
        bad("fn f(x: int) { for a in x {} }", "cannot iterate");
        ok("fn f(xs: [int]) -> [int] { xs[1..] }");
        bad("fn f(xs: [int]) -> int { xs[true] }", "index must be");
        ok("let N: usize = 1 << 20\nfn f() -> usize { N }");
        bad("type A = A\nfn f(a: A) {}", "refers to itself");
    }

    #[test]
    fn unsupported_features_are_reported_not_accepted_silently() {
        let file = parse_file(include_str!("../examples/generics.mou")).unwrap();
        let report = check(&file).unwrap_or_else(|es| panic!("{:#?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        assert!(report.skipped_decls.iter().any(|d| d == "struct View"));
        assert!(report.skipped_exprs > 0);
    }

    #[test]
    fn default_methods_and_supertraits() {
        ok(include_str!("../tests/native/defaults.mou"));
        let d = "trait A {\n fn a(self) -> int\n fn twice(self) -> int { self.a() + self.a() }\n}\n\
                 trait B: A { fn b(self) -> int }\nstruct S { v: int }\n";
        let f = |body: &str| format!("{d}{body}");
        ok(&f("impl A for S { fn a(self) -> int { self.v } }\nfn f(s: S) -> int { s.twice() }"));
        ok(&f("impl A for S { fn a(self) -> int { 1 }\n fn twice(self) -> int { 7 } }\nimpl B for S { fn b(self) -> int { 2 } }"));
        ok(&f("fn need[X: A](x: X) -> int { x.twice() }\nfn pass[X: B](x: X) -> int { need(x) + x.a() + x.b() }"));
        bad(&f("impl B for S { fn b(self) -> int { 2 } }"), "`B` requires `A`, which `S` does not implement");
        bad(&f("impl A for S { }"), "is missing the method `a`");
        bad(&f("impl A for S { fn a(self) -> int { 1 } }\nfn f(s: S) -> str { s.twice() }"), "expected return type `str`");
        bad(&f("trait C { fn c(self) -> int { self.nope() } }"), "type `Self` has no method `nope`");
        bad(&f("trait C: C { fn c(self) -> int }"), "cannot require itself");
        bad(&f("trait C: int { fn c(self) -> int }"), "must be a trait");
        bad(&f("fn need[X: B](x: X) {}\nimpl A for S { fn a(self) -> int { 1 } }\nfn f(s: S) { need(s) }"), "does not implement `B`");
    }

    #[test]
    fn dyn_traits() {
        ok(include_str!("../tests/native/dyn.mou"));
        let d = "trait A {\n fn a(self) -> int\n fn same(self, o: Self) -> bool { true }\n fn make() -> int { 1 }\n fn set(mut self, v: int)\n}\n\
                 trait B { fn b(self) -> int }\nstruct S { v: int }\nstruct T { v: int }\n\
                 impl A for S {\n fn a(self) -> int { self.v }\n fn set(mut self, v: int) { self.v = v }\n}\n";
        let f = |body: &str| format!("{d}{body}");
        ok(&f("fn f() -> int {\n var d = S(v: 1) as dyn A\n d.set(2)\n let e: dyn A? = d\n d.a()\n}"));
        ok(&f("fn f[X: A](x: X) -> dyn A { x as dyn A }\nfn g() -> int { f(S(v: 1)).a() }"));
        bad(&f("fn f() { let e = T(v: 1) as dyn A }"), "`T` does not implement `A`");
        bad(&f("fn f(d: dyn A) { let x = d as dyn B }"), "only keeps the methods of its own trait");
        bad(&f("fn f() { let g: dyn A = S(v: 2) }"), "expected `dyn A`, found `S`");
        bad(&f("fn f(d: dyn A + B) {}"), "`dyn` takes one trait");
        bad(&f("fn f(d: dyn S) {}"), "must be followed by a trait");
        bad(&f("fn f(d: dyn A) { d.same(d) }"), "cannot be called on a `dyn A`");
        bad(&f("fn f(d: dyn A) { d.make() }"), "cannot be called on a `dyn A`");
        bad(&f("fn f(d: dyn A) { d.zzz() }"), "`dyn A` has no method `zzz`");
        bad(&f("fn f(d: dyn A) { d.set(3) }"), "mut");
        bad(&f("fn f(d: dyn A) -> int { d.a(1) }"), "argument");
    }

    #[test]
    fn ref_results() {
        ok(include_str!("../tests/native/refs.mou"));
        let d = "struct P { x: int, y: int }\nstruct B { ps: [P], o: P }\n\
                 impl B {\n fn at(mut self, i: int) -> ref mut P { self.ps[i] }\n fn ro(self, i: int) -> ref P { self.ps[i] }\n}\n\
                 trait Cells {\n fn cell(mut self, i: int) -> ref mut P\n fn fill(mut self, v: int) { self.cell(0).x = v }\n}\n\
                 fn two(a: mut P, b: P) {}\nfn mk() -> B { B([P(1, 2)], P(0, 0)) }\n";
        let f = |body: &str| format!("{d}{body}");
        ok(&f("fn f() -> int { var b = mk()\n b.at(0).x = 5\n b.at(0) = P(1, 1)\n b.at(0).y += 1\n two(mut b.at(0), P(0, 0))\n b.ro(0).x }"));
        ok(&f("fn f() -> int { let b = mk()\n let p = b.ro(0)\n p.x + b.ro(0).y }"));
        ok(&f("impl B { fn deep(mut self, i: int) -> ref mut int { if i < 0 { self.o.x } else { self.at(i).y } } }"));
        ok(&f("fn sec(p: mut P) -> ref mut int { p.y }\nfn g() { var p = P(1, 2)\n sec(mut p) = 3 }"));
        ok(&f("impl Cells for B { fn cell(mut self, i: int) -> ref mut P { self.ps[i] } }\nfn poke[T: Cells](t: mut T) { t.cell(0).x = 9 }"));
        // The result is part of `self`, or of the first parameter.
        bad(&f("impl P { fn a(self) -> ref mut int { self.x } }"), "take `self` as `mut`");
        bad(&f("fn a(p: P) -> ref mut int { p.x }"), "take `p` as `mut`");
        bad(&f("fn a() -> ref int { 1 }"), "has no parameter for the result to be part of");
        bad(&f("impl B { fn a(mut self) -> ref mut P { let q = self.o\n q } }"), "must be `self`, or a field or an item of it");
        bad(&f("impl B { fn a(mut self) -> ref mut P { P(1, 2) } }"), "must be `self`, or a field or an item of it");
        bad(&f("impl B { fn a(mut self, o: P) -> ref mut P { let ps = [o]\n ps[0] } }"), "must be `self`, or a field or an item of it");
        bad(&f("impl B { fn a(mut self) -> ref mut [P] { self.ps[0..1] } }"), "must be `self`, or a field or an item of it");
        bad(&f("impl B { fn a(mut self) -> ref mut int { self.ro(0).x } }"), "must be `self`, or a field or an item of it");
        bad(&f("fn a(p: mut P, q: mut P) -> ref mut int { q.x }"), "must be `p`, or a field or an item of it");
        bad(&f("impl B { fn a(mut self, c: bool) -> ref mut P { if c { self.o } else { P(0, 0) } } }"), "must be `self`");
        bad(&f("impl B { fn a(mut self) -> ref mut P { return self.o } }"), "not with `return`");
        bad(&f("enum E { Bad }\nimpl B { fn a(mut self) -> ref mut P ! E { self.o } }"), "cannot fail with `!`");
        // What can be written to.
        bad(&f("fn f() { var b = mk()\n b.ro(0).x = 5 }"), "cannot be assigned to");
        bad(&f("fn f() { mk().o = P(1, 1) }"), "cannot be assigned to");
        bad(&f("fn f() { let b = mk()\n b.at(0).x = 5 }"), "it is not mutable");
        bad(&f("fn f() { var b = mk()\n two(mut b.at(0), b.ro(0)) }"), "accessed twice");
        bad(&f("fn f() { var b = mk()\n for p in b.ps { b.at(0).x = 1 } }"), "while a `for` iterates over it");
        bad(&f("impl Cells for B { fn cell(mut self, i: int) -> P { self.ps[i] } }"), "does not have the signature");
        bad(&f("impl Cells for B { fn cell(mut self, i: int) -> ref mut P { self.ps[i] } }\nfn g(d: mut dyn Cells) { d.cell(0).x = 1 }"), "cannot be assigned to");
    }

    #[test]
    fn comptime_parameters() {
        let head = "let SIDE = 4\nfn power(comptime n: int, x: int) -> int {\n var r = 1\n for i in 0..n { r = r * x }\n r\n}\nfn small(comptime k: u8) -> int { k as int }\n";
        let f = |body: &str| format!("{head}fn f(k: int) -> int {{ {body} }}");
        ok(&f("power(3, k) + power(SIDE * 2 - 1, k) + small(100 + 100)"));
        ok(&format!("{head}fn twice(comptime n: int, x: int) -> int {{ power(n * 2, x) }}\nfn f() -> int {{ twice(2, 3) }}"));
        bad(&f("power(k, 2)"), "`n` is a `comptime` parameter: what it is given must be a number known when the program is compiled");
        bad(&f("power(f(1), 2)"), "must be a number known when the program is compiled");
        bad(&f("small(100 * 4)"), "400 does not fit in `u8`, the type of the `comptime` parameter `k`");
        bad("fn word(comptime s: str) -> str { s }", "`s` is a `comptime` parameter, and for now it has to be an integer or a `type`");
        // A type as an argument.
        let t = "fn filled(comptime T: type, comptime n: int, v: T) -> [T] {\n var out: [T] = []\n for i in 0..n { out.push(v) }\n out\n}\nfn empty(comptime T: type) -> [T] { [] }\n";
        ok(&format!("{t}fn f() -> int {{\n var xs = empty(int)\n xs.push(1)\n let ys = filled(str, 2, \"a\")\n xs[0] + ys.len as int\n}}"));
        bad(&format!("{t}fn f() {{ let a = filled(3, 1, 0) }}"), "expected a type");
        bad(&format!("{t}fn f() {{ let a = filled(int, 2, \"a\") }}"), "argument: expected `i64`, found `str`");
        bad(&format!("{t}fn f() {{ let a = empty(Nope) }}"), "unknown type `Nope`");
        bad("struct B { v: int }\nimpl B {\n fn up(self, comptime n: int) -> int { self.v + n }\n}", "a method cannot have `comptime` parameters yet");
    }

    #[test]
    fn c_functions() {
        let head = "@profile(system)\nimport c \"math.h\" as m {\n fn sqrt(x: f64) -> f64\n fn abs(n: i32) -> i32\n}\n";
        let f = |body: &str| format!("{head}fn f(x: f64) -> f64 {{ {body} }}");
        ok(&f("unsafe { m.sqrt(x) }"));
        ok(&f("unsafe { m.sqrt(4) + m.abs(-3) as f64 }"));
        bad(&f("m.sqrt(x)"), "is a C function, which the compiler cannot check: call it inside `unsafe { }`");
        bad(&f("unsafe { m.sqrt(x, x) }"), "expected 1 arguments, found 2");
        bad(&f("unsafe { m.sqrt(true) }"), "expected `f64`");
        bad("import c \"math.h\" as m { fn sqrt(x: f64) -> f64 }\nfn f() -> f64 { unsafe { m.sqrt(1.0) } }", "`unsafe` is only allowed in a module that starts with `@profile(system)`");
        bad("import c \"s.h\" as s { fn len(t: [str]) -> usize }", "a C function can only take and return numbers");
        bad("import c \"s.h\" as s { fn all() -> [int] }", "a C function can only take and return numbers");
        bad("import c \"s.h\" as s { fn f(n: mut i32) }", "a C function can only take and return numbers");
        bad("@c(\"a;b\")\nstruct S { a: i32 }", "`@c` takes the name of the type in C");
        bad("import c \"s.h\" as s { fn each(f: fn(str)) }", "a C function can only take and return numbers");
        bad("@c(\"s\")\nstruct S { a: str }", "a struct of C can only have fields");
        bad("import c \"s.h\" as s { fn one() -> int { 1 } }", "is a C function: it is declared with its parameters and its result");
    }

    #[test]
    fn generic_methods_of_traits() {
        ok(include_str!("../tests/native/generic_methods.mou"));
        ok(include_str!("../tests/native/multi_impl.mou"));
        ok(include_str!("../tests/native/dyn_generic.mou"));
        let d = "trait Show { fn show(self) -> str }\ntrait Box {\n fn get(self) -> int\n fn tag[U: Show](self, u: U) -> str\n fn conv[U](self, f: fn(int) -> U) -> U\n}\n\
                 struct A { v: int }\nimpl Show for int { fn show(self) -> str { \"i\" } }\n";
        let i = "impl Box for A {\n fn get(self) -> int { self.v }\n fn tag[V: Show](self, u: V) -> str { u.show() }\n fn conv[W](self, f: fn(int) -> W) -> W { f(self.v) }\n}\n";
        let f = |body: &str| format!("{d}{i}{body}");
        ok(&f("fn g[B: Box](b: B) -> str { b.tag(3) }\nfn h(a: A) -> int { a.conv(fn(n: int) -> int { n + g(a).len as int }) }"));
        ok(&f("fn g(a: A) -> int { let d = a as dyn Box\n d.get() }"));
        bad(&f("fn g[B: Box](b: B) -> str { b.tag(true) }"), "type `bool` does not implement `Show`, which `U` requires");
        bad(&f("fn g(a: A) -> str { a.tag(true) }"), "type `bool` does not implement `Show`, which `V` requires");
        bad(&f("fn g(a: A) -> str { let d = a as dyn Box\n d.tag(3) }"), "has type parameters");
        bad(&format!("{d}impl Box for A {{\n fn get(self) -> int {{ 1 }}\n fn tag[V](self, u: V) -> str {{ \"x\" }}\n fn conv[W](self, f: fn(int) -> W) -> W {{ f(1) }}\n}}"), "does not have the signature");
        bad(&format!("{d}impl Box for A {{\n fn get(self) -> int {{ 1 }}\n fn tag(self, u: int) -> str {{ \"x\" }}\n fn conv[W](self, f: fn(int) -> W) -> W {{ f(1) }}\n}}"), "does not have the signature");
        bad("trait Mk { fn mk[U](self) -> [U] }\nfn g[M: Mk](m: M) { let x = m.mk() }", "cannot infer the type parameter `U`");
        bad("trait Mk[T] { fn mk[T](self) -> [T] }", "already a parameter of the trait");
        // A function of a type parameter.
        ok("trait From[T] { fn from(v: T) -> Self }\nstruct M { v: int }\nimpl From[int] for M { fn from(v: int) -> Self { M(v) } }\n\
            impl From[str] for M { fn from(v: str) -> Self { M(0) } }\nfn mk[T: From[int]](n: int) -> T { T.from(n) }\n\
            fn g() -> int { let m: M = mk(3)\n M.from(\"a\").v + M.from(1).v + m.v }");
        bad("trait From[T] { fn from(v: T) -> Self }\nfn mk[T](n: int) -> T { T.from(n) }", "no bound of `T` has a function `from`");
    }

    #[test]
    fn generic_traits() {
        ok(include_str!("../tests/native/generic_traits.mou"));
        let d = "trait Into[T] { fn into(self) -> T }\ntrait Show { fn show(self) -> str }\nstruct S { v: int }\n\
                 impl Into[str] for S { fn into(self) -> str { \"s\" } }\nfn conv[A: Into[B], B](a: A) -> B { a.into() }\n\
                 fn text[A: Into[str]](a: A) -> str { a.into() }\n";
        let f = |body: &str| format!("{d}{body}");
        ok(&f("fn f() -> str { conv(S(v: 1)) }"));
        ok(&f("fn f() -> str { text(S(v: 1)) }"));
        ok(&f("fn g[X: Into[int]](x: X) -> int { x.into() + 1 }"));
        ok(&f("trait Both[T]: Into[T] { fn other(self) -> T { self.into() } }\nimpl Both[str] for S { }\nfn f() -> str { S(v: 1).other() }"));
        bad(&f("fn f() -> int { conv(S(v: 1)) }"), "expected return type `i64`, found `str`");
        bad(&f("impl Into[int] for int { fn into(self) -> int { self } }\nfn f() -> str { text(3) }"), "`i64` does not implement `Into[str]`");
        bad(&f("impl Into[str] for S { fn into(self) -> str { \"t\" } }"), "already implements `Into[str]`");
        bad(&f("struct W[T] { v: T }\nimpl[T] Into[T] for W[T] { fn into(self) -> T { self.v } }\nimpl[T] Into[str] for W[T] { fn into(self) -> str { \"w\" } }"), "without type parameters");
        // Several impls of one trait: the call says which by its type.
        let two = f("impl Into[int] for S { fn into(self) -> int { self.v } }\n");
        ok(&format!("{two}fn g(s: S) -> int {{ let n: int = s.into()\n let t: str = s.into()\n n + conv[S, int](s) + text(s).len as int }}"));
        ok(&format!("{two}fn g(s: S) -> str {{ let d = s as dyn Into[str]\n d.into() }}"));
        bad(&format!("{two}fn g(s: S) {{ let x = s.into() }}"), "could be the one of `Into[str]` or `Into[i64]`");
        bad(&format!("{two}fn g(s: S) -> f64 {{ conv(s) }}"), "does not implement");
        bad(&f("impl Into[int] for int { fn into(self) -> str { \"x\" } }"), "does not have the signature `Into[i64]` declares");
        bad(&f("impl Into for int { fn into(self) -> int { 1 } }"), "trait `Into` takes 1 type arguments, found 0");
        bad(&f("fn g[X: Show[int]](x: X) {}"), "trait `Show` takes 0 type arguments, found 1");
        bad(&f("fn g[X: Into[int]](x: X) -> str { x.into() }"), "expected return type `str`, found `i64`");
        bad(&f("trait Both[T]: Into[T] { }\nimpl Both[int] for int { }"), "`Both` requires `Into[i64]`");
        bad(&f("trait Both[T]: Into[T] { }\nimpl Both[int] for S { }"), "`Both` requires `Into[i64]`, which `S` does not implement");
    }

    #[test]
    fn generic_types() {
        ok(include_str!("../examples/generic_types.mou"));
        let d = "struct Box[T] { v: T }\nenum Opt[T] { Nothing, Just(T) }\ntrait Show { fn show(self) -> str }\n\
                 impl[T] Box[T] {\n fn get(self) -> T { self.v }\n fn put(mut self, v: T) { self.v = v }\n\
                 fn new(v: T) -> Box[T] { Box(v: v) }\n fn empty() -> Opt[T] { Opt.Nothing }\n}\n\
                 impl[T: Show] Show for Box[T] { fn show(self) -> str { self.v.show() } }\nfn need[T: Show](x: T) {}\n";
        let f = |body: &str| format!("{d}{body}");
        ok(&f("fn f() -> int { let b = Box(v: 1)\n b.get() }"));
        ok(&f("fn f() -> Box[u8] { Box.new(3) }"));
        ok(&f("fn f() {\n let o = Opt.Nothing\n let p: Opt[int] = o\n let e: Opt[int] = Box.empty()\n let g = Box[str].empty()\n}"));
        ok(&f("type Two[T] = (T, T)\nfn f(p: Two[int]) -> int { p.0 + p.1 }"));
        ok(&f("impl Show for int { fn show(self) -> str { \"i\" } }\nfn f() { need(Box(v: Box(v: 1))) }"));
        bad(&f("fn f(b: Box) {}"), "takes 1 type arguments, found 0");
        bad(&f("fn f(b: Box[int, int]) {}"), "takes 1 type arguments, found 2");
        bad(&f("fn f() { let b: Box[str] = Box(v: 1) }"), "expected `Box[str]`, found `Box[i64]`");
        bad(&f("fn f() { var o = Opt.Nothing }"), "is not fully known");
        bad(&f("fn f() {\n var b = Box(v: 1)\n b.put(\"a\")\n}"), "expected `i64`, found `str`");
        bad(&f("fn f() { need(Box(v: 1)) }"), "`Box[i64]` does not implement `Show`");
        bad(&f("fn f() { let e = Box.empty() }"), "cannot infer the type parameter `T`");
        bad(&f("fn f() { let h: Opt[int] = Box[str].empty() }"), "expected `Opt[i64]`, found `Opt[str]`");
        bad(&f("impl Box[int] { fn x(self) {} }"), "is not supported");
        bad(&f("fn f(o: Opt[int]) -> int { match o { Opt.Just(x) => x } }"), "missing `Nothing`");
        bad(&f("fn f(o: Opt[int]) -> Opt[str] { o }"), "expected return type `Opt[str]`");
        bad(&f("fn f(b: Box[int]) -> str { match b { Box(v) => v } }"), "expected return type `str`");
        bad(&f("struct S[T: Show] { v: T }"), "put them on the `impl`");
        bad(&f("fn g() -> Box[u8] { Box.new(300) }"), "out of range for `u8`");
        bad(&f("type Two[T] = (T, T)\nfn f(p: Two) {}"), "takes 1 type arguments");
    }

    #[test]
    fn exclusivity() {
        let d = "struct P { x: int, y: int }\nfn swap(a: mut int, b: mut int) {}\nfn add(p: mut P, q: P) {}\nfn addn(p: mut P, n: int) {}\n\
                 fn bump(n: mut int) -> int { n }\nimpl P { fn merge(mut self, o: P) {} }\n";
        let f = |body: &str| format!("{d}fn f() {{\n var p = P(1, 2)\n var xs = [1, 2, 3]\n var i = 0\n{body}\n}}");
        ok(&f("swap(mut p.x, mut p.y)\n swap(mut xs[0], mut xs[1])\n addn(mut p, p.y)\n for x in xs { p.x += x }"));
        bad(&f("swap(mut p.x, mut p.x)"), "accessed twice");
        bad(&f("add(mut p, p)"), "accessed twice");
        bad(&f("swap(mut xs[i], mut xs[0])"), "accessed twice");
        bad(&f("p.merge(p)"), "accessed twice");
        bad(&f("addn(mut p, bump(mut p.x))"), "modified here while another argument");
        bad(&f("for x in xs { xs[0] = x }"), "while a `for` iterates over it");
        bad(&f("for x in xs { swap(mut xs[0], mut i) }"), "while a `for` iterates over it");
        let m = "struct B { d: [int] }\nfn take(b: own B) {}\n";
        ok(&format!("{m}fn f(b: own B) {{ take(own b) }}"));
        bad(&format!("{m}fn f(b: B) {{ take(own b) }}"), "it is borrowed");
        bad(&format!("{m}fn f(b: mut B) {{ take(own b) }}"), "it is borrowed");
        bad(&format!("{m}struct W {{ b: B }}\nfn f(w: own W) {{ take(own w.b) }}"), "moves a whole variable");
    }

    #[test]
    fn generics_and_traits() {
        let t = "trait Ord { fn cmp(self, other: Self) -> int }\nstruct P { x: int }\nimpl Ord for P { fn cmp(self, other: Self) -> int { self.x - other.x } }\n\
                 fn max[T: Ord](a: T, b: T) -> T { if a.cmp(b) > 0 { a } else { b } }\n";
        ok(&format!("{t}fn f() -> P {{ max(P(1), P(2)) }}"));
        ok(&format!("{t}fn f() -> P {{ max[P](P(1), P(2)) }}"));
        bad(&format!("{t}fn f() -> f64 {{ max(1.5, 2.5) }}"), "does not implement `Ord`");
        bad(&format!("{t}fn f() -> P {{ max(P(1), 2) }}"), "expected `P`");
        bad(&format!("{t}fn g[T](a: T) -> int {{ a.cmp(a) }}"), "add a trait bound");
        bad(&format!("{t}fn g[T](a: T, b: T) -> T {{ a + b }}"), "cannot be applied to `T` and `T`");
        bad(&format!("{t}struct Q {{ x: int }}\nimpl Ord for Q {{ }}"), "missing the method `cmp`");
        bad(&format!("{t}struct Q {{ x: int }}\nimpl Ord for Q {{ fn cmp(self, other: int) -> int {{ 0 }} }}"), "does not have the signature");
        bad(&format!("{t}fn g[T: P](a: T) {{}}"), "must be a trait");
        bad("fn none[T]() -> T? { None }\nfn f() { none() }", "cannot infer");
        bad("fn f() {\n var n = 1\n let g = fn() { n = 2 }\n}", "captures it by value");
    }
}
