//! Mid-level IR: one control-flow graph per function, and the lowering from
//! the type-checked syntax tree.
//!
//! Every expression is flattened into assignments to locals; control flow
//! becomes basic blocks ending in a terminator. `mut` arguments are passed by
//! copy-in/copy-out: the law of exclusivity makes that indistinguishable from
//! passing a reference.

use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::*;
use crate::check::{assignable, id, int_value, join, Info, Res, Sig, Target, Ty};
use crate::lexer::{Error, Span};

pub type Local = usize;
pub type BlockId = usize;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i128),
    Float(f64),
    F32(f32),
    Bool(bool),
    Char(char),
    Str(Rc<str>),
    Unit,
    /// Tuples, and ranges as `(start?, end?, inclusive)`.
    Tuple(Vec<Value>),
    Struct(usize, Vec<Value>),
    Enum(usize, usize, Vec<Value>),
    Array(Rc<Vec<Value>>),
    None,
    Some(Box<Value>),
    Ok(Box<Value>),
    Err(Box<Value>),
    Fn(usize),
    /// A closure: its function and the values it captured.
    Closure(usize, Rc<Vec<Value>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Proj {
    /// Field of a struct, tuple or variant; payload of `Some`, `Ok` and `Err`.
    Field(usize),
    Index(Operand),
    /// Steps only known when the program runs, which a function that
    /// returns `ref mut` computed: the local holds an array of pairs, each
    /// `0` and a field or `1` and an index.
    Path(Local),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub local: Local,
    pub proj: Vec<Proj>,
}

impl Place {
    fn local(local: Local) -> Place {
        Place { local, proj: Vec::new() }
    }

    fn field(&self, k: usize) -> Place {
        let mut p = self.clone();
        p.proj.push(Proj::Field(k));
        p
    }

    fn index(&self, i: Operand) -> Place {
        let mut p = self.clone();
        p.proj.push(Proj::Index(i));
        p
    }

    fn path(&self, steps: Local) -> Place {
        let mut p = self.clone();
        p.proj.push(Proj::Path(steps));
        p
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Const(Value),
    Place(Place),
}

/// How an arithmetic operation treats its operands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    /// A sized integer, or `"lit"` for a literal that has no type yet.
    Int(&'static str),
    Float,
    /// Floats known to be `f64`.
    F64,
    /// Decided by the values: comparisons, equality, bit operations.
    Dyn,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    AddWrap,
    SubWrap,
    MulWrap,
    AddSat,
    SubSat,
    MulSat,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            AddWrap => "+%",
            SubWrap => "-%",
            MulWrap => "*%",
            AddSat => "+|",
            SubSat => "-|",
            MulSat => "*|",
            Shl => "<<",
            Shr => ">>",
            BitAnd => "&",
            BitOr => "|",
            BitXor => "^",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
        }
    }

    fn parse(op: &str) -> Option<BinOp> {
        use BinOp::*;
        [Add, Sub, Mul, Div, Rem, AddWrap, SubWrap, MulWrap, AddSat, SubSat, MulSat, Shl, Shr, BitAnd, BitOr, BitXor, Eq, Ne, Lt, Le, Gt, Ge]
            .into_iter()
            .find(|b| b.symbol() == op)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

/// A conversion between two types the checker considers compatible.
#[derive(Debug, Clone, PartialEq)]
pub enum Conv {
    /// An untyped integer becomes a sized one: checked at run time.
    Fit(&'static str),
    /// To `f32` when true, to `f64` otherwise.
    ToFloat(bool),
    /// `T` becomes `T?`.
    Wrap(Option<Box<Conv>>),
    /// Converts the value inside an optional.
    Inside(Box<Conv>),
    Elems(Box<Conv>),
    Tuple(Vec<Option<Conv>>),
    Res(Option<Box<Conv>>, Option<Box<Conv>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum FmtPart {
    Lit(String),
    Val(Operand),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rvalue {
    Use(Operand),
    Unary(UnOp, Num, Operand),
    Binary(BinOp, Num, Operand, Operand),
    /// Numeric cast to an integer kind or to a float (`f32` when true).
    CastInt(Operand, &'static str),
    CastFloat(Operand, bool),
    Convert(Operand, Conv),
    Tuple(Vec<Operand>),
    Struct(usize, Vec<Operand>),
    Enum(usize, usize, Vec<Operand>),
    Array(Vec<Operand>),
    Repeat(Operand, Operand),
    Some(Operand),
    Ok(Operand),
    Err(Operand),
    /// Whether an enum, optional or result holds the given variant
    /// (`None`/`Ok` are 0, `Some`/`Err` are 1).
    Is(Operand, usize),
    Len(Operand),
    /// `base[range]`.
    Slice(Operand, Operand),
    Format(Vec<FmtPart>),
    /// The characters of a string, as an array.
    Chars(Operand),
    /// Value of a global constant.
    Const(usize),
    /// First value of a range; panics when it has no start.
    RangeStart(Operand, Num),
    /// Whether a value is still inside the range's upper bound.
    RangeHas(Operand, Operand),
    /// Whether a value is the inclusive end of the range.
    RangeLast(Operand, Operand),
    /// The next integer or character.
    Succ(Operand, Num),
    /// Appends an item to the array at a place, without copying the array
    /// when nothing else shares it. Produces unit.
    Push(Place, Operand),
    /// Removes the last item of the array at a place: an optional.
    Pop(Place),
    /// `base.get(i)`: the item, or `None` when the index is out of range.
    Get(Operand, Operand),
    /// A closure value: its function and the values it captures.
    Closure(usize, Vec<Operand>),
    /// A value captured by the closure being run.
    Capture(usize),
    /// Marks a variable declared without a value. Checked by `own`, not run.
    Uninit,
    /// Marks a variable whose value was moved away with `own`.
    Moved,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Callee {
    Fn(usize),
    Indirect(Operand),
    /// `io.print`.
    Print,
    /// Another function of a built-in module, by its name: `io.read_line`.
    Native(&'static str),
    /// `panic(message)`: stops the program.
    Panic,
    /// A function of a C header: index in `Program::externs`.
    Extern(usize),
}

/// A function of a C header. The types are names of the language: `i32`,
/// `f64`, `bool`, and `unit` for a function that returns nothing.
#[derive(Debug)]
pub struct Extern {
    pub name: String,
    pub header: String,
    pub params: Vec<&'static str>,
    pub ret: &'static str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Term {
    Goto(BlockId),
    Branch(Operand, BlockId, BlockId),
    Call {
        callee: Callee,
        args: Vec<Operand>,
        /// Parameters whose final value is copied back to a place of the caller.
        writeback: Vec<(usize, Place)>,
        dest: Place,
        next: BlockId,
    },
    Return(Operand),
    Panic(String),
    Unreachable,
}

#[derive(Debug)]
pub struct Stmt {
    pub place: Place,
    pub rv: Rvalue,
    pub span: Span,
}

#[derive(Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub term: Term,
    pub span: Span,
    done: bool,
}

#[derive(Debug)]
pub struct Func {
    pub name: String,
    /// The parameters are the first locals.
    pub params: usize,
    pub locals: usize,
    pub blocks: Vec<Block>,
    /// Name of each local that is a variable of the program.
    pub names: Vec<Option<String>>,
    /// Immutable variables declared without a value: assigned exactly once.
    pub once: Vec<Local>,
    /// For each parameter, whether its type says it is an integer or a bool.
    pub scalars: Vec<Option<Scalar>>,
    /// For each local, the same when it is a variable of such a type.
    pub hints: Vec<Option<Scalar>>,
    /// The same for the value the function returns.
    pub ret: Option<Scalar>,
    /// Where the function is declared.
    pub span: Span,
    /// The effects its declaration rules out: `NOALLOC`, `NOPANIC`, `NOIO`.
    pub limits: u8,
    /// The block where its body starts, after its `where` conditions.
    pub body: BlockId,
    /// Where it copies a value that holds an array into another place.
    pub copies: Vec<Span>,
}

pub const NOALLOC: u8 = 1;
pub const NOPANIC: u8 = 2;
pub const NOIO: u8 = 4;
pub const NOUNSAFE: u8 = 16;
pub const NOBLOCK: u8 = 32;
/// With `NOALLOC`: it comes from the profile of the module, not from an annotation.
pub const SYSTEM: u8 = 8;

/// A type the C backend can keep in a machine register.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scalar {
    Int(&'static str),
    Bool,
    F64,
    /// A struct whose fields are all booleans, integers of up to 64 bits
    /// or `f64`: it is kept as a struct of C, by its number.
    Rec(usize),
    /// An optional boolean (1), integer with sign of up to 64 bits (2),
    /// `f64` (3) or integer without sign of up to 64 bits (4): it is kept
    /// as whether it is there and the value.
    Opt(u8),
}

/// Whether a field of that type can be a member of a struct of C.
fn plain_field(t: &Ty) -> bool {
    matches!(t, Ty::Bool | Ty::Int("i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize") | Ty::Float("f64"))
}

fn scalar_of(info: &Info, t: &Ty) -> Option<Scalar> {
    match t {
        Ty::Int(k) => Some(Scalar::Int(k)),
        Ty::Bool => Some(Scalar::Bool),
        Ty::Float("f64") => Some(Scalar::F64),
        Ty::Optional(inner) => match &**inner {
            Ty::Bool => Some(Scalar::Opt(1)),
            Ty::Float("f64") => Some(Scalar::Opt(3)),
            Ty::Int(k) if !k.ends_with("128") => Some(Scalar::Opt(if k.starts_with('u') { 4 } else { 2 })),
            _ => None,
        },
        Ty::Struct(i, args) if args.is_empty() => {
            let s = &info.structs[*i];
            let plain = !s.fields.is_empty() && s.tparams.is_empty() && !s.generic && s.fields.iter().all(|f| plain_field(&f.ty));
            plain.then_some(Scalar::Rec(*i))
        }
        _ => None,
    }
}

#[derive(Debug)]
pub struct Program {
    pub fns: Vec<Func>,
    /// Functions that compute the global constants, in declaration order.
    pub consts: Vec<usize>,
    /// The value of each constant, once it was computed while compiling.
    pub values: Vec<Option<Value>>,
    pub main: Option<usize>,
    /// The tests of the program's own file: its functions without
    /// parameters whose name starts with `test_`.
    pub tests: Vec<usize>,
    pub structs: Vec<(String, Vec<String>)>,
    /// For each struct: how each field is kept when an array holds the
    /// struct in line (1 bool, 2 integer, 3 `f64`, 4 char), or nothing
    /// when a field is of another kind.
    pub records: Vec<Vec<u8>>,
    pub enums: Vec<(String, Vec<(String, Vec<Option<String>>)>)>,
    /// For each struct and for each enum, whether a value of it lives on
    /// the heap: it is `dyn`, or its type holds itself.
    pub boxed: (Vec<bool>, Vec<bool>),
    /// The structs from this one on are `dyn` values.
    pub dyn_from: usize,
    pub externs: Vec<Extern>,
    /// The structs that are a type of a C header: the struct, the name of
    /// that type, and the type of each field as C takes it.
    pub cstructs: Vec<(usize, String, Vec<&'static str>)>,
}

/// Whether a value of type `t` holds, without an array in between, a value
/// of the struct or enum `target`.
fn holds(info: &Info, t: &Ty, target: (bool, usize), seen: &mut Vec<(bool, usize)>) -> bool {
    match t {
        Ty::Struct(i, args) | Ty::Enum(i, args) => {
            let me = (matches!(t, Ty::Enum(..)), *i);
            if me == target {
                return true;
            }
            if seen.contains(&me) {
                return false;
            }
            seen.push(me);
            let mut inner: Vec<&Ty> = args.iter().collect();
            if me.0 {
                inner.extend(info.enums.get(*i).into_iter().flat_map(|e| e.variants.iter().flat_map(|v| v.fields.iter().map(|(_, t)| t))));
            } else {
                inner.extend(info.structs.get(*i).into_iter().flat_map(|s| s.fields.iter().map(|f| &f.ty)));
            }
            inner.into_iter().any(|t| holds(info, t, target, seen))
        }
        Ty::Optional(t) | Ty::Array(t) => holds(info, t, target, seen),
        Ty::Tuple(ts) => ts.iter().any(|t| holds(info, t, target, seen)),
        Ty::Result(a, b) => holds(info, a, target, seen) || holds(info, b, target, seen),
        _ => false,
    }
}

/// A lowered program as text, in the form the lowering written in Mou
/// (`compiler/mir.mou`) prints it.
pub fn dump(p: &Program) -> String {
    use std::fmt::Write;
    fn quote(s: &str) -> String {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
    }
    fn value(v: &Value) -> String {
        match v {
            Value::Int(i) => format!("int {i}"),
            Value::Float(f) => format!("float {f:?}"),
            Value::F32(f) => format!("f32 {f:?}"),
            Value::Bool(b) => format!("bool {b}"),
            Value::Char(c) => format!("char {}", *c as u32),
            Value::Str(s) => format!("str {}", quote(s)),
            Value::Unit => "()".to_string(),
            Value::None => "none".to_string(),
            Value::Fn(f) => format!("fn {f}"),
            Value::Enum(e, v, fields) if fields.is_empty() => format!("enum {e}.{v}"),
            other => format!("{other:?}"),
        }
    }
    fn place(p: &Place) -> String {
        let mut out = format!("L{}", p.local);
        for pr in &p.proj {
            match pr {
                Proj::Field(k) => out.push_str(&format!(".{k}")),
                Proj::Index(i) => out.push_str(&format!("[{}]", operand(i))),
                Proj::Path(l) => out.push_str(&format!("@{l}")),
            }
        }
        out
    }
    fn operand(op: &Operand) -> String {
        match op {
            Operand::Const(v) => value(v),
            Operand::Place(p) => place(p),
        }
    }
    fn operands(ops: &[Operand]) -> String {
        ops.iter().map(operand).collect::<Vec<_>>().join(", ")
    }
    fn num(n: Num) -> String {
        match n {
            Num::Int(k) => k.to_string(),
            Num::Float => "float".to_string(),
            Num::F64 => "f64".to_string(),
            Num::Dyn => "dyn".to_string(),
        }
    }
    fn conv(c: &Conv) -> String {
        let opt = |c: &Option<Box<Conv>>| c.as_ref().map_or("_".to_string(), |c| conv(c));
        match c {
            Conv::Fit(k) => format!("fit {k}"),
            Conv::ToFloat(single) => format!("tofloat {single}"),
            Conv::Wrap(c) => format!("wrap({})", opt(c)),
            Conv::Inside(c) => format!("inside({})", conv(c)),
            Conv::Elems(c) => format!("elems({})", conv(c)),
            Conv::Tuple(cs) => format!("tuple({})", cs.iter().map(|c| c.as_ref().map_or("_".to_string(), conv)).collect::<Vec<_>>().join(", ")),
            Conv::Res(a, b) => format!("res({}, {})", opt(a), opt(b)),
        }
    }
    fn rvalue(rv: &Rvalue) -> String {
        use Rvalue::*;
        match rv {
            Use(a) => format!("use {}", operand(a)),
            Unary(op, n, a) => {
                let op = match op {
                    UnOp::Neg => "neg",
                    UnOp::Not => "not",
                    UnOp::BitNot => "bitnot",
                };
                format!("{op} {} {}", num(*n), operand(a))
            }
            Binary(op, n, a, b) => format!("{} {} {}, {}", op.symbol(), num(*n), operand(a), operand(b)),
            CastInt(a, k) => format!("castint {k} {}", operand(a)),
            CastFloat(a, single) => format!("castfloat {single} {}", operand(a)),
            Convert(a, c) => format!("convert {} {}", conv(c), operand(a)),
            Tuple(ops) => format!("tuple({})", operands(ops)),
            Struct(i, ops) => format!("struct {i}({})", operands(ops)),
            Enum(i, v, ops) => format!("enum {i}.{v}({})", operands(ops)),
            Array(ops) => format!("array[{}]", operands(ops)),
            Repeat(a, b) => format!("repeat {}, {}", operand(a), operand(b)),
            Some(a) => format!("some {}", operand(a)),
            Ok(a) => format!("ok {}", operand(a)),
            Err(a) => format!("err {}", operand(a)),
            Is(a, v) => format!("is {v} {}", operand(a)),
            Len(a) => format!("len {}", operand(a)),
            Slice(a, b) => format!("slice {}, {}", operand(a), operand(b)),
            Format(parts) => {
                let parts: Vec<String> = parts
                    .iter()
                    .map(|part| match part {
                        FmtPart::Lit(s) => quote(s),
                        FmtPart::Val(o) => operand(o),
                    })
                    .collect();
                format!("format({})", parts.join(", "))
            }
            Chars(a) => format!("chars {}", operand(a)),
            Const(k) => format!("const {k}"),
            RangeStart(a, n) => format!("rangestart {} {}", num(*n), operand(a)),
            RangeHas(a, b) => format!("rangehas {}, {}", operand(a), operand(b)),
            RangeLast(a, b) => format!("rangelast {}, {}", operand(a), operand(b)),
            Succ(a, n) => format!("succ {} {}", num(*n), operand(a)),
            Push(p, a) => format!("push {}, {}", place(p), operand(a)),
            Pop(p) => format!("pop {}", place(p)),
            Get(a, b) => format!("get {}, {}", operand(a), operand(b)),
            Closure(f, ops) => format!("closure {f}({})", operands(ops)),
            Capture(k) => format!("capture {k}"),
            Uninit => "uninit".to_string(),
            Moved => "moved".to_string(),
        }
    }
    fn scalar(s: &Option<Scalar>) -> String {
        match s {
            Option::Some(Scalar::Int(k)) => k.to_string(),
            Option::Some(Scalar::Bool) => "bool".to_string(),
            Option::Some(Scalar::F64) => "f64".to_string(),
            Option::Some(Scalar::Rec(i)) => format!("rec{i}"),
            Option::Some(Scalar::Opt(k)) => format!("opt{k}"),
            Option::None => "_".to_string(),
        }
    }
    let mut out = String::new();
    for (i, f) in p.fns.iter().enumerate() {
        let _ = writeln!(out, "fn {i} {} params={} locals={} body={} limits={} ret={} at={}:{}", f.name, f.params, f.locals, f.body, f.limits, scalar(&f.ret), f.span.line, f.span.col);
        let names: Vec<String> = f.names.iter().enumerate().filter_map(|(l, n)| n.as_ref().map(|n| format!("{l}={n}"))).collect();
        let _ = writeln!(out, "  names {}", names.join(" "));
        let once: Vec<String> = f.once.iter().map(|l| l.to_string()).collect();
        let _ = writeln!(out, "  once {}", once.join(" "));
        let scalars: Vec<String> = f.scalars.iter().map(scalar).collect();
        let _ = writeln!(out, "  scalars {}", scalars.join(" "));
        let hints: Vec<String> = f.hints.iter().enumerate().filter(|(_, h)| h.is_some()).map(|(l, h)| format!("{l}={}", scalar(h))).collect();
        let _ = writeln!(out, "  hints {}", hints.join(" "));
        let copies: Vec<String> = f.copies.iter().map(|s| format!("{}:{}", s.line, s.col)).collect();
        let _ = writeln!(out, "  copies {}", copies.join(" "));
        for (k, b) in f.blocks.iter().enumerate() {
            let _ = writeln!(out, "  b{k} {}:{}", b.span.line, b.span.col);
            for st in &b.stmts {
                let _ = writeln!(out, "    {} = {} @{}:{}", place(&st.place), rvalue(&st.rv), st.span.line, st.span.col);
            }
            let term = match &b.term {
                Term::Goto(t) => format!("goto b{t}"),
                Term::Branch(c, yes, no) => format!("branch {} b{yes} b{no}", operand(c)),
                Term::Call { callee, args, writeback, dest, next } => {
                    let callee = match callee {
                        Callee::Fn(f) => format!("fn {f}"),
                        Callee::Indirect(op) => format!("indirect {}", operand(op)),
                        Callee::Print => "print".to_string(),
                        Callee::Native(n) => format!("native {n}"),
                        Callee::Panic => "panic".to_string(),
                        Callee::Extern(x) => format!("extern {x}"),
                    };
                    let back: Vec<String> = writeback.iter().map(|(i, p)| format!("{i}->{}", place(p))).collect();
                    format!("call {callee}({}) back[{}] -> {} b{next}", operands(args), back.join(", "), place(dest))
                }
                Term::Return(op) => format!("return {}", operand(op)),
                Term::Panic(msg) => format!("panic {}", quote(msg)),
                Term::Unreachable => "unreachable".to_string(),
            };
            let _ = writeln!(out, "    {term}");
        }
    }
    let ints = |xs: &[usize]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" ");
    let _ = writeln!(out, "consts {}", ints(&p.consts));
    let _ = writeln!(out, "main {}", p.main.map_or("_".to_string(), |m| m.to_string()));
    let _ = writeln!(out, "tests {}", ints(&p.tests));
    for (i, (name, fields)) in p.structs.iter().enumerate() {
        let record: Vec<String> = p.records[i].iter().map(|k| k.to_string()).collect();
        let _ = writeln!(out, "struct {i} {name} ({}) record[{}] boxed={}", fields.join(", "), record.join(" "), p.boxed.0[i]);
    }
    for (i, (name, variants)) in p.enums.iter().enumerate() {
        let _ = writeln!(out, "enum {i} {name} boxed={}", p.boxed.1[i]);
        for (v, fields) in variants {
            let fields: Vec<String> = fields.iter().map(|f| f.clone().unwrap_or_else(|| "_".to_string())).collect();
            let _ = writeln!(out, "  variant {v} ({})", fields.join(", "));
        }
    }
    let _ = writeln!(out, "dyn_from {}", p.dyn_from);
    for x in &p.externs {
        let _ = writeln!(out, "extern {} {} ({}) -> {}", x.name, x.header, x.params.join(", "), x.ret);
    }
    for (i, name, kinds) in &p.cstructs {
        let _ = writeln!(out, "cstruct {i} {name} ({})", kinds.join(", "));
    }
    out
}

/// Inclusive range of an integer kind, as far as the interpreter models it.
pub fn bounds(kind: &str) -> (i128, i128) {
    match kind {
        "lit" | "i128" => (i128::MIN, i128::MAX),
        "u128" => (0, i128::MAX),
        "isize" => bounds("i64"),
        "usize" => bounds("u64"),
        _ => {
            let bits: u32 = kind[1..].parse().unwrap_or(64);
            if kind.starts_with('i') {
                (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
            } else {
                (0, (1i128 << bits) - 1)
            }
        }
    }
}

pub fn conv(from: &Ty, to: &Ty) -> Option<Conv> {
    use Ty::*;
    match (from, to) {
        (Never | Unknown, _) | (_, Unknown) => None,
        (IntLit, Int(k)) => Some(Conv::Fit(k)),
        (IntLit, Float(k)) => Some(Conv::ToFloat(*k == "f32")),
        (IntLit, FloatLit) => Some(Conv::ToFloat(false)),
        (FloatLit, Float(k)) if *k == "f32" => Some(Conv::ToFloat(true)),
        (Optional(a), Optional(b)) => conv(a, b).map(|c| Conv::Inside(Box::new(c))),
        (a, Optional(b)) => Some(Conv::Wrap(conv(a, b).map(Box::new))),
        (Tuple(a), Tuple(b)) if a.len() == b.len() => {
            let cs: Vec<Option<Conv>> = a.iter().zip(b).map(|(x, y)| conv(x, y)).collect();
            cs.iter().any(|c| c.is_some()).then_some(Conv::Tuple(cs))
        }
        (Slice(a) | Array(a), Slice(b) | Array(b)) => conv(a, b).map(|c| Conv::Elems(Box::new(c))),
        (Result(a, x), Result(b, y)) => {
            let (ok, err) = (conv(a, b), conv(x, y));
            (ok.is_some() || err.is_some()).then(|| Conv::Res(ok.map(Box::new), err.map(Box::new)))
        }
        _ => None,
    }
}

fn num_of(t: &Ty) -> Num {
    match t {
        Ty::Int(k) => Num::Int(k),
        Ty::IntLit => Num::Int("lit"),
        Ty::Float("f64") | Ty::FloatLit => Num::F64,
        Ty::Float(_) => Num::Float,
        _ => Num::Dyn,
    }
}

#[derive(Default)]
struct Scope<'a> {
    vars: HashMap<String, Local>,
    defers: Vec<&'a Expr>,
    /// The scope of a `scope` block: where its tasks are waited for on the way out.
    tasks: Option<Local>,
}

struct LoopCtx {
    brk: BlockId,
    cont: BlockId,
    /// Where a `break` value goes, and the type it must have.
    result: Option<(Local, Ty)>,
    /// Number of scopes that were open when the loop started.
    depth: usize,
}

enum Job<'a> {
Fn { name: String, f: &'a FnDecl, sig: &'a Sig, self_mode: Option<Mode>, span: Span, inst: usize, path: bool, limits: u8 },
    Const { name: &'a str, value: &'a Expr, span: Span },
}

struct Lower<'a> {
    info: &'a Info,
    fn_ids: HashMap<usize, usize>,
    method_ids: HashMap<(String, String), usize>,
    const_ids: Vec<usize>,
    /// Function of each generic instance.
    instance_ids: Vec<usize>,
    /// Instance whose types are being read (see `Info::types`).
    inst: usize,
    fn_decls: HashMap<usize, &'a FnDecl>,
    struct_decls: Vec<&'a StructDecl>,
    errors: Vec<Error>,
    // State of the function being lowered.
    blocks: Vec<Block>,
    cur: BlockId,
    locals: usize,
    scopes: Vec<Scope<'a>>,
    loops: Vec<LoopCtx>,
    ret: Ty,
    err: Option<Ty>,
    span: Span,
    /// Inside an `|` pattern every alternative binds the same locals.
    or_depth: usize,
    /// For a closure: the locals of the enclosing function it captures.
    captures: Vec<Local>,
    is_closure: bool,
    vars: Vec<(Local, String)>,
    once: Vec<Local>,
    hints: Vec<(Local, Scalar)>,
    /// Functions being lowered around the current closure.
    stack: Vec<Saved<'a>>,
    /// Closures lowered so far; their ids start at `base`.
    extra: Vec<Func>,
    base: usize,
    /// For a function that returns `ref mut`: the one that computes where
    /// its result is.
    path_ids: HashMap<usize, usize>,
    /// The call being lowered must compute a place, not a value.
    /// Whether a constant is being lowered: it is all computed while compiling.
    in_const: bool,
    /// Where the function being lowered is declared.
    decl_span: Span,
    copies: Vec<Span>,
    twin: bool,
    /// What the last such call took its result from.
    twin_src: Option<Place>,
}

/// The state of a function whose lowering is suspended.
struct Saved<'a> {
    blocks: Vec<Block>,
    cur: BlockId,
    locals: usize,
    scopes: Vec<Scope<'a>>,
    loops: Vec<LoopCtx>,
    ret: Ty,
    err: Option<Ty>,
    or_depth: usize,
    captures: Vec<Local>,
    is_closure: bool,
    vars: Vec<(Local, String)>,
    once: Vec<Local>,
    hints: Vec<(Local, Scalar)>,
}

pub fn lower(file: &File, info: &Info) -> Result<Program, Vec<Error>> {
    let mut lw = Lower {
        info,
        fn_ids: HashMap::new(),
        method_ids: HashMap::new(),
        const_ids: Vec::new(),
        instance_ids: Vec::new(),
        inst: 0,
        fn_decls: HashMap::new(),
        struct_decls: Vec::new(),
        errors: Vec::new(),
        blocks: Vec::new(),
        cur: 0,
        locals: 0,
        scopes: Vec::new(),
        loops: Vec::new(),
        ret: Ty::Unit,
        err: None,
        span: Span::default(),
        or_depth: 0,
        captures: Vec::new(),
        is_closure: false,
        vars: Vec::new(),
        once: Vec::new(),
        hints: Vec::new(),
        stack: Vec::new(),
        extra: Vec::new(),
        base: 0,
        path_ids: HashMap::new(),
        twin: false,
        twin_src: None,
        in_const: false,
        decl_span: Span::default(),
        copies: Vec::new(),
    };
    let mut jobs = Vec::new();
    // The methods whose `self` has a type known here, and the type.
    let mut receivers: Vec<(usize, Ty)> = Vec::new();
    let mut main = None;
    let mut tests = Vec::new();
    for (i, d) in file.decls.iter().enumerate() {
        match &d.kind {
            DeclKind::Fn(f) => {
                let Some(&idx) = info.fn_of_decl.get(&i) else { continue };
                if info.fns[idx].generic || !info.fns[idx].tparams.is_empty() {
                    continue;
                }
                lw.fn_ids.insert(idx, jobs.len());
                lw.fn_decls.insert(jobs.len(), f);
                // Only the program's own file can hold the entry point.
                if file.modules.first().is_none_or(|m| m.decls.contains(&i)) {
                    if f.name == "main" {
                        main = Some(jobs.len());
                    } else if f.name.starts_with("test_") && info.fns[idx].sig.params.is_empty() {
                        tests.push(jobs.len());
                    }
                }
                let limits = lw.limits(file, d);
                jobs.push(Job::Fn { name: f.name.clone(), f, sig: &info.fns[idx].sig, self_mode: None, span: d.span, inst: 0, path: false, limits });
            }
            DeclKind::Impl(im) => {
                let Some(key) = info.impl_key.get(&i) else { continue };
                for m in &im.decls {
                    let DeclKind::Fn(f) = &m.kind else { continue };
                    let suffix = info.impl_suffix.get(&i).map_or("", |s| s.as_str());
                    let k = (key.clone(), format!("{}{suffix}", f.name));
                    let Some(method) = info.methods.get(&k) else { continue };
                    if !method.tparams.is_empty() {
                        continue;
                    }
                    lw.method_ids.insert(k, jobs.len());
                    lw.fn_decls.insert(jobs.len(), f);
                    let name = format!("{key}.{}", f.name);
                    let limits = lw.limits(file, m);
                    if method.self_mode.is_some() {
                        receivers.push((jobs.len(), method.self_ty.clone()));
                    }
                    jobs.push(Job::Fn { name, f, sig: &method.sig, self_mode: method.self_mode, span: m.span, inst: 0, path: false, limits });
                }
            }
            DeclKind::Const { name, value, .. } => {
                lw.const_ids.push(jobs.len());
                jobs.push(Job::Const { name, value, span: d.span });
            }
            DeclKind::Struct(s) => lw.struct_decls.push(s),
            _ => {}
        }
    }
    for (k, instance) in info.instances.iter().enumerate() {
        // Every instance takes a slot, so that `instance_ids[k]` is instance `k`.
        let (d, prefix, self_mode) = match &instance.f {
            Target::Fn(fi) => (&file.decls[info.decl_of_fn[*fi]], String::new(), None),
            Target::Method(t, n) => {
                let m = &info.methods[&(t.clone(), n.clone())];
                let Some(d) = crate::check::method_decl(file, m) else { unreachable!("a method lives in an impl or a trait") };
                (d, format!("{t}."), m.self_mode)
            }
        };
        let DeclKind::Fn(f) = &d.kind else { unreachable!("an instance is made from a function") };
        lw.instance_ids.push(jobs.len());
        lw.fn_decls.insert(jobs.len(), f);
        let name = match instance.shown.as_str() {
            "" => format!("{prefix}{}", f.name),
            shown => format!("{prefix}{}[{shown}]", f.name),
        };
        let limits = lw.limits(file, d);
        jobs.push(Job::Fn { name, f, sig: &instance.sig, self_mode, span: d.span, inst: k + 1, path: false, limits });
    }
    // A function that returns `ref mut` is lowered a second time, to
    // compute where its result is instead of the result.
    for i in 0..jobs.len() {
        let Job::Fn { name, f, sig, self_mode, span, inst, limits, .. } = &jobs[i] else { continue };
        if crate::check::ref_kind(f) != Some(true) {
            continue;
        }
        let twin = Job::Fn { name: format!("{name} (place)"), f, sig, self_mode: *self_mode, span: *span, inst: *inst, path: true, limits: *limits };
        lw.path_ids.insert(i, jobs.len());
        lw.fn_decls.insert(jobs.len(), f);
        jobs.push(twin);
    }
    lw.base = jobs.len();
    let mut fns: Vec<Func> = jobs
        .iter()
        .map(|job| match job {
            Job::Fn { name, f, sig, self_mode, span, inst, path, limits } => {
                lw.inst = *inst;
                let mut func = lw.function(name, f, sig, *self_mode, *span, *path);
                func.limits = *limits;
                func
            }
            Job::Const { name, value, span } => {
                lw.inst = 0;
                lw.constant(name, value, *span)
            }
        })
        .collect();
    // The receiver of a method is kept like any parameter of its type.
    for (j, ty) in &receivers {
        if let Some(first) = fns[*j].scalars.first_mut() {
            *first = scalar_of(info, ty);
        }
    }
    fns.append(&mut lw.extra);
    if !lw.errors.is_empty() {
        lw.errors.sort_by_key(|e| (e.span.line, e.span.col));
        lw.errors.dedup_by(|a, b| a.span.line == b.span.line && a.msg == b.msg);
        return Err(lw.errors);
    }
    let mut structs: Vec<(String, Vec<String>)> =
        info.structs.iter().map(|s| (s.name.clone(), s.fields.iter().map(|f| f.name.clone()).collect())).collect();
    let mut records: Vec<Vec<u8>> = info
        .structs
        .iter()
        .map(|s| {
            let kinds: Option<Vec<u8>> = s
                .fields
                .iter()
                .map(|f| match &f.ty {
                    Ty::Bool => Some(1),
                    Ty::Int("i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32") => Some(2),
                    Ty::Int("u64" | "usize") => Some(5),
                    Ty::Float("f64") => Some(3),
                    Ty::Char => Some(4),
                    _ => None,
                })
                .collect();
            kinds.filter(|_| s.tparams.is_empty() && !s.generic).unwrap_or_default()
        })
        .collect();
    records.resize(info.structs.len() + info.traits.len(), Vec::new());
    // A `dyn Trait` value is a struct of the value and its methods, which
    // is shown as the value alone.
    for t in 0..info.traits.len() {
        let slots = crate::check::dyn_slots(&info.traits, &(t, Vec::new()));
        let fields = std::iter::once("value".to_string()).chain(slots.into_iter().map(|s| s.0)).collect();
        structs.push((format!("dyn {}", info.traits[t].name), fields));
    }
    let enums = info
        .enums
        .iter()
        .map(|e| {
            let variants = e.variants.iter().map(|v| (v.name.clone(), v.fields.iter().map(|(n, _)| n.clone()).collect()));
            (e.name.clone(), variants.collect())
        })
        .collect();
    let mut boxed_structs: Vec<bool> =
        (0..info.structs.len()).map(|i| info.structs[i].fields.iter().any(|f| holds(info, &f.ty, (false, i), &mut Vec::new()))).collect();
    boxed_structs.resize(info.structs.len() + info.traits.len(), true);
    let boxed_enums = (0..info.enums.len())
        .map(|i| info.enums[i].variants.iter().any(|v| v.fields.iter().any(|(_, t)| holds(info, t, (true, i), &mut Vec::new()))))
        .collect();
    let dyn_from = info.structs.len();
    let c_type = |t: &Ty| match t {
        Ty::Int(k) | Ty::Float(k) => *k,
        Ty::Bool => "bool",
        Ty::Str => "str",
        Ty::Optional(inner) if **inner == Ty::Str => "str?",
        Ty::Builtin("ptr") => "ptr",
        Ty::Optional(inner) if **inner == Ty::Builtin("ptr") => "ptr?",
        // A struct of C, by its number.
        Ty::Struct(i, _) => Box::leak(format!("struct {i}").into_boxed_str()),
        _ => "unit",
    };
    // A `mut` one goes as its address.
    let c_param = |p: &crate::check::ParamSig| match (&p.ty, p.mode) {
        (Ty::Struct(i, _), Mode::Mut) => &*Box::leak(format!("mut {i}").into_boxed_str()),
        // An array of numbers goes as the address of the numbers.
        (Ty::Slice(t), mode) => match &**t {
            Ty::Int(k) | Ty::Float(k) => &*Box::leak(format!("{} {k}", if mode == Mode::Mut { "mutbuf" } else { "buf" }).into_boxed_str()),
            _ => "unit",
        },
        _ => c_type(&p.ty),
    };
    let mut cstructs: Vec<(usize, String, Vec<&'static str>)> =
        info.cstructs.iter().map(|(i, name)| (*i, name.clone(), info.structs[*i].fields.iter().map(|f| c_type(&f.ty)).collect())).collect();
    cstructs.sort_by_key(|c| c.0);
    let externs = info
        .externs
        .iter()
        .map(|x| Extern {
            name: x.name.clone(),
            header: x.header.clone(),
            params: x.sig.params.iter().map(c_param).collect(),
            ret: c_type(&x.sig.ret),
        })
        .collect();
    let values = vec![None; lw.const_ids.len()];
    Ok(Program { fns, consts: lw.const_ids, values, main, tests, structs, records, enums, boxed: (boxed_structs, boxed_enums), dyn_from, externs, cstructs })
}

impl<'a> Lower<'a> {
    /// The effects a declaration rules out: its own annotations, and what
    /// the profile of its module does not allow.
    fn limits(&mut self, file: &File, d: &Decl) -> u8 {
        let mut limits = 0;
        for a in &d.attrs {
            match a.name.as_str() {
                "noalloc" => limits |= NOALLOC,
                "nopanic" => limits |= NOPANIC,
                "noio" => limits |= NOIO,
                "noblock" => limits |= NOBLOCK,
                "nounsafe" => limits |= NOUNSAFE,
                other => {
                    let msg = format!("unknown annotation `@{other}`: a function takes `@noalloc`, `@nopanic`, `@noio`, `@noblock` and `@nounsafe`");
                    self.errors.push(Error { msg, span: d.span });
                }
            }
        }
        let module = (d.span.line >> crate::lexer::FILE_SHIFT) as usize;
        let profile = match file.modules.get(module) {
            Some(m) => m.profile.as_deref(),
            None => file.profile.as_deref(),
        };
        // A `system` module has no allocator of its own.
        if profile == Some("system") && limits & NOALLOC == 0 {
            limits |= NOALLOC | SYSTEM;
        }
        limits
    }

    /// Whether a value of the type holds an array that can grow.
    fn holds_array(&self, t: &Ty, seen: &mut Vec<Ty>) -> bool {
        if seen.contains(t) {
            return false;
        }
        match t {
            Ty::Slice(_) => true,
            Ty::Array(t) | Ty::Optional(t) => self.holds_array(t, seen),
            Ty::Tuple(ts) => ts.iter().any(|t| self.holds_array(t, seen)),
            Ty::Result(a, b) => self.holds_array(a, seen) || self.holds_array(b, seen),
            Ty::Struct(i, args) => {
                seen.push(t.clone());
                args.iter().any(|a| self.holds_array(a, seen))
                    || self.info.structs.get(*i).is_some_and(|s| s.fields.iter().any(|f| self.holds_array(&f.ty, seen)))
            }
            Ty::Enum(i, args) => {
                seen.push(t.clone());
                args.iter().any(|a| self.holds_array(a, seen))
                    || self.info.enums.get(*i).is_some_and(|e| e.variants.iter().any(|v| v.fields.iter().any(|(_, t)| self.holds_array(t, seen))))
            }
            Ty::Dyn(..) | Ty::Fn(_) | Ty::Unknown => true,
            _ => false,
        }
    }

    /// Notes that the value of an expression is copied to another place,
    /// when that copies an array: it is a variable, a field or an item.
    fn copied(&mut self, v: &Expr) {
        let place = match &v.kind {
            ExprKind::Path(_) => self.info.res.get(&(self.inst, id(v))) == Some(&Res::Local),
            ExprKind::Field { .. } => matches!(self.info.res.get(&(self.inst, id(v))), Some(Res::Field(_))),
            ExprKind::Index { args, .. } => args.len() == 1 && !matches!(self.ty(&args[0]), Ty::Range(_)),
            _ => false,
        };
        if place && self.holds_array(&self.ty(v), &mut Vec::new()) {
            self.copies.push(v.span);
        }
    }

    // ---- building blocks ----

    fn unsupported(&mut self, span: Span, what: &str) -> Operand {
        self.errors.push(Error { msg: format!("the interpreter does not support {what} yet"), span });
        Operand::Const(Value::Unit)
    }

    /// Says how the temporary that holds the value of `e` is kept, when
    /// its type says it.
    fn typed(&mut self, e: &Expr, value: Operand) -> Operand {
        if let (Operand::Place(p), Some(s)) = (&value, scalar_of(self.info, &self.ty(e))) {
            self.hints.push((p.local, s));
        }
        value
    }

    fn ty(&self, e: &Expr) -> Ty {
        self.info.types.get(&(self.inst, id(e))).cloned().unwrap_or(Ty::Unknown)
    }

    /// Type of the value a block produces.
    fn block_ty(&self, b: &Block_) -> Ty {
        b.tail.as_ref().map_or(Ty::Unit, |t| self.ty(t))
    }

    fn temp(&mut self) -> Local {
        self.locals += 1;
        self.locals - 1
    }

    fn new_block(&mut self) -> BlockId {
        self.blocks.push(Block { stmts: Vec::new(), term: Term::Unreachable, span: self.span, done: false });
        self.blocks.len() - 1
    }

    fn switch_to(&mut self, b: BlockId) {
        self.cur = b;
    }

    fn terminate(&mut self, term: Term) {
        let span = self.span;
        let b = &mut self.blocks[self.cur];
        if !b.done {
            b.term = term;
            b.span = span;
            b.done = true;
        }
    }

    /// Ends the current block and continues in a block nothing jumps to.
    fn diverge(&mut self, term: Term) -> Operand {
        self.terminate(term);
        let dead = self.new_block();
        self.switch_to(dead);
        Operand::Const(Value::Unit)
    }

    fn emit(&mut self, place: Place, rv: Rvalue) {
        let span = self.span;
        self.blocks[self.cur].stmts.push(Stmt { place, rv, span });
    }

    fn set(&mut self, local: Local, op: Operand) {
        self.emit(Place::local(local), Rvalue::Use(op));
    }

    /// Stores an rvalue in a fresh local.
    fn tmp(&mut self, rv: Rvalue) -> Operand {
        let t = self.temp();
        self.emit(Place::local(t), rv);
        Operand::Place(Place::local(t))
    }

    /// A place holding the operand, so that it can be projected.
    fn stash(&mut self, op: Operand) -> Place {
        match op {
            Operand::Place(p) => p,
            c => {
                let t = self.temp();
                self.set(t, c);
                Place::local(t)
            }
        }
    }

    /// Copies the value of a place to a fresh local.
    fn read(&mut self, place: Place) -> Operand {
        self.tmp(Rvalue::Use(Operand::Place(place)))
    }

    /// The same, for a place whose type is known.
    fn read_as(&mut self, place: Place, ty: &Ty) -> Operand {
        let op = self.tmp(Rvalue::Use(Operand::Place(place)));
        if let (Operand::Place(p), Some(s)) = (&op, scalar_of(self.info, ty)) {
            self.hints.push((p.local, s));
        }
        op
    }

    fn convert(&mut self, op: Operand, from: &Ty, to: &Ty) -> Operand {
        match (conv(from, to), &op) {
            (None, _) => op,
            (Some(Conv::Fit(k)), Operand::Const(Value::Int(v))) if bounds(k).0 <= *v && *v <= bounds(k).1 => op,
            // A number that is written out and does not fit is not a
            // matter for when the program runs.
            (Some(Conv::Fit(k)), Operand::Const(Value::Int(v))) => {
                let shown = if k == "i64" { "int" } else { k };
                self.errors.push(Error { msg: format!("the number {v} does not fit in `{shown}`"), span: self.span });
                op
            }
            (Some(Conv::ToFloat(false)), Operand::Const(Value::Int(v))) => Operand::Const(Value::Float(*v as f64)),
            (Some(c), _) => self.tmp(Rvalue::Convert(op, c)),
        }
    }

    fn declare(&mut self, name: &str, local: Local) {
        self.vars.push((local, name.to_string()));
        if let Some(scope) = self.scopes.last_mut() {
            scope.vars.insert(name.to_string(), local);
        }
    }

    fn lookup(&mut self, name: &str) -> Option<Local> {
        if let Some(l) = self.scopes.iter().rev().find_map(|s| s.vars.get(name)).copied() {
            return Some(l);
        }
        if !self.is_closure || self.stack.is_empty() {
            return None;
        }
        // A name of the enclosing function: the closure captures its value.
        let outer = self.capture(name, self.stack.len() - 1)?;
        let l = self.temp();
        let rv = Rvalue::Capture(self.captures.len());
        self.captures.push(outer);
        self.blocks[0].stmts.insert(0, Stmt { place: Place::local(l), rv, span: self.span });
        self.scopes[0].vars.insert(name.to_string(), l);
        Some(l)
    }

    /// The local holding `name` in a suspended function, capturing it from
    /// further out when that function is itself a closure.
    fn capture(&mut self, name: &str, level: usize) -> Option<Local> {
        if let Some(l) = self.stack[level].scopes.iter().rev().find_map(|s| s.vars.get(name)).copied() {
            return Some(l);
        }
        if level == 0 || !self.stack[level].is_closure {
            return None;
        }
        let outer = self.capture(name, level - 1)?;
        let span = self.span;
        let st = &mut self.stack[level];
        let l = st.locals;
        st.locals += 1;
        let rv = Rvalue::Capture(st.captures.len());
        st.captures.push(outer);
        st.blocks[0].stmts.insert(0, Stmt { place: Place::local(l), rv, span });
        st.scopes[0].vars.insert(name.to_string(), l);
        Some(l)
    }

    /// `comptime e`: a constant without a name, computed while compiling.
    fn comptime(&mut self, e: &'a Expr, inner: &'a Expr) -> Operand {
        let span = e.span;
        let saved = Saved {
            blocks: std::mem::take(&mut self.blocks),
            cur: self.cur,
            locals: self.locals,
            scopes: std::mem::take(&mut self.scopes),
            loops: std::mem::take(&mut self.loops),
            ret: self.ret.clone(),
            err: self.err.clone(),
            or_depth: self.or_depth,
            captures: std::mem::take(&mut self.captures),
            is_closure: self.is_closure,
            vars: std::mem::take(&mut self.vars),
            once: std::mem::take(&mut self.once),
            hints: std::mem::take(&mut self.hints),
        };
        self.stack.push(saved);
        let outer = (std::mem::take(&mut self.copies), self.decl_span);
        self.start(span);
        // As in a closure, a name of the function around is noticed.
        self.is_closure = true;
        (self.ret, self.err) = (Ty::Unknown, None);
        self.in_const = true;
        let v = self.expr(inner);
        self.in_const = false;
        self.terminate(Term::Return(v));
        let captured = !self.captures.is_empty();
        self.captures.clear();
        let func = self.finish("comptime", 0);
        (self.copies, self.decl_span) = outer;
        if let Some(saved) = self.stack.pop() {
            self.blocks = saved.blocks;
            self.cur = saved.cur;
            self.locals = saved.locals;
            self.scopes = saved.scopes;
            self.loops = saved.loops;
            self.ret = saved.ret;
            self.err = saved.err;
            self.or_depth = saved.or_depth;
            self.captures = saved.captures;
            self.is_closure = saved.is_closure;
            self.vars = saved.vars;
            self.once = saved.once;
            self.hints = saved.hints;
        }
        self.span = span;
        if captured {
            let msg = "`comptime` is computed when the program is compiled, so it cannot use a variable of the function it is in";
            self.errors.push(Error { msg: msg.into(), span });
            return Operand::Const(Value::Unit);
        }
        let f = self.base + self.extra.len();
        self.extra.push(func);
        self.const_ids.push(f);
        self.tmp(Rvalue::Const(self.const_ids.len() - 1))
    }

    fn closure(&mut self, e: &'a Expr, params: &'a [Param], body: &'a Block_) -> Operand {
        let span = e.span;
        let Ty::Fn(sig) = self.ty(e) else { return self.unsupported(span, "this closure") };
        if params.iter().any(|p| p.ty.is_none()) {
            return self.unsupported(span, "closures whose parameters have no type");
        }
        self.closure_of(span, &sig.ret, params, Ok(body))
    }

    /// A closure of a block, or of the expression a task runs.
    fn closure_of(&mut self, span: Span, ret: &Ty, params: &'a [Param], body: Result<&'a Block_, &'a Expr>) -> Operand {
        let saved = Saved {
            blocks: std::mem::take(&mut self.blocks),
            cur: self.cur,
            locals: self.locals,
            scopes: std::mem::take(&mut self.scopes),
            loops: std::mem::take(&mut self.loops),
            ret: self.ret.clone(),
            err: self.err.clone(),
            or_depth: self.or_depth,
            captures: std::mem::take(&mut self.captures),
            is_closure: self.is_closure,
            vars: std::mem::take(&mut self.vars),
            once: std::mem::take(&mut self.once),
            hints: std::mem::take(&mut self.hints),
        };
        self.stack.push(saved);
        let outer = (std::mem::take(&mut self.copies), self.decl_span);
        self.start(span);
        self.is_closure = true;
        for p in params {
            let l = self.temp();
            self.declare(&p.name, l);
        }
        let nparams = self.locals;
        (self.ret, self.err) = match (ret, body) {
            (Ty::Result(t, e), Ok(_)) => ((**t).clone(), Some((**e).clone())),
            (other, _) => (other.clone(), None),
        };
        let (t, v) = match body {
            Ok(b) => (self.block_ty(b), self.block(b)),
            Err(e) => (self.ty(e), self.expr(e)),
        };
        let r = self.ret_value(v, &t);
        self.terminate(Term::Return(r));
        let captures = std::mem::take(&mut self.captures);
        let func = self.finish(if body.is_ok() { "closure" } else { "task" }, nparams);
        (self.copies, self.decl_span) = outer;
        if let Some(saved) = self.stack.pop() {
            self.blocks = saved.blocks;
            self.cur = saved.cur;
            self.locals = saved.locals;
            self.scopes = saved.scopes;
            self.loops = saved.loops;
            self.ret = saved.ret;
            self.err = saved.err;
            self.or_depth = saved.or_depth;
            self.captures = saved.captures;
            self.is_closure = saved.is_closure;
            self.vars = saved.vars;
            self.once = saved.once;
            self.hints = saved.hints;
        }
        let f = self.base + self.extra.len();
        self.extra.push(func);
        let ops = captures.into_iter().map(|l| Operand::Place(Place::local(l))).collect();
        self.span = span;
        self.tmp(Rvalue::Closure(f, ops))
    }

    /// Emits the deferred expressions of every scope from `depth` outwards-in.
    fn run_defers(&mut self, depth: usize) {
        for k in (depth..self.scopes.len()).rev() {
            if let Some(l) = self.scopes[k].tasks {
                let span = self.span;
                self.finish_call(Callee::Native("task.close"), vec![Operand::Place(Place::local(l))], Vec::new(), span);
            }
            let defers = self.scopes[k].defers.clone();
            if defers.is_empty() {
                continue;
            }
            // A deferred expression only sees the names of its own scope.
            let inner = self.scopes.split_off(k + 1);
            for d in defers.iter().rev() {
                self.expr(d);
            }
            self.scopes.extend(inner);
        }
    }

    // ---- functions ----

    fn start(&mut self, span: Span) {
        self.blocks.clear();
        self.locals = 0;
        self.scopes = vec![Scope::default()];
        self.loops.clear();
        self.span = span;
        self.decl_span = span;
        self.copies.clear();
        self.or_depth = 0;
        self.captures.clear();
        self.is_closure = false;
        self.vars.clear();
        self.once.clear();
        self.hints.clear();
        let entry = self.new_block();
        self.switch_to(entry);
    }

    fn finish(&mut self, name: &str, params: usize) -> Func {
        let mut names = vec![None; self.locals];
        for (l, n) in self.vars.drain(..) {
            names[l] = Some(n);
        }
        let once = std::mem::take(&mut self.once);
        let mut hints = vec![None; self.locals];
        for (l, s) in self.hints.drain(..) {
            hints[l] = Some(s);
        }
        Func {
            name: name.to_string(),
            params,
            locals: self.locals,
            blocks: std::mem::take(&mut self.blocks),
            names,
            once,
            scalars: Vec::new(),
            hints,
            ret: None,
            span: self.decl_span,
            limits: 0,
            body: 0,
            copies: std::mem::take(&mut self.copies),
        }
    }

    fn function(&mut self, name: &str, f: &'a FnDecl, sig: &Sig, self_mode: Option<Mode>, span: Span, path: bool) -> Func {
        self.start(span);
        let mut scalars = Vec::new();
        if self_mode.is_some() {
            let l = self.temp();
            self.declare("self", l);
            scalars.push(None);
        }
        for p in &sig.params {
            let l = self.temp();
            self.declare(&p.name, l);
            scalars.push(scalar_of(self.info, &p.ty));
        }
        let params = self.locals;
        (self.ret, self.err) = match &sig.ret {
            Ty::Result(t, e) => ((**t).clone(), Some((**e).clone())),
            other => (other.clone(), None),
        };
        for w in &f.where_ {
            let (ok, bad) = (self.new_block(), self.new_block());
            self.jump(w, ok, bad);
            self.span = w.span;
            self.switch_to(bad);
            self.terminate(Term::Panic(format!("a `where` condition of `{name}` does not hold")));
            self.switch_to(ok);
        }
        let body_start = self.cur;
        match &f.body {
            Some(body) if path => {
                let v = self.path_expr(body);
                self.terminate(Term::Return(v));
            }
            Some(body) => {
                let t = self.ty(body);
                let v = self.expr(body);
                let r = self.ret_value(v, &t);
                self.terminate(Term::Return(r));
            }
            None => {
                self.unsupported(span, "functions without a body");
            }
        }
        let mut func = self.finish(name, params);
        func.scalars = scalars;
        func.ret = if path { None } else { scalar_of(self.info, &sig.ret) };
        func.body = body_start;
        func
    }

    fn constant(&mut self, name: &str, value: &'a Expr, span: Span) -> Func {
        self.start(span);
        (self.ret, self.err) = (Ty::Unknown, None);
        self.in_const = true;
        let v = self.expr(value);
        self.in_const = false;
        self.terminate(Term::Return(v));
        self.finish(&format!("constant {name}"), 0)
    }

    /// Wraps a returned value the way the function's signature requires.
    fn ret_value(&mut self, v: Operand, t: &Ty) -> Operand {
        let (ret, err) = (self.ret.clone(), self.err.clone());
        if assignable(&ret, t) {
            let v = self.convert(v, t, &ret);
            return if err.is_some() { self.tmp(Rvalue::Ok(v)) } else { v };
        }
        if let Some(et) = &err {
            if *et != Ty::Unknown && assignable(et, t) {
                let v = self.convert(v, t, et);
                return self.tmp(Rvalue::Err(v));
            }
        }
        // The function returns nothing: the value of its body is discarded.
        let unit = Operand::Const(Value::Unit);
        if err.is_some() {
            self.tmp(Rvalue::Ok(unit))
        } else {
            unit
        }
    }

    // ---- statements ----

    fn block(&mut self, b: &'a Block_) -> Operand {
        self.scopes.push(Scope::default());
        for s in &b.stmts {
            self.stmt(s);
        }
        let v = match &b.tail {
            Some(e) => self.expr(e),
            None => Operand::Const(Value::Unit),
        };
        self.run_defers(self.scopes.len() - 1);
        self.scopes.pop();
        v
    }

    fn stmt(&mut self, s: &'a crate::ast::Stmt) {
        match s {
            crate::ast::Stmt::Let { pat, value, mutable, .. } => match value {
                Some(v) => {
                    let t = self.info.wants.get(&(self.inst, id(v))).cloned().unwrap_or_else(|| self.ty(v));
                    self.copied(v);
                    let op = self.expr(v);
                    let place = self.stash(op);
                    self.pattern(pat, &place, &t, None);
                }
                None => {
                    if let Pattern::Bind { name, .. } = pat {
                        let l = self.temp();
                        self.declare(name, l);
                        self.emit(Place::local(l), Rvalue::Uninit);
                        if !*mutable {
                            self.once.push(l);
                        }
                    }
                }
            },
            crate::ast::Stmt::Assign { target, op, value } => {
                if *op == "=" {
                    self.copied(value);
                    let v = self.expr(value);
                    let place = self.place(target);
                    self.span = target.span;
                    self.emit(place, Rvalue::Use(v));
                    return;
                }
                let op = &op[..op.len() - 1];
                let (tt, vt) = (self.ty(target), self.ty(value));
                // The value is computed first, so the target is not held across it.
                let v = self.expr(value);
                let j = if matches!(op, "<<" | ">>") { tt.clone() } else { join(&tt, &vt).unwrap_or(Ty::Unknown) };
                let v = if matches!(op, "<<" | ">>") { v } else { self.convert(v, &vt, &j) };
                let place = self.place(target);
                let cur = self.read_as(place.clone(), &tt);
                self.span = target.span;
                match BinOp::parse(op) {
                    // A part of a value: the result is computed as a
                    // number of the machine and then stored.
                    Some(b) if !place.proj.is_empty() && scalar_of(self.info, &tt).is_some() && scalar_of(self.info, &tt) == scalar_of(self.info, &j) => {
                        let r = self.tmp(Rvalue::Binary(b, num_of(&j), cur, v));
                        if let (Operand::Place(p), Some(s)) = (&r, scalar_of(self.info, &tt)) {
                            self.hints.push((p.local, s));
                        }
                        self.emit(place, Rvalue::Use(r));
                    }
                    Some(b) => self.emit(place, Rvalue::Binary(b, num_of(&j), cur, v)),
                    None => {
                        self.unsupported(target.span, "this assignment operator");
                    }
                }
            }
            crate::ast::Stmt::Defer(e) => {
                if let Some(scope) = self.scopes.last_mut() {
                    scope.defers.push(e);
                }
            }
            crate::ast::Stmt::Expr(e) => {
                self.expr(e);
            }
            // Lowered with the declarations of the module.
            crate::ast::Stmt::Decl(_) => {}
        }
    }

    /// The place an expression denotes, or a temporary holding its value.
    fn place(&mut self, e: &'a Expr) -> Place {
        let res = self.info.res.get(&(self.inst, id(e)));
        match &e.kind {
            ExprKind::Path(name) if res == Some(&Res::Local) => match self.lookup(name) {
                Some(l) => return Place::local(l),
                None => {}
            },
            ExprKind::Field { base, .. } => {
                if let Some(Res::Field(k)) = res {
                    return self.place(base).field(*k);
                }
            }
            ExprKind::Index { base, args } if args.len() == 1 => {
                let indexable = matches!(self.ty(base), Ty::Slice(_) | Ty::Array(_));
                if indexable && !matches!(self.ty(&args[0]), Ty::Range(_)) {
                    let p = self.place(base);
                    let i = self.expr(&args[0]);
                    return p.index(i);
                }
            }
            // The result of a `ref mut` function that is written to: the
            // call computes where it is inside what it was taken from.
            ExprKind::Call { .. } if self.info.ref_writes.contains(&(self.inst, id(e))) => {
                self.twin = true;
                let steps = self.expr(e);
                self.twin = false;
                if let Some(src) = self.twin_src.take() {
                    let steps = self.stash(steps).local;
                    return src.path(steps);
                }
                return self.stash(steps);
            }
            _ => {}
        }
        let op = self.expr(e);
        self.stash(op)
    }

    /// Lowers the body of a function that returns `ref mut` to the steps
    /// that lead from its first parameter to the result.
    fn path_expr(&mut self, e: &'a Expr) -> Operand {
        match &e.kind {
            ExprKind::Block(b) => self.path_block(b),
            ExprKind::If { cond, then, els: Some(els) } => {
                let (else_bb, end) = (self.new_block(), self.new_block());
                let res = self.temp();
                self.scopes.push(Scope::default());
                self.cond(cond, else_bb);
                let tv = self.path_block(then);
                self.scopes.pop();
                self.set(res, tv);
                self.terminate(Term::Goto(end));
                self.switch_to(else_bb);
                let ev = self.path_expr(els);
                self.set(res, ev);
                self.terminate(Term::Goto(end));
                self.switch_to(end);
                Operand::Place(Place::local(res))
            }
            _ => {
                let p = self.place(e);
                self.span = e.span;
                // Fields and indices known here go together; steps another
                // function computed are joined to them.
                let mut all: Option<Operand> = None;
                let mut known = Vec::new();
                for pr in &p.proj {
                    match pr {
                        Proj::Field(k) => known.extend([Operand::Const(Value::Int(0)), Operand::Const(Value::Int(*k as i128))]),
                        Proj::Index(i) => known.extend([Operand::Const(Value::Int(1)), i.clone()]),
                        Proj::Path(l) => {
                            let more = Operand::Place(Place::local(*l));
                            let before = match (all.take(), known.is_empty()) {
                                (None, true) => None,
                                (None, false) => Some(self.tmp(Rvalue::Array(std::mem::take(&mut known)))),
                                (Some(a), true) => Some(a),
                                (Some(a), false) => {
                                    let k = self.tmp(Rvalue::Array(std::mem::take(&mut known)));
                                    Some(self.finish_call(Callee::Native("path.join"), vec![a, k], Vec::new(), e.span))
                                }
                            };
                            all = Some(match before {
                                Some(b) => self.finish_call(Callee::Native("path.join"), vec![b, more], Vec::new(), e.span),
                                None => more,
                            });
                        }
                    }
                }
                match (all, known.is_empty()) {
                    (Some(a), true) => a,
                    (Some(a), false) => {
                        let k = self.tmp(Rvalue::Array(known));
                        self.finish_call(Callee::Native("path.join"), vec![a, k], Vec::new(), e.span)
                    }
                    (None, _) => self.tmp(Rvalue::Array(known)),
                }
            }
        }
    }

    fn path_block(&mut self, b: &'a Block_) -> Operand {
        self.scopes.push(Scope::default());
        for s in &b.stmts {
            self.stmt(s);
        }
        let v = match &b.tail {
            Some(t) => self.path_expr(t),
            None => self.tmp(Rvalue::Array(Vec::new())),
        };
        self.run_defers(self.scopes.len() - 1);
        self.scopes.pop();
        v
    }

    // ---- patterns ----

    /// Jumps to `fail` when the condition is false. Without a `fail` block the
    /// pattern is known to always match and nothing is tested.
    fn test(&mut self, rv: Rvalue, fail: Option<BlockId>) {
        if let Some(fail) = fail {
            let c = self.tmp(rv);
            let ok = self.new_block();
            self.terminate(Term::Branch(c, ok, fail));
            self.switch_to(ok);
        }
    }

    fn patterns(&mut self, ps: &'a [Pattern], place: &Place, tys: &[Ty], fail: Option<BlockId>) {
        for (k, p) in ps.iter().enumerate() {
            self.pattern(p, &place.field(k), tys.get(k).unwrap_or(&Ty::Unknown), fail);
        }
    }

    fn pattern(&mut self, p: &'a Pattern, place: &Place, ty: &Ty, fail: Option<BlockId>) {
        let info = self.info;
        let here = Operand::Place(place.clone());
        match p {
            Pattern::Wildcard | Pattern::Rest => {}
            Pattern::Bind { name, .. } => {
                let reused = if self.or_depth > 0 { self.scopes.last().and_then(|s| s.vars.get(name)).copied() } else { None };
                let l = reused.unwrap_or_else(|| self.temp());
                if let Some(s) = scalar_of(self.info, ty) {
                    self.hints.push((l, s));
                }
                self.set(l, here);
                self.declare(name, l);
            }
            Pattern::Literal(e) => {
                if fail.is_none() {
                    return;
                }
                let et = self.ty(e);
                let j = join(ty, &et).unwrap_or(Ty::Unknown);
                let v = self.expr(e);
                let v = self.convert(v, &et, &j);
                let cur = self.read(place.clone());
                let cur = self.convert(cur, ty, &j);
                self.test(Rvalue::Binary(BinOp::Eq, Num::Dyn, cur, v), fail);
            }
            Pattern::Range { lo, hi, inclusive } => {
                if fail.is_none() {
                    return;
                }
                let cur = self.read(place.clone());
                let lt = self.ty(lo);
                let v = self.expr(lo);
                let v = self.convert(v, &lt, ty);
                self.test(Rvalue::Binary(BinOp::Ge, Num::Dyn, cur.clone(), v), fail);
                if let Some(hi) = hi {
                    let ht = self.ty(hi);
                    let v = self.expr(hi);
                    let v = self.convert(v, &ht, ty);
                    let op = if *inclusive { BinOp::Le } else { BinOp::Lt };
                    self.test(Rvalue::Binary(op, Num::Dyn, cur, v), fail);
                }
            }
            Pattern::Ctor { path, args } => {
                let name = path.last().map_or("", |n| n.as_str());
                match ty {
                    Ty::Optional(inner) if name == "Some" => {
                        self.test(Rvalue::Is(here, 1), fail);
                        self.patterns(args, place, std::slice::from_ref(inner), fail);
                    }
                    Ty::Optional(_) => self.test(Rvalue::Is(here, 0), fail),
                    Ty::Enum(i, targs) => {
                        let Some(v) = info.enums[*i].variants.iter().position(|v| v.name == name) else { return };
                        self.test(Rvalue::Is(here, v), fail);
                        let tys = info.enums[*i].field_tys(v, targs);
                        self.patterns(args, place, &tys, fail);
                    }
                    Ty::Struct(i, targs) => {
                        let tys = info.structs[*i].field_tys(targs);
                        self.patterns(args, place, &tys, fail);
                    }
                    _ => {
                        self.unsupported(self.span, "patterns on values whose type is not checked");
                    }
                }
            }
            Pattern::Struct { path, fields, .. } => {
                let name = path.last().map_or("", |n| n.as_str());
                let known: Vec<(String, Ty)> = match ty {
                    Ty::Struct(i, targs) => {
                        let st = &info.structs[*i];
                        st.fields.iter().map(|f| f.name.clone()).zip(st.field_tys(targs)).collect()
                    }
                    Ty::Enum(i, targs) => {
                        let Some(v) = info.enums[*i].variants.iter().position(|v| v.name == name) else { return };
                        self.test(Rvalue::Is(here, v), fail);
                        let fs = &info.enums[*i].variants[v].fields;
                        fs.iter().map(|(n, _)| n.clone().unwrap_or_default()).zip(info.enums[*i].field_tys(v, targs)).collect()
                    }
                    _ => {
                        self.unsupported(self.span, "patterns on values whose type is not checked");
                        return;
                    }
                };
                for (fname, sub) in fields {
                    let Some(k) = known.iter().position(|(n, _)| n == fname) else { continue };
                    let field = place.field(k);
                    match sub {
                        Some(p) => self.pattern(p, &field, &known[k].1, fail),
                        None => {
                            let l = self.temp();
                            self.set(l, Operand::Place(field));
                            self.declare(fname, l);
                        }
                    }
                }
            }
            Pattern::Tuple(ps) => {
                let Ty::Tuple(ts) = ty else {
                    self.unsupported(self.span, "patterns on values whose type is not checked");
                    return;
                };
                let rest = ps.iter().position(|p| matches!(p, Pattern::Rest)).unwrap_or(ps.len());
                for (k, p) in ps.iter().enumerate() {
                    // Elements after a `..` are counted from the end.
                    let at = if k > rest { (ts.len() + k).saturating_sub(ps.len()) } else { k };
                    self.pattern(p, &place.field(at), ts.get(at).unwrap_or(&Ty::Unknown), fail);
                }
            }
            Pattern::Slice(ps) => {
                let (Ty::Slice(elem) | Ty::Array(elem)) = ty else {
                    self.unsupported(self.span, "patterns on values whose type is not checked");
                    return;
                };
                let n = self.tmp(Rvalue::Len(here));
                let rest = ps.iter().position(|p| matches!(p, Pattern::Rest));
                let fixed = Operand::Const(Value::Int((ps.len() - rest.map_or(0, |_| 1)) as i128));
                let cmp = if rest.is_some() { BinOp::Ge } else { BinOp::Eq };
                self.test(Rvalue::Binary(cmp, Num::Dyn, n.clone(), fixed), fail);
                for (k, p) in ps.iter().enumerate() {
                    let index = match rest {
                        Some(r) if k > r => {
                            let back = Operand::Const(Value::Int((ps.len() - k) as i128));
                            self.tmp(Rvalue::Binary(BinOp::Sub, Num::Int("usize"), n.clone(), back))
                        }
                        _ => Operand::Const(Value::Int(k as i128)),
                    };
                    self.pattern(p, &place.index(index), elem, fail);
                }
            }
            Pattern::Or(alts) => {
                let Some(fail) = fail else {
                    if let Some(first) = alts.first() {
                        self.pattern(first, place, ty, None);
                    }
                    return;
                };
                let ok = self.new_block();
                self.or_depth += 1;
                for (k, alt) in alts.iter().enumerate() {
                    let next = if k + 1 == alts.len() { fail } else { self.new_block() };
                    self.pattern(alt, place, ty, Some(next));
                    self.terminate(Term::Goto(ok));
                    if next != fail {
                        self.switch_to(next);
                    }
                }
                self.or_depth -= 1;
                self.switch_to(ok);
            }
        }
    }

    // ---- expressions ----

    /// Lowers an expression, converting it to the type its context requires.
    /// The result is a constant or a fresh temporary, never a variable.
    fn expr(&mut self, e: &'a Expr) -> Operand {
        let op = self.expr_raw(e);
        match self.info.wants.get(&(self.inst, id(e))) {
            Some(want) => {
                let from = self.ty(e);
                self.span = e.span;
                self.convert(op, &from, want)
            }
            None => op,
        }
    }

    /// Branches to `fail` unless the condition holds; bindings of an
    /// `if let` are declared in the current scope.
    /// Ends the block by going to `yes` or to `no` on a condition. `&&`,
    /// `||` and `!` become jumps, so that each way out knows which of the
    /// comparisons held.
    fn jump(&mut self, e: &'a Expr, yes: BlockId, no: BlockId) {
        match &e.kind {
            ExprKind::Binary { op, lhs, rhs } if *op == "&&" || *op == "||" => {
                let more = self.new_block();
                if *op == "&&" {
                    self.jump(lhs, more, no);
                } else {
                    self.jump(lhs, yes, more);
                }
                self.switch_to(more);
                self.jump(rhs, yes, no);
            }
            ExprKind::Unary { op, expr } if *op == "!" && self.ty(expr) == Ty::Bool => self.jump(expr, no, yes),
            _ => {
                let v = self.expr(e);
                self.span = e.span;
                self.terminate(Term::Branch(v, yes, no));
            }
        }
    }

    fn cond(&mut self, c: &'a Cond, fail: BlockId) {
        match c {
            Cond::Expr(e) => {
                let ok = self.new_block();
                self.jump(e, ok, fail);
                self.switch_to(ok);
            }
            Cond::Let(pat, e) => {
                let t = self.ty(e);
                let v = self.expr(e);
                let place = self.stash(v);
                self.pattern(pat, &place, &t, Some(fail));
            }
        }
    }

    fn expr_raw(&mut self, e: &'a Expr) -> Operand {
        let span = e.span;
        self.span = span;
        let info = self.info;
        let unit = Operand::Const(Value::Unit);
        match &e.kind {
            ExprKind::Int(s) => match int_value(s).and_then(|v| i128::try_from(v).ok()) {
                Some(v) => Operand::Const(Value::Int(v)),
                None => self.unsupported(span, "integer literals of 2^127 or more"),
            },
            ExprKind::Float(s) => match s.replace('_', "").parse::<f64>() {
                Ok(v) => Operand::Const(Value::Float(v)),
                Err(_) => self.unsupported(span, "this float literal"),
            },
            ExprKind::Char(c) => Operand::Const(Value::Char(*c)),
            ExprKind::Bool(b) => Operand::Const(Value::Bool(*b)),
            ExprKind::Str(parts) => {
                let mut out = Vec::new();
                for part in parts {
                    match part {
                        StrPart::Lit(s) => out.push(FmtPart::Lit(s.clone())),
                        StrPart::Expr(inner) => out.push(FmtPart::Val(self.expr(inner))),
                    }
                }
                match out.as_slice() {
                    [] => Operand::Const(Value::Str("".into())),
                    [FmtPart::Lit(s)] => Operand::Const(Value::Str(s.as_str().into())),
                    _ => {
                        self.span = span;
                        self.tmp(Rvalue::Format(out))
                    }
                }
            }
            ExprKind::Path(name) => match info.res.get(&(self.inst, id(e))) {
                Some(Res::Local) => match self.lookup(name) {
                    Some(l) => self.read(Place::local(l)),
                    None => self.unsupported(span, "this name"),
                },
                Some(Res::NoneValue) => Operand::Const(Value::None),
                Some(Res::Fn(i)) => match self.fn_ids.get(i) {
                    Some(f) => Operand::Const(Value::Fn(*f)),
                    None => self.unsupported(span, "generic functions"),
                },
                Some(Res::Const(i)) => self.tmp(Rvalue::Const(*i)),
                _ => self.unsupported(span, "generic functions and imported modules"),
            },
            ExprKind::Tuple(items) if items.is_empty() => unit,
            ExprKind::Tuple(items) => {
                let ops = items.iter().map(|i| self.expr(i)).collect();
                self.tmp(Rvalue::Tuple(ops))
            }
            ExprKind::Array(items) => {
                let elem = match self.ty(e) {
                    Ty::Array(t) => *t,
                    _ => Ty::Unknown,
                };
                let mut ops = Vec::new();
                for item in items {
                    let t = self.ty(item);
                    let v = self.expr(item);
                    ops.push(self.convert(v, &t, &elem));
                }
                self.tmp(Rvalue::Array(ops))
            }
            ExprKind::ArrayRepeat(value, len) => {
                let v = self.expr(value);
                let n = self.expr(len);
                self.span = span;
                self.tmp(Rvalue::Repeat(v, n))
            }
            ExprKind::Block(b) | ExprKind::Unsafe(b) => self.block(b),
            // Without a compile-time evaluator yet, `comptime` runs with the program.
            ExprKind::Comptime(inner) if self.in_const => self.expr(inner),
            ExprKind::Comptime(inner) => self.comptime(e, inner),
            ExprKind::If { cond, then, els } => {
                let j = self.ty(e);
                let (else_bb, end) = (self.new_block(), self.new_block());
                let res = self.temp();
                self.scopes.push(Scope::default());
                self.cond(cond, else_bb);
                let tv = self.block(then);
                self.scopes.pop();
                if els.is_some() {
                    let tv = self.convert(tv, &self.block_ty(then), &j);
                    self.set(res, tv);
                }
                self.terminate(Term::Goto(end));
                self.switch_to(else_bb);
                if let Some(els) = els {
                    let et = self.ty(els);
                    let ev = self.expr(els);
                    let ev = self.convert(ev, &et, &j);
                    self.set(res, ev);
                }
                self.terminate(Term::Goto(end));
                self.switch_to(end);
                if els.is_some() {
                    Operand::Place(Place::local(res))
                } else {
                    unit
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                let (st, j) = (self.ty(scrutinee), self.ty(e));
                let v = self.expr(scrutinee);
                let place = self.stash(v);
                let res = self.temp();
                let end = self.new_block();
                for arm in arms {
                    let next = self.new_block();
                    self.scopes.push(Scope::default());
                    self.pattern(&arm.pat, &place, &st, Some(next));
                    if let Some(g) = &arm.guard {
                        let c = self.expr(g);
                        let ok = self.new_block();
                        self.terminate(Term::Branch(c, ok, next));
                        self.switch_to(ok);
                    }
                    let bt = self.ty(&arm.body);
                    let bv = self.expr(&arm.body);
                    let bv = self.convert(bv, &bt, &j);
                    self.set(res, bv);
                    self.scopes.pop();
                    self.terminate(Term::Goto(end));
                    self.switch_to(next);
                }
                self.span = span;
                self.terminate(Term::Panic("no arm of this `match` matched the value".into()));
                self.switch_to(end);
                Operand::Place(Place::local(res))
            }
            ExprKind::For { pat, iter, body } => {
                let it = self.ty(iter);
                // `for i in a..b` over integers counts in two locals; no
                // range value is built.
                if let (Ty::Range(t), ExprKind::Range { lo: Some(lo), hi: Some(hi), inclusive }) = (&it, &iter.kind) {
                    if let Ty::Int(_) | Ty::IntLit = **t {
                        let k = if let Ty::Int(k) = **t { k } else { "i64" };
                        let first = self.expr(lo);
                        let last = self.expr(hi);
                        let (i, end) = (self.temp(), self.temp());
                        self.hints.push((i, Scalar::Int(k)));
                        self.hints.push((end, Scalar::Int(k)));
                        self.set(i, first);
                        self.set(end, last);
                        let (cursor, end) = (Operand::Place(Place::local(i)), Operand::Place(Place::local(end)));
                        let (head, body_bb, step, exit) = (self.new_block(), self.new_block(), self.new_block(), self.new_block());
                        self.terminate(Term::Goto(head));
                        self.switch_to(head);
                        let op = if *inclusive { BinOp::Le } else { BinOp::Lt };
                        let more = self.tmp(Rvalue::Binary(op, Num::Dyn, cursor.clone(), end.clone()));
                        self.terminate(Term::Branch(more, body_bb, exit));
                        self.switch_to(body_bb);
                        self.scopes.push(Scope::default());
                        self.pattern(pat, &Place::local(i), t, None);
                        self.loops.push(LoopCtx { brk: exit, cont: step, result: None, depth: self.scopes.len() });
                        self.block(body);
                        self.loops.pop();
                        self.scopes.pop();
                        self.terminate(Term::Goto(step));
                        self.switch_to(step);
                        self.span = span;
                        if *inclusive {
                            // The last value may be the largest of its type.
                            let done = self.tmp(Rvalue::Binary(BinOp::Eq, Num::Dyn, cursor.clone(), end));
                            let advance = self.new_block();
                            self.terminate(Term::Branch(done, exit, advance));
                            self.switch_to(advance);
                        }
                        self.emit(Place::local(i), Rvalue::Succ(cursor, Num::Int(k)));
                        self.terminate(Term::Goto(head));
                        self.switch_to(exit);
                        return unit;
                    }
                }
                let v = self.expr(iter);
                let (head, body_bb, step, exit) = (self.new_block(), self.new_block(), self.new_block(), self.new_block());
                let i = self.temp();
                let cursor = Operand::Place(Place::local(i));
                // What each iteration binds, and its type.
                let (item, elem, counter) = match &it {
                    Ty::Range(t) => {
                        let num = match &**t {
                            Ty::Int(k) => Num::Int(k),
                            Ty::IntLit => Num::Int("i64"),
                            _ => Num::Dyn,
                        };
                        self.emit(Place::local(i), Rvalue::RangeStart(v.clone(), num));
                        (Place::local(i), (**t).clone(), num)
                    }
                    Ty::Slice(t) | Ty::Array(t) => {
                        self.set(i, Operand::Const(Value::Int(0)));
                        (self.stash(v.clone()).index(cursor.clone()), (**t).clone(), Num::Int("usize"))
                    }
                    Ty::Str => {
                        self.set(i, Operand::Const(Value::Int(0)));
                        let chars = self.tmp(Rvalue::Chars(v.clone()));
                        (self.stash(chars).index(cursor.clone()), Ty::Char, Num::Int("usize"))
                    }
                    _ => {
                        self.unsupported(iter.span, "iterating over this type");
                        return unit;
                    }
                };
                let ranged = matches!(it, Ty::Range(_));
                let len = if ranged { unit.clone() } else { self.tmp(Rvalue::Len(Operand::Place(Place::local(item.local)))) };
                self.terminate(Term::Goto(head));
                self.switch_to(head);
                let more = if ranged {
                    self.tmp(Rvalue::RangeHas(v.clone(), cursor.clone()))
                } else {
                    self.tmp(Rvalue::Binary(BinOp::Lt, Num::Dyn, cursor.clone(), len))
                };
                self.terminate(Term::Branch(more, body_bb, exit));
                self.switch_to(body_bb);
                self.scopes.push(Scope::default());
                self.pattern(pat, &item, &elem, None);
                self.loops.push(LoopCtx { brk: exit, cont: step, result: None, depth: self.scopes.len() });
                self.block(body);
                self.loops.pop();
                self.scopes.pop();
                self.terminate(Term::Goto(step));
                self.switch_to(step);
                self.span = span;
                if ranged {
                    let last = self.tmp(Rvalue::RangeLast(v, cursor.clone()));
                    let advance = self.new_block();
                    self.terminate(Term::Branch(last, exit, advance));
                    self.switch_to(advance);
                    self.emit(Place::local(i), Rvalue::Succ(cursor, counter));
                } else {
                    let one = Operand::Const(Value::Int(1));
                    self.emit(Place::local(i), Rvalue::Binary(BinOp::Add, counter, cursor, one));
                }
                self.terminate(Term::Goto(head));
                self.switch_to(exit);
                unit
            }
            ExprKind::While { cond, body } => {
                let (head, exit) = (self.new_block(), self.new_block());
                self.terminate(Term::Goto(head));
                self.switch_to(head);
                self.scopes.push(Scope::default());
                self.cond(cond, exit);
                self.loops.push(LoopCtx { brk: exit, cont: head, result: None, depth: self.scopes.len() });
                self.block(body);
                self.loops.pop();
                self.scopes.pop();
                self.terminate(Term::Goto(head));
                self.switch_to(exit);
                unit
            }
            ExprKind::Loop(body) => {
                let (head, exit) = (self.new_block(), self.new_block());
                let res = self.temp();
                self.terminate(Term::Goto(head));
                self.switch_to(head);
                let result = Some((res, self.ty(e)));
                self.loops.push(LoopCtx { brk: exit, cont: head, result, depth: self.scopes.len() });
                self.block(body);
                self.loops.pop();
                self.terminate(Term::Goto(head));
                self.switch_to(exit);
                Operand::Place(Place::local(res))
            }
            ExprKind::Break(value) => {
                let Some(l) = self.loops.last() else { return unit };
                let (brk, depth, result) = (l.brk, l.depth, l.result.clone());
                if let (Some(v), Some((res, j))) = (value, result) {
                    let t = self.ty(v);
                    let op = self.expr(v);
                    let op = self.convert(op, &t, &j);
                    self.set(res, op);
                }
                self.run_defers(depth);
                self.diverge(Term::Goto(brk))
            }
            ExprKind::Continue => {
                let Some(l) = self.loops.last() else { return unit };
                let (cont, depth) = (l.cont, l.depth);
                self.run_defers(depth);
                self.diverge(Term::Goto(cont))
            }
            ExprKind::Return(value) => {
                let (v, t) = match value {
                    Some(v) => (self.expr(v), self.ty(v)),
                    None => (unit, Ty::Unit),
                };
                let r = self.ret_value(v, &t);
                self.run_defers(0);
                self.span = span;
                self.diverge(Term::Return(r))
            }
            ExprKind::Closure { params, body, .. } => self.closure(e, params, body),
            ExprKind::Scope { name, body } => {
                // Its tasks are waited for wherever the block is left.
                self.scopes.push(Scope::default());
                let opened = self.finish_call(Callee::Native("task.open"), Vec::new(), Vec::new(), span);
                let l = self.temp();
                self.set(l, opened);
                self.declare(name, l);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.tasks = Some(l);
                }
                let v = self.block(body);
                self.span = span;
                self.run_defers(self.scopes.len() - 1);
                self.scopes.pop();
                v
            }
            ExprKind::With { .. } => self.unsupported(span, "`with` blocks"),
            ExprKind::Unary { op, expr } => {
                let t = self.ty(expr);
                let v = self.expr(expr);
                let op = match *op {
                    "-" => UnOp::Neg,
                    "!" => UnOp::Not,
                    _ => UnOp::BitNot,
                };
                self.span = span;
                self.tmp(Rvalue::Unary(op, num_of(&t), v))
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, op, lhs, rhs),
            ExprKind::Range { lo, hi, inclusive } => {
                let bound = |this: &mut Self, b: &'a Option<Box<Expr>>| match b {
                    Some(b) => {
                        let v = this.expr(b);
                        this.tmp(Rvalue::Some(v))
                    }
                    None => Operand::Const(Value::None),
                };
                let lo = bound(self, lo);
                let hi = bound(self, hi);
                self.tmp(Rvalue::Tuple(vec![lo, hi, Operand::Const(Value::Bool(*inclusive))]))
            }
            ExprKind::Cast { expr, .. } => {
                let v = self.expr(expr);
                self.span = span;
                if let Ty::Dyn(t, _) = self.ty(e) {
                    // A value that is already a `dyn` stays as it is.
                    let Some(table) = info.dyns.get(&(self.inst, id(e))) else { return v };
                    let mut ops = vec![v];
                    for r in table {
                        let f = match r {
                            Res::Method(t, n) => self.method_ids.get(&(t.clone(), n.clone())).copied(),
                            Res::Generic(k) => self.instance_ids.get(*k).copied(),
                            _ => None,
                        };
                        match f {
                            Some(f) => ops.push(Operand::Const(Value::Fn(f))),
                            None => return self.unsupported(span, "this conversion to `dyn`"),
                        }
                    }
                    return self.tmp(Rvalue::Struct(info.structs.len() + t, ops));
                }
                match self.ty(e) {
                    Ty::Int(k) => self.tmp(Rvalue::CastInt(v, k)),
                    Ty::Float(k) => self.tmp(Rvalue::CastFloat(v, k == "f32")),
                    Ty::Char => self.finish_call(Callee::Native("raw.to_char"), vec![v], Vec::new(), span),
                    _ => self.unsupported(span, "casts to this type"),
                }
            }
            ExprKind::Call { callee, args } => self.call(e, callee, args),
            ExprKind::Index { base, args } => {
                if args.len() != 1 || !info.types.contains_key(&(self.inst, id(base))) {
                    return self.unsupported(span, "generic arguments");
                }
                if matches!(self.ty(&args[0]), Ty::Range(_)) {
                    let b = self.expr(base);
                    let r = self.expr(&args[0]);
                    self.span = span;
                    return self.tmp(Rvalue::Slice(b, r));
                }
                if !matches!(self.ty(base), Ty::Slice(_) | Ty::Array(_)) {
                    return self.unsupported(span, "indexing a value of this type");
                }
                let place = self.place(e);
                self.span = span;
                self.read_as(place, &self.ty(e))
            }
            ExprKind::Field { base, .. } => match info.res.get(&(self.inst, id(e))) {
                Some(Res::Variant(en, v)) => Operand::Const(Value::Enum(*en, *v, Vec::new())),
                Some(Res::Field(_)) => {
                    let place = self.place(e);
                    self.read_as(place, &self.ty(e))
                }
                Some(Res::Len) => {
                    let b = Operand::Place(self.place(base));
                    self.tmp(Rvalue::Len(b))
                }
                // A function or a constant of another module: `module.name`.
                Some(Res::Fn(i)) => match self.fn_ids.get(i) {
                    Some(f) => Operand::Const(Value::Fn(*f)),
                    None => self.unsupported(span, "generic functions"),
                },
                Some(Res::Const(i)) => self.tmp(Rvalue::Const(*i)),
                _ => self.unsupported(span, "imported modules and generic types"),
            },
            ExprKind::Try(inner) => {
                let t = self.ty(inner);
                let v = self.expr(inner);
                let place = self.stash(v);
                let here = Operand::Place(place.clone());
                let (good, bad) = (self.new_block(), self.new_block());
                self.span = span;
                match t {
                    Ty::Result(_, found) => {
                        let failed = self.tmp(Rvalue::Is(here, 1));
                        self.terminate(Term::Branch(failed, bad, good));
                        self.switch_to(bad);
                        let payload = self.read(place.field(0));
                        let declared = self.err.clone().unwrap_or(Ty::Unknown);
                        let payload = self.convert(payload, &found, &declared);
                        let r = self.tmp(Rvalue::Err(payload));
                        self.run_defers(0);
                        self.terminate(Term::Return(r));
                    }
                    Ty::Optional(_) => {
                        let some = self.tmp(Rvalue::Is(here, 1));
                        self.terminate(Term::Branch(some, good, bad));
                        self.switch_to(bad);
                        let none = Ty::Optional(Box::new(Ty::Never));
                        let r = self.ret_value(Operand::Const(Value::None), &none);
                        self.run_defers(0);
                        self.terminate(Term::Return(r));
                    }
                    _ => return self.unsupported(span, "`?` on a value whose type is not checked"),
                }
                self.switch_to(good);
                self.read(place.field(0))
            }
            ExprKind::Catch { expr, binding, handler } => {
                let Ty::Result(ok, _) = self.ty(expr) else {
                    return self.unsupported(span, "`catch` on a value whose type is not checked");
                };
                let j = self.ty(e);
                let v = self.expr(expr);
                let place = self.stash(v);
                let (good, bad, end) = (self.new_block(), self.new_block(), self.new_block());
                let res = self.temp();
                let failed = self.tmp(Rvalue::Is(Operand::Place(place.clone()), 1));
                self.terminate(Term::Branch(failed, bad, good));
                self.switch_to(bad);
                self.scopes.push(Scope::default());
                if let Some(b) = binding {
                    let l = self.temp();
                    self.set(l, Operand::Place(place.field(0)));
                    self.declare(b, l);
                }
                let hv = self.block(handler);
                let hv = self.convert(hv, &self.block_ty(handler), &j);
                self.set(res, hv);
                self.scopes.pop();
                self.terminate(Term::Goto(end));
                self.switch_to(good);
                let okv = self.read(place.field(0));
                let okv = self.convert(okv, &ok, &j);
                self.set(res, okv);
                self.terminate(Term::Goto(end));
                self.switch_to(end);
                Operand::Place(Place::local(res))
            }
        }
    }

    fn binary(&mut self, e: &'a Expr, op: &str, lhs: &'a Expr, rhs: &'a Expr) -> Operand {
        if op == "&&" || op == "||" {
            let res = self.temp();
            let l = self.expr(lhs);
            self.set(res, l);
            let (more, end) = (self.new_block(), self.new_block());
            let c = Operand::Place(Place::local(res));
            self.terminate(if op == "&&" { Term::Branch(c, more, end) } else { Term::Branch(c, end, more) });
            self.switch_to(more);
            let r = self.expr(rhs);
            self.set(res, r);
            self.terminate(Term::Goto(end));
            self.switch_to(end);
            return Operand::Place(Place::local(res));
        }
        let (lt, rt) = (self.ty(lhs), self.ty(rhs));
        let shift = matches!(op, "<<" | ">>");
        let j = if shift { lt.clone() } else { join(&lt, &rt).unwrap_or(Ty::Unknown) };
        let l = self.expr(lhs);
        let l = if shift { l } else { self.convert(l, &lt, &j) };
        let r = self.expr(rhs);
        let r = if shift { r } else { self.convert(r, &rt, &j) };
        self.span = e.span;
        let Some(b) = BinOp::parse(op) else {
            return self.unsupported(e.span, "this operator");
        };
        use BinOp::*;
        // Wrapping and saturating between untyped literals is made as
        // `int`, so both have to be one.
        let (l, r) = match (b, num_of(&j)) {
            (AddWrap | SubWrap | MulWrap | AddSat | SubSat | MulSat, Num::Int("lit")) => {
                let int = Ty::Int("i64");
                (self.convert(l, &Ty::IntLit, &int), self.convert(r, &Ty::IntLit, &int))
            }
            _ => (l, r),
        };
        let num = match (b, num_of(&j)) {
            (Eq | Ne | Lt | Le | Gt | Ge | BitAnd | BitOr | BitXor, _) => Num::Dyn,
            // Wrapping and saturating need a width: an untyped literal is an `int`.
            (AddWrap | SubWrap | MulWrap | AddSat | SubSat | MulSat, Num::Int("lit")) => Num::Int("i64"),
            (_, n) => n,
        };
        self.tmp(Rvalue::Binary(b, num, l, r))
    }

    // ---- calls ----

    fn call(&mut self, e: &'a Expr, callee: &'a Expr, args: &'a [Arg]) -> Operand {
        let info = self.info;
        let span = e.span;
        // Whether this call must compute the place of its result.
        let twin = std::mem::take(&mut self.twin);
        let pick = |lw: &Self, f: usize| if twin { lw.path_ids.get(&f).copied().unwrap_or(f) } else { f };
        match info.res.get(&(self.inst, id(e))) {
            Some(Res::SomeCtor) => {
                let Some(a) = args.first() else { return Operand::Const(Value::None) };
                let v = self.expr(&a.value);
                self.tmp(Rvalue::Some(v))
            }
            Some(Res::StructCtor(i)) => {
                let mut ops: Vec<Operand> = args.iter().map(|a| self.expr(&a.value)).collect();
                for k in ops.len()..info.structs[*i].fields.len() {
                    let default = self.struct_decls.get(*i).and_then(|s| s.fields.get(k)).and_then(|f| f.default.as_ref());
                    ops.push(self.default(default, 0, span));
                }
                self.tmp(Rvalue::Struct(*i, ops))
            }
            Some(Res::Variant(en, v)) => {
                let ops = args.iter().map(|a| self.expr(&a.value)).collect();
                self.tmp(Rvalue::Enum(*en, *v, ops))
            }
            Some(Res::Fn(i)) => match self.fn_ids.get(i).copied() {
                Some(f) => self.invoke(Callee::Fn(pick(self, f)), None, &info.fns[*i].sig, self.fn_decls.get(&f).copied(), 0, args, span),
                None => self.unsupported(span, "generic functions"),
            },
            Some(Res::Generic(k)) => {
                let Some(f) = self.instance_ids.get(*k).copied() else { return self.unsupported(span, "this generic call") };
                let self_mode = match &info.instances[*k].f {
                    Target::Fn(_) => None,
                    Target::Method(t, n) => info.methods.get(&(t.clone(), n.clone())).and_then(|m| m.self_mode),
                };
                let receiver = match (self_mode, &callee.kind) {
                    (Some(Mode::Mut), ExprKind::Field { base, .. }) => {
                        let p = self.place(base);
                        Some((Operand::Place(p.clone()), Some(p)))
                    }
                    (Some(_), ExprKind::Field { base, .. }) => Some((self.expr(base), None)),
                    _ => None,
                };
                self.invoke(Callee::Fn(pick(self, f)), receiver, &info.instances[*k].sig, self.fn_decls.get(&f).copied(), k + 1, args, span)
            }
            Some(Res::Assoc(t, n)) => {
                let key = (t.clone(), n.clone());
                match (self.method_ids.get(&key).copied(), info.methods.get(&key)) {
                    (Some(f), Some(m)) => self.invoke(Callee::Fn(pick(self, f)), None, &m.sig, self.fn_decls.get(&f).copied(), 0, args, span),
                    _ => self.unsupported(span, "generic methods"),
                }
            }
            Some(Res::Method(t, n)) => {
                let key = (t.clone(), n.clone());
                let (Some(f), Some(m)) = (self.method_ids.get(&key).copied(), info.methods.get(&key)) else {
                    return self.unsupported(span, "generic methods");
                };
                let ExprKind::Field { base, .. } = &callee.kind else { return self.unsupported(span, "this call") };
                let receiver = if m.self_mode == Some(Mode::Mut) {
                    let p = self.place(base);
                    (Operand::Place(p.clone()), Some(p))
                } else {
                    (self.expr(base), None)
                };
                self.invoke(Callee::Fn(pick(self, f)), Some(receiver), &m.sig, self.fn_decls.get(&f).copied(), 0, args, span)
            }
            Some(Res::Dyn(k, is_mut)) => {
                let ExprKind::Field { base, .. } = &callee.kind else { return self.unsupported(span, "this call") };
                let Ty::Dyn(t, targs) = self.ty(base) else { return self.unsupported(span, "this call") };
                let Some((_, sig, _)) = crate::check::dyn_slots(&info.traits, &(t, targs)).into_iter().nth(*k) else {
                    return self.unsupported(span, "this call");
                };
                // The value is the first field; its methods follow.
                let p = self.place(base);
                let f = self.read(p.field(k + 1));
                let inner = p.field(0);
                let receiver = if *is_mut { (Operand::Place(inner.clone()), Some(inner)) } else { (self.read(inner), None) };
                self.invoke(Callee::Indirect(f), Some(receiver), &sig, None, 0, args, span)
            }
            Some(Res::Builtin("panic")) => {
                let ops = args.iter().map(|a| self.expr(&a.value)).collect();
                self.finish_call(Callee::Panic, ops, Vec::new(), span)
            }
            Some(Res::Builtin("assert")) => {
                let Some(first) = args.first() else { return Operand::Const(Value::Unit) };
                let (pass, fail) = (self.new_block(), self.new_block());
                self.jump(&first.value, pass, fail);
                self.span = span;
                // The message is only computed when the assertion fails.
                self.switch_to(fail);
                let message = match args.get(1) {
                    Some(a) => self.expr(&a.value),
                    None => Operand::Const(Value::Str("assertion failed".into())),
                };
                self.finish_call(Callee::Panic, vec![message], Vec::new(), span);
                self.terminate(Term::Goto(pass));
                self.switch_to(pass);
                Operand::Const(Value::Unit)
            }
            Some(Res::Extern(x)) => {
                // A struct it takes as `mut` is read back after the call.
                let sig = &self.info.externs[*x].sig;
                self.invoke(Callee::Extern(*x), None, sig, None, 0, args, span)
            }
            Some(Res::Builtin("task.spawn")) => {
                let Some(a) = args.first() else { return Operand::Const(Value::Unit) };
                let Ty::Task(t) = self.ty(e) else { return self.unsupported(span, "this `spawn`") };
                let run = self.closure_of(span, &t, &[], Err(&a.value));
                self.finish_call(Callee::Native("task.spawn"), vec![run], Vec::new(), span)
            }
            Some(Res::Builtin(name @ ("task.wait" | "task.cancel"))) => {
                let ExprKind::Field { base, .. } = &callee.kind else { return self.unsupported(span, "this call") };
                let task = self.expr(base);
                self.finish_call(Callee::Native(name), vec![task], Vec::new(), span)
            }
            Some(Res::Builtin("mutex.update")) => {
                // The value is taken out, changed and put back: nothing
                // else reads or writes it in between.
                let ExprKind::Field { base, .. } = &callee.kind else { return self.unsupported(span, "this call") };
                let Some(a) = args.first() else { return Operand::Const(Value::Unit) };
                let shared = self.expr(base);
                let m = self.temp();
                self.set(m, shared);
                let change = self.expr(&a.value);
                let cell = || Operand::Place(Place::local(m));
                let old = self.finish_call(Callee::Native("mutex.take"), vec![cell()], Vec::new(), span);
                let new = self.finish_call(Callee::Indirect(change), vec![old], Vec::new(), span);
                self.finish_call(Callee::Native("mutex.put"), vec![cell(), new], Vec::new(), span)
            }
            Some(Res::Builtin(name @ ("chan.send" | "chan.recv" | "chan.close" | "mutex.get" | "mutex.set"))) => {
                let ExprKind::Field { base, .. } = &callee.kind else { return self.unsupported(span, "this call") };
                let mut ops = vec![self.expr(base)];
                ops.extend(args.iter().map(|a| self.expr(&a.value)));
                self.finish_call(Callee::Native(name), ops, Vec::new(), span)
            }
            Some(Res::Builtin(name)) if name.contains('.') && *name != "io.print" => {
                let ops = args.iter().map(|a| self.expr(&a.value)).collect();
                self.finish_call(Callee::Native(name), ops, Vec::new(), span)
            }
            Some(Res::Builtin(name)) if *name != "io.print" => {
                let ExprKind::Field { base, .. } = &callee.kind else { return self.unsupported(span, "this call") };
                match *name {
                    "push" => {
                        let p = self.place(base);
                        let Some(a) = args.first() else { return Operand::Const(Value::Unit) };
                        let v = self.expr(&a.value);
                        self.tmp(Rvalue::Push(p, v))
                    }
                    "pop" => {
                        let p = self.place(base);
                        let got = self.tmp(Rvalue::Pop(p));
                        self.typed(e, got)
                    }
                    _ => {
                        let b = self.expr(base);
                        let Some(a) = args.first() else { return Operand::Const(Value::None) };
                        let i = self.expr(&a.value);
                        let got = self.tmp(Rvalue::Get(b, i));
                        self.typed(e, got)
                    }
                }
            }
            Some(Res::Builtin(_)) => {
                let ops = args.iter().map(|a| self.expr(&a.value)).collect();
                self.finish_call(Callee::Print, ops, Vec::new(), span)
            }
            _ => match self.ty(callee) {
                Ty::Fn(sig) => {
                    let f = self.expr(callee);
                    self.invoke(Callee::Indirect(f), None, &sig, None, 0, args, span)
                }
                _ => self.unsupported(span, "generic functions, imported modules and methods of built-in types"),
            },
        }
    }

    /// Lowers a default value, which only sees global names.
    /// `inst` is the instance the default was type-checked in: the callee's.
    fn default(&mut self, default: Option<&'a Expr>, inst: usize, span: Span) -> Operand {
        match default {
            Some(d) => {
                let saved = std::mem::take(&mut self.scopes);
                let caller = std::mem::replace(&mut self.inst, inst);
                self.scopes.push(Scope::default());
                let v = self.expr(d);
                self.scopes = saved;
                self.inst = caller;
                v
            }
            None => self.unsupported(span, "this call"),
        }
    }

    fn invoke(
        &mut self,
        callee: Callee,
        receiver: Option<(Operand, Option<Place>)>,
        sig: &Sig,
        decl: Option<&'a FnDecl>,
        inst: usize,
        args: &'a [Arg],
        span: Span,
    ) -> Operand {
        let mut ops = Vec::new();
        let mut writeback = Vec::new();
        if let Some((op, place)) = receiver {
            if let Some(p) = place {
                writeback.push((0, p));
            }
            ops.push(op);
        }
        let offset = ops.len();
        for (i, a) in args.iter().enumerate() {
            if sig.params.get(i).is_some_and(|p| p.mode == Mode::Mut) {
                let p = self.place(&a.value);
                writeback.push((offset + i, p.clone()));
                ops.push(Operand::Place(p));
            } else {
                ops.push(self.expr(&a.value));
                // `own x` leaves the variable without a value.
                let own = a.mode == Mode::Own && sig.params.get(i).is_some_and(|p| p.mode == Mode::Own);
                if let (true, ExprKind::Path(name)) = (own, &a.value.kind) {
                    if let Some(l) = self.lookup(name) {
                        self.span = a.value.span;
                        self.emit(Place::local(l), Rvalue::Moved);
                    }
                }
            }
        }
        for i in args.len()..sig.params.len() {
            let param = decl.and_then(|d| d.params.iter().filter(|p| p.name != "self").nth(i));
            ops.push(self.default(param.and_then(|p| p.default.as_ref()), inst, span));
        }
        // A call that computes a place: its result is part of the first argument.
        if let Callee::Fn(f) = &callee {
            if self.path_ids.values().any(|t| t == f) {
                self.twin_src = writeback.iter().find(|(i, _)| *i == 0).map(|(_, p)| p.clone());
            }
        }
        self.finish_call(callee, ops, writeback, span)
    }

    fn finish_call(&mut self, callee: Callee, args: Vec<Operand>, writeback: Vec<(usize, Place)>, span: Span) -> Operand {
        let dest = self.temp();
        let next = self.new_block();
        self.span = span;
        self.terminate(Term::Call { callee, args, writeback, dest: Place::local(dest), next });
        self.switch_to(next);
        Operand::Place(Place::local(dest))
    }
}

type Block_ = crate::ast::Block;
