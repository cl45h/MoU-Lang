//! C backend: turns the MIR into one self-contained C file.
//!
//! Compound values keep the tagged representation the interpreter uses,
//! through the runtime in `runtime.c`, so native programs behave exactly
//! like `mou run`. Locals that hold integers of 64 bits or less, booleans
//! or `f64` are plain C variables, and arrays of those are read and written
//! inline.

use std::fmt::Write;

use crate::mir::*;

const RUNTIME: &str = include_str!("runtime.c");

const KINDS: &[&str] = &["lit", "i8", "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "isize", "usize"];

fn kind(k: &str) -> usize {
    KINDS.iter().position(|x| *x == k).unwrap_or(4)
}

fn num(n: Num) -> String {
    match n {
        Num::Int(k) => kind(k).to_string(),
        _ => "-1".into(),
    }
}

/// A C string literal holding the bytes of `s`.
fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' | b'\\' | b'?' => write!(out, "\\{}", b as char).unwrap(),
            0x20..=0x7E => out.push(b as char),
            _ => write!(out, "\\{b:03o}").unwrap(),
        }
    }
    out.push('"');
    out
}

fn conv_code(c: &Conv) -> String {
    let opt = |c: &Option<Box<Conv>>| c.as_ref().map_or("_".to_string(), |c| conv_code(c));
    match c {
        Conv::Fit(k) => format!("F{}", (b'a' + kind(k) as u8) as char),
        Conv::ToFloat(single) => if *single { "s" } else { "d" }.to_string(),
        Conv::Wrap(inner) => format!("W{}", opt(inner)),
        Conv::Inside(c) => format!("I{}", conv_code(c)),
        Conv::Elems(c) => format!("E{}", conv_code(c)),
        Conv::Tuple(cs) => {
            let parts: String = cs.iter().map(|c| c.as_ref().map_or("_".to_string(), conv_code)).collect();
            format!("T{}{parts}", (b'a' + cs.len() as u8) as char)
        }
        Conv::Res(ok, err) => format!("R{}{}", opt(ok), opt(err)),
    }
}

fn constant(v: &Value) -> Result<String, String> {
    Ok(match v {
        Value::Int(i) => match i64::try_from(*i) {
            Ok(i64::MIN) => "kint(INT64_MIN)".into(),
            Ok(small) => format!("kint({small}LL)"),
            Err(_) => format!("kint2(0x{:x}ULL, 0x{:x}ULL)", (*i as u128 >> 64) as u64, *i as u128 as u64),
        },
        // A number that is one is written in decimal, with the fewest
        // digits that give it back: the C compiler reads the same number.
        Value::Float(f) if f.is_finite() => format!("kf64({f:?})"),
        Value::Float(f) => format!("kf64_bits(0x{:016x}ULL)", f.to_bits()),
        // An `f32` is one of the numbers an `f64` holds: it is written as that one.
        Value::F32(f) if f.is_finite() => format!("kf32((float){:?})", *f as f64),
        Value::F32(f) => format!("kf32((float)kf64_bits(0x{:016x}ULL).u.f)", (*f as f64).to_bits()),
        Value::Bool(b) => format!("kbool({})", *b as u8),
        Value::Char(c) => format!("kchar({}u)", *c as u32),
        Value::Str(s) => format!("kstr({}, {})", quote(s), s.len()),
        Value::Unit => "KUNIT".into(),
        Value::None => "KNONE".into(),
        Value::Enum(e, variant, fields) if fields.is_empty() => format!("kagg(T_ENUM, {e}, {variant}, 0, NULL)"),
        Value::Fn(f) => format!("kfn({f})"),
        Value::Tuple(vs) => format!("kagg(T_TUPLE, 0, 0, {})", constants(vs)?),
        Value::Struct(i, vs) => format!("kagg(T_STRUCT, {i}, 0, {})", constants(vs)?),
        Value::Enum(e, variant, vs) => format!("kagg(T_ENUM, {e}, {variant}, {})", constants(vs)?),
        Value::Array(vs) => format!("kagg(T_ARRAY, 0, 0, {})", constants(vs)?),
        Value::Some(v) => format!("kbox(T_SOME, {})", constant(v)?),
        Value::Ok(v) => format!("kbox(T_OK, {})", constant(v)?),
        Value::Err(v) => format!("kbox(T_ERR, {})", constant(v)?),
        other => return Err(format!("the C backend cannot write the constant {other:?}")),
    })
}

/// The C type of a number an array lent to C holds.
fn buf_type(kind: &str) -> String {
    match kind {
        "f64" => "double".into(),
        "f32" => "float".into(),
        k => c_int(k),
    }
}

/// The C type of an integer type of the language.
fn c_int(kind: &str) -> String {
    match kind {
        "isize" => "ptrdiff_t".into(),
        "usize" => "size_t".into(),
        k if k.starts_with('u') => format!("uint{}_t", &k[1..]),
        k => format!("int{}_t", &k[1..]),
    }
}

/// The items of a constant, as the last arguments of `kagg`.
fn constants(vs: &[Value]) -> Result<String, String> {
    if vs.is_empty() {
        return Ok("0, NULL".into());
    }
    let parts: Result<Vec<String>, String> = vs.iter().map(constant).collect();
    Ok(format!("{}, (kv[]){{{}}}", vs.len(), parts?.join(", ")))
}

/// The projections of a place as a C array, and how many there are.
fn steps(cx: &Cx, p: &Place) -> Result<(usize, String), String> {
    let mut parts = Vec::new();
    for pr in &p.proj {
        parts.push(match pr {
            Proj::Field(k) => format!("{{0, {k}}}"),
            Proj::Index(Operand::Const(Value::Int(i))) => format!("{{1, kidxc({i})}}"),
            Proj::Index(Operand::Place(i)) if matches!(cx.rep(i), Rep::Int(_)) => format!("{{1, kidxc((i128){})}}", cx.name(i)),
            Proj::Index(Operand::Place(i)) if i.proj.is_empty() => format!("{{1, kidx(&L[{}])}}", i.local),
            Proj::Index(_) => return Err("the C backend needs an index to be a local or a constant".into()),
            Proj::Path(_) => return Err("the C backend cannot follow the result of a `ref mut` function here".into()),
        });
    }
    Ok((parts.len(), format!("(kstep[]){{{}}}", parts.join(", "))))
}

/// The arguments that tell the runtime where a place is.
fn target(cx: &Cx, p: &Place) -> Result<String, String> {
    if p.proj.is_empty() {
        return Ok(format!("&L[{}], 0, NULL", p.local));
    }
    let (n, s) = steps(cx, p)?;
    Ok(format!("&L[{}], {n}, {s}", p.local))
}

/// An expression that yields a new reference to the operand's value.
fn operand(cx: &Cx, op: &Operand) -> Result<String, String> {
    match op {
        Operand::Const(v) => constant(v),
        Operand::Place(p) if p.proj.is_empty() && cx.reps[p.local] != Rep::Boxed => Ok(cx.boxed(p.local)),
        Operand::Place(p) if part(&cx.reps, cx.recs, p).is_some() => Ok(tagged(cx.rep(p), &cx.name(p))),
        Operand::Place(p) if p.proj.is_empty() => Ok(format!("kretain(L[{}])", p.local)),
        Operand::Place(p) => {
            if let Some((base, i)) = cx.item(p, false)? {
                return Ok(format!("kitem({base}, {i})"));
            }
            if let Some((base, i, f)) = cx.field_item(p, false)? {
                return Ok(format!("kitem_field({base}, {i}, {f})"));
            }
            if let Some(ptr) = cx.ptr(p.local, &p.proj, false) {
                return Ok(format!("kretain(*{ptr})"));
            }
            if let Some((Proj::Path(l), before)) = p.proj.split_last() {
                if let Some(base) = cx.walk(p.local, before, false) {
                    return Ok(format!("kpath_get({base}, &L[{l}])"));
                }
            }
            let (n, s) = steps(cx, p)?;
            Ok(format!("kread(&L[{}], {n}, {s})", p.local))
        }
    }
}

fn list(cx: &Cx, ops: &[Operand]) -> Result<String, String> {
    if ops.is_empty() {
        return Ok("0, NULL".into());
    }
    let parts: Result<Vec<String>, String> = ops.iter().map(|o| operand(cx, o)).collect();
    Ok(format!("{}, (kv[]){{{}}}", ops.len(), parts?.join(", ")))
}

fn store(cx: &Cx, p: &Place, value: &str) -> Result<String, String> {
    if p.proj.is_empty() {
        return Ok(format!("kset(&L[{}], {value});", p.local));
    }
    // The value is computed first: it may read what the place is part of.
    if let Some((base, i)) = cx.item(p, true)? {
        return Ok(format!("{{ kv v_ = {value}; kitem_set({base}, {i}, v_); }}"));
    }
    if let Some((base, i, f)) = cx.field_item(p, true)? {
        return Ok(format!("{{ kv v_ = {value}; kitem_field_set({base}, {i}, {f}, v_); }}"));
    }
    if let Some((Proj::Path(l), before)) = p.proj.split_last() {
        if let Some(base) = cx.walk(p.local, before, true) {
            return Ok(format!("{{ kv v_ = {value}; kpath_set({base}, &L[{l}], v_); }}"));
        }
    }
    if let Some(ptr) = cx.ptr(p.local, &p.proj, true) {
        return Ok(format!("{{ kv v_ = {value}; kset({ptr}, v_); }}"));
    }
    let (n, s) = steps(cx, p)?;
    Ok(format!("kwrite(&L[{}], {n}, {s}, {value});", p.local))
}

fn binop(op: BinOp) -> &'static str {
    use BinOp::*;
    match op {
        Add => "O_ADD",
        Sub => "O_SUB",
        Mul => "O_MUL",
        Div => "O_DIV",
        Rem => "O_REM",
        AddWrap => "O_ADDW",
        SubWrap => "O_SUBW",
        MulWrap => "O_MULW",
        AddSat => "O_ADDS",
        SubSat => "O_SUBS",
        MulSat => "O_MULS",
        Shl => "O_SHL",
        Shr => "O_SHR",
        BitAnd => "O_AND",
        BitOr => "O_OR",
        BitXor => "O_XOR",
        Eq => "O_EQ",
        Ne => "O_NE",
        Lt => "O_LT",
        Le => "O_LE",
        Gt => "O_GT",
        Ge => "O_GE",
    }
}

fn rvalue(cx: &Cx, rv: &Rvalue) -> Result<String, String> {
    let o = |op: &Operand| operand(cx, op);
    Ok(match rv {
        Rvalue::Use(a) => o(a)?,
        Rvalue::Unary(op, n, a) => {
            let op = match op {
                UnOp::Neg => "U_NEG",
                UnOp::Not => "U_NOT",
                UnOp::BitNot => "U_BITNOT",
            };
            format!("kun({op}, {}, {})", num(*n), o(a)?)
        }
        Rvalue::Binary(op, n, a, b) => format!("kbin({}, {}, {}, {})", binop(*op), num(*n), o(a)?, o(b)?),
        Rvalue::CastInt(a, k) => format!("kcast_int({}, {})", o(a)?, kind(k)),
        Rvalue::CastFloat(a, single) => format!("kcast_float({}, {})", o(a)?, *single as u8),
        Rvalue::Convert(a, c) => format!("kconv({}, {})", o(a)?, quote(&conv_code(c))),
        Rvalue::Tuple(ops) => format!("kagg(T_TUPLE, 0, 0, {})", list(cx, ops)?),
        Rvalue::Struct(i, ops) => format!("kagg(T_STRUCT, {i}, 0, {})", list(cx, ops)?),
        Rvalue::Enum(e, v, ops) => format!("kagg(T_ENUM, {e}, {v}, {})", list(cx, ops)?),
        Rvalue::Array(ops) => format!("kagg(T_ARRAY, 0, 0, {})", list(cx, ops)?),
        Rvalue::Repeat(v, n) => format!("krepeat({}, {})", o(v)?, o(n)?),
        Rvalue::Some(a) => format!("kbox(T_SOME, {})", o(a)?),
        Rvalue::Ok(a) => format!("kbox(T_OK, {})", o(a)?),
        Rvalue::Err(a) => format!("kbox(T_ERR, {})", o(a)?),
        Rvalue::Is(a, tag) => format!("kis({}, {tag})", o(a)?),
        Rvalue::Len(a) => format!("klength({})", o(a)?),
        Rvalue::Push(p, a) => match (cx.ptr(p.local, &p.proj, true), cx.known_bool(a), cx.known_int(a)) {
            (Some(ptr), Some(t), _) => format!("kpush_bool({ptr}, {t})"),
            (Some(ptr), _, Some(x)) => format!("kpush_int({ptr}, (i128)({x}))"),
            (Some(ptr), _, _) if cx.known_f64(a).is_some() => format!("kpush_f64({ptr}, {})", cx.known_f64(a).unwrap_or_default()),
            (Some(ptr), _, _) if p.proj.iter().any(|pr| matches!(pr, Proj::Path(_))) => format!("kpush({ptr}, 0, NULL, {})", o(a)?),
            _ => format!("kpush({}, {})", target(cx, p)?, o(a)?),
        },
        Rvalue::Pop(p) => match cx.ptr(p.local, &p.proj, true) {
            Some(ptr) if p.proj.iter().any(|pr| matches!(pr, Proj::Path(_))) => format!("kpop({ptr}, 0, NULL)"),
            _ => format!("kpop({})", target(cx, p)?),
        },
        Rvalue::Get(a, i) => format!("kget({}, {})", o(a)?, o(i)?),
        Rvalue::Slice(b, r) => format!("kslice({}, {})", o(b)?, o(r)?),
        Rvalue::Chars(a) => format!("kchars({})", o(a)?),
        Rvalue::Const(i) => format!("kretain(KC[{i}])"),
        Rvalue::RangeStart(r, _) => format!("krange_start({})", o(r)?),
        Rvalue::RangeHas(r, i) => format!("krange_has({}, {})", o(r)?, o(i)?),
        Rvalue::RangeLast(r, i) => format!("krange_last({}, {})", o(r)?, o(i)?),
        Rvalue::Succ(v, n) => format!("ksucc({}, {})", o(v)?, num(*n)),
        Rvalue::Closure(f, ops) => format!("kclosure({f}, {})", list(cx, ops)?),
        Rvalue::Capture(k) => format!("kcapture(env, {k})"),
        Rvalue::Uninit | Rvalue::Moved => "KUNIT".into(),
        Rvalue::Format(_) => return Err("a formatted string is written as statements".into()),
    })
}

// ---- machine integers ----
//
// A local that only ever holds integers of one kind of 64 bits or less, or
// only booleans, is a plain C variable instead of a tagged value. Arithmetic
// on it is written inline; the tagged helpers of the runtime remain the
// slow path, which is where an overflow is reported.

/// How a local is stored.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Rep {
    /// Nothing is known yet.
    Top,
    /// Only integer literals so far: any integer kind fits.
    AnyInt,
    Int(&'static str),
    Bool,
    F64,
    /// A struct of plain fields, kept as a struct of C: its number.
    Rec(usize),
    /// An optional number or boolean, kept as whether it is there and the
    /// value: 1 a boolean, 2 an integer with sign, 3 an `f64`, 4 an
    /// integer without sign.
    Opt(u8),
    /// A tagged value, in `L`.
    Boxed,
}

/// The name of the struct of C a value is kept as, when it is kept as one.
fn agg(r: Rep) -> Option<String> {
    match r {
        Rep::Rec(i) => Some(format!("KS{i}")),
        Rep::Opt(k) => Some(format!("KO{}", ['B', 'I', 'F', 'U'][(k as usize - 1).min(3)])),
        _ => None,
    }
}

/// How what an optional holds is stored.
fn opt_inner(k: u8) -> Rep {
    match k {
        1 => Rep::Bool,
        3 => Rep::F64,
        4 => Rep::Int("u64"),
        _ => Rep::Int("i64"),
    }
}

/// The kind of optional that holds a value stored that way.
fn opt_of(r: Rep) -> Option<u8> {
    match r {
        Rep::Bool => Some(1),
        Rep::F64 => Some(3),
        Rep::Int(k) => Some(if k.starts_with('u') { 4 } else { 2 }),
        _ => None,
    }
}

/// How a part of a local kept as a struct of C is stored: a field of a
/// struct, or what an optional holds.
fn part(reps: &[Rep], recs: &[Vec<u8>], p: &Place) -> Option<Rep> {
    match (reps[p.local], p.proj.as_slice()) {
        (Rep::Rec(i), [Proj::Field(k)]) => {
            let kinds = kinds_of(recs, i);
            // The fields of an enum come after which variant it is.
            Some(field_rep(kinds[*k + (kinds[0] == 6) as usize]))
        }
        (Rep::Opt(k), [Proj::Field(0)]) => Some(opt_inner(k)),
        _ => None,
    }
}

/// The kinds of the fields of what is kept as a struct of C: a struct of
/// the program, or a tuple.
fn kinds_of(recs: &[Vec<u8>], i: usize) -> Vec<u8> {
    if i >= TUPLE_BASE { tuple_kinds(i) } else { recs[i].clone() }
}

/// How a field of a struct kept as a struct of C is stored, by the kind
/// the program gives for it: 1 a boolean, 2 an integer, 3 an `f64`.
fn field_rep(kind: u8) -> Rep {
    match kind {
        1 => Rep::Bool,
        3 => Rep::F64,
        5 => Rep::Int("u64"),
        _ => Rep::Int("i64"),
    }
}

struct Cx<'a> {
    /// The kinds of the fields of each struct an array keeps in line.
    recs: &'a [Vec<u8>],
    reps: Vec<Rep>,
    /// Temporaries a call takes as an argument and nothing else reads:
    /// the call takes the value itself, not a copy of it.
    spent: Vec<bool>,
}

/// The temporaries that are given a value once and read once, by a call
/// that always comes after. A copy kept in such a temporary would
/// make whatever it holds shared until the function returns, and the
/// next write to the original would copy all of it.
pub fn spent(f: &Func) -> Vec<bool> {
    fn through(p: &Place, uses: &mut [u32]) {
        for pr in &p.proj {
            match pr {
                Proj::Index(op) => read(op, uses),
                Proj::Path(l) => uses[*l] += 1,
                Proj::Field(_) => {}
            }
        }
    }
    fn read(op: &Operand, uses: &mut [u32]) {
        if let Operand::Place(p) = op {
            uses[p.local] += 1;
            through(p, uses);
        }
    }
    fn written(p: &Place, block: usize, uses: &mut [u32], defs: &mut [(u32, usize)]) {
        if p.proj.is_empty() {
            defs[p.local] = (defs[p.local].0 + 1, block);
        } else {
            uses[p.local] += 2;
            through(p, uses);
        }
    }
    let n = f.locals.max(1);
    let mut uses = vec![0u32; n];
    let mut defs = vec![(0u32, usize::MAX); n];
    let mut by_call = vec![usize::MAX; n];
    // Temporaries that hold the result of a call.
    let mut by_result = vec![false; n];
    for (k, b) in f.blocks.iter().enumerate() {
        for st in &b.stmts {
            written(&st.place, k, &mut uses, &mut defs);
            each_operand(&st.rv, &mut |op| read(op, &mut uses));
            if let Rvalue::Push(p, _) | Rvalue::Pop(p) = &st.rv {
                uses[p.local] += 2;
                through(p, &mut uses);
            }
        }
        match &b.term {
            Term::Call { callee, args, writeback, dest, .. } => {
                if let Callee::Indirect(op, _) = callee {
                    read(op, &mut uses);
                }
                for a in args {
                    read(a, &mut uses);
                    if let Operand::Place(p) = a {
                        if p.proj.is_empty() {
                            by_call[p.local] = k;
                        }
                    }
                }
                for (_, place) in writeback {
                    uses[place.local] += 2;
                    through(place, &mut uses);
                }
                // The result is stored after the arguments are read: it
                // is there from the block that follows.
                written(dest, k, &mut uses, &mut defs);
                if dest.proj.is_empty() {
                    by_result[dest.local] = true;
                }
            }
            Term::Branch(op, _, _) | Term::Return(op) => read(op, &mut uses),
            _ => {}
        }
    }
    let next = |k: usize| match &f.blocks[k].term {
        Term::Goto(t) => vec![*t],
        Term::Branch(_, a, b) => vec![*a, *b],
        Term::Call { next, .. } => vec![*next],
        _ => Vec::new(),
    };
    // Whether `target` can be reached from `start` without going through `avoid`.
    let reaches = |start: Vec<usize>, target: usize, avoid: usize| {
        let mut seen = vec![false; f.blocks.len()];
        let mut work = start;
        while let Some(b) = work.pop() {
            if b == avoid || std::mem::replace(&mut seen[b], true) {
                continue;
            }
            if b == target {
                return true;
            }
            work.extend(next(b));
        }
        false
    };
    (0..n)
        .map(|l| {
            let (given, used) = (defs[l].1, by_call[l]);
            let single = l >= f.params
                && f.names.get(l).is_none_or(|name| name.is_none())
                && !f.once.contains(&l)
                && uses[l] == 1
                && defs[l].0 == 1
                && given != usize::MAX
                && used != usize::MAX;
            if !single {
                return false;
            }
            if given == used {
                return !by_result[l];
            }
            // Given a value in one block and read by a call in another:
            // every way to the call must go through where the value is
            // given, and the call must not run again without it.
            !reaches(vec![0], used, given) && !reaches(next(used), used, given)
        })
        .collect()
}

fn rep_type(r: Rep) -> String {
    match r {
        Rep::Int(k) => ctype(k).to_string(),
        Rep::F64 => "double".to_string(),
        Rep::Boxed => "kv".to_string(),
        Rep::Rec(_) | Rep::Opt(_) => format!("struct {}", agg(r).unwrap_or_default()),
        _ => "bool".to_string(),
    }
}

/// A machine value as a tagged one.
fn tagged(r: Rep, value: &str) -> String {
    match r {
        Rep::Int(_) => format!("kint((i128)({value}))"),
        Rep::F64 => format!("kf64({value})"),
        Rep::Boxed => value.to_string(),
        Rep::Rec(_) | Rep::Opt(_) => format!("{}_box({value})", agg(r).unwrap_or_default()),
        _ => format!("kbool({value})"),
    }
}

/// The machine signature of a function, when all of its parameters and its
/// result are integers or booleans and it never changes a parameter (so
/// there is nothing to hand back to the caller).
fn direct_sig(f: &Func, fns: &[Func], recs: &[Vec<u8>]) -> Option<(Vec<Rep>, Rep)> {
    // A function called with its arguments as they are, instead of through
    // an array of tagged values: one that leaves its parameters alone (so
    // that a tagged argument can be lent to it, without counting it) and
    // that captures nothing.
    let ret = scalar_rep(f.ret);
    if f.scalars.len() != f.params {
        return None;
    }
    let reps = infer(f, fns, recs);
    let params: Vec<Rep> = reps[..f.params].to_vec();
    let changes = |rv: &Rvalue| match rv {
        Rvalue::Push(p, _) | Rvalue::Pop(p) => p.local < f.params,
        Rvalue::Capture(_) => true,
        _ => false,
    };
    let written = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|st| st.place.local < f.params || changes(&st.rv))
            || matches!(&b.term, Term::Call { dest, writeback, .. }
                if dest.local < f.params || writeback.iter().any(|(_, place)| place.local < f.params))
    });
    (!written).then_some((params, ret))
}

fn machine(k: &str) -> bool {
    matches!(k, "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize")
}

fn ctype(k: &str) -> &'static str {
    if k.starts_with('i') {
        "int64_t"
    } else {
        "uint64_t"
    }
}

/// A 128-bit constant that fits in 64 bits, as a C expression.
fn wide(v: i128) -> String {
    if v == i64::MIN as i128 {
        "((i128)INT64_MIN)".into()
    } else if v < 0 || v <= i64::MAX as i128 {
        format!("((i128){v}LL)")
    } else {
        format!("((i128){v}ULL)")
    }
}

/// The check that a result left the range of a kind narrower than its C type.
fn narrow(k: &str, r: &str) -> String {
    let (lo, hi) = bounds(k);
    if lo == i64::MIN as i128 || hi == u64::MAX as i128 {
        return String::new();
    }
    if lo == 0 {
        format!(" || {r} > {hi}ULL")
    } else {
        format!(" || {r} < {lo}LL || {r} > {hi}LL")
    }
}

fn scalar_rep(s: Option<Scalar>) -> Rep {
    match s {
        Some(Scalar::Int(k)) if machine(k) => Rep::Int(k),
        Some(Scalar::Bool) => Rep::Bool,
        Some(Scalar::F64) => Rep::F64,
        Some(Scalar::Rec(i)) => Rep::Rec(i),
        Some(Scalar::Opt(k)) => Rep::Opt(k),
        _ => Rep::Boxed,
    }
}

fn join(a: Rep, b: Rep) -> Rep {
    use Rep::*;
    match (a, b) {
        (Top, x) | (x, Top) => x,
        (AnyInt, Int(k)) | (Int(k), AnyInt) => Int(k),
        (x, y) if x == y => x,
        _ => Boxed,
    }
}

fn is_cmp(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}

/// What an rvalue produces, given what is known about the locals.
fn rep_of(rv: &Rvalue, reps: &[Rep], recs: &[Vec<u8>]) -> Rep {
    use Rep::*;
    let int = |n: &Num| match n {
        Num::Int(k) if machine(k) => Int(k),
        _ => Boxed,
    };
    match rv {
        Rvalue::Use(Operand::Const(Value::Int(i))) if i64::try_from(*i).is_ok() => AnyInt,
        Rvalue::Use(Operand::Const(Value::Bool(_))) => Bool,
        Rvalue::Use(Operand::Const(Value::Float(_))) => F64,
        Rvalue::Binary(op, Num::F64, _, _) if !is_cmp(*op) => F64,
        Rvalue::Unary(UnOp::Neg, Num::F64, _) | Rvalue::CastFloat(_, false) | Rvalue::Convert(_, Conv::ToFloat(false)) => F64,
        Rvalue::Use(Operand::Place(p)) if p.proj.is_empty() => reps[p.local],
        Rvalue::Use(Operand::Place(p)) if part(reps, recs, p).is_some() => part(reps, recs, p).unwrap_or(Boxed),
        // `None` fits any optional; `Some` of a number is kept with it.
        Rvalue::Use(Operand::Const(Value::None)) => Top,
        Rvalue::Some(Operand::Place(p)) if p.proj.is_empty() => match reps[p.local] {
            Top | AnyInt => Top,
            r => opt_of(r).map_or(Boxed, Opt),
        },
        // A struct of plain fields is made in place, without a block.
        Rvalue::Struct(i, ops) if !recs[*i].is_empty() && !recs[*i].contains(&4) && recs[*i].len() == ops.len() => Rec(*i),
        Rvalue::Binary(op, _, _, _) if is_cmp(*op) => Bool,
        Rvalue::Binary(_, n, _, _) | Rvalue::Unary(UnOp::Neg | UnOp::BitNot, n, _) => int(n),
        Rvalue::Unary(UnOp::Not, _, _) | Rvalue::Is(..) | Rvalue::RangeHas(..) | Rvalue::RangeLast(..) => Bool,
        Rvalue::CastInt(_, k) | Rvalue::Convert(_, Conv::Fit(k)) if machine(k) => Int(k),
        Rvalue::Len(_) => Int("usize"),
        Rvalue::RangeStart(_, n) | Rvalue::Succ(_, n) => int(n),
        Rvalue::Uninit | Rvalue::Moved => Top,
        _ => Boxed,
    }
}

/// Decides how every local of a function is stored.
fn infer(f: &Func, fns: &[Func], recs: &[Vec<u8>]) -> Vec<Rep> {
    let mut reps = vec![Rep::Top; f.locals];
    for l in 0..f.params.min(f.locals) {
        reps[l] = scalar_rep(f.scalars.get(l).copied().flatten());
    }
    // A variable whose type is an integer or a boolean is stored as one,
    // whatever its value is read from.
    let mut typed = vec![false; f.locals];
    for l in f.params..f.locals {
        let r = scalar_rep(f.hints.get(l).copied().flatten());
        if r != Rep::Boxed {
            reps[l] = r;
            typed[l] = true;
        }
    }
    // A local with fields or items is a tagged value.
    fn projected(op: &Operand, reps: &mut [Rep]) {
        if let Operand::Place(p) = op {
            through(p, reps);
        }
    }
    fn through(p: &Place, reps: &mut [Rep]) {
        // A part of a struct of C is read and written where it is.
        let whole = matches!((reps[p.local], p.proj.as_slice()), (Rep::Rec(_), [Proj::Field(_)]) | (Rep::Opt(_), [Proj::Field(0)]));
        if !p.proj.is_empty() && !whole {
            reps[p.local] = Rep::Boxed;
        }
        for pr in &p.proj {
            match pr {
                Proj::Index(i) => projected(i, reps),
                // The steps are an array, read where it is kept.
                Proj::Path(l) => reps[*l] = Rep::Boxed,
                Proj::Field(_) => {}
            }
        }
    }
    for b in &f.blocks {
        for st in &b.stmts {
            through(&st.place, &mut reps);
            each_operand(&st.rv, &mut |op| projected(op, &mut reps));
            if let Rvalue::Push(p, _) | Rvalue::Pop(p) = &st.rv {
                reps[p.local] = Rep::Boxed;
            }
        }
        match &b.term {
            Term::Call { args, writeback, dest, .. } => {
                args.iter().for_each(|a| projected(a, &mut reps));
                through(dest, &mut reps);
                for (_, place) in writeback {
                    // A whole struct of C goes to the call as a block
                    // made for it, and is read back from what the call leaves.
                    if !(place.proj.is_empty() && matches!(reps[place.local], Rep::Rec(_) | Rep::Opt(_))) {
                        reps[place.local] = Rep::Boxed;
                    }
                }
            }
            Term::Branch(op, _, _) | Term::Return(op) => projected(op, &mut reps),
            _ => {}
        }
    }
    // A parameter kept as a struct of C is read from the block of the
    // caller and never handed back: one that is written to stays tagged.
    for b in &f.blocks {
        let mut written = |l: usize| {
            if l < f.params && matches!(reps[l], Rep::Rec(_) | Rep::Opt(_)) {
                reps[l] = Rep::Boxed;
            }
        };
        b.stmts.iter().for_each(|st| written(st.place.local));
        // What a call hands back to it is written too.
        if let Term::Call { dest, writeback, .. } = &b.term {
            written(dest.local);
            writeback.iter().for_each(|(_, place)| written(place.local));
        }
    }
    loop {
        let before = reps.clone();
        for b in &f.blocks {
            for st in &b.stmts {
                if st.place.proj.is_empty() && !typed[st.place.local] {
                    let l = st.place.local;
                    reps[l] = join(reps[l], rep_of(&st.rv, &reps, recs));
                }
            }
            if let Term::Call { callee, dest, .. } = &b.term {
                let produced = match callee {
                    Callee::Fn(g) => scalar_rep(fns[*g].ret),
                    _ => Rep::Boxed,
                };
                if !typed[dest.local] {
                    reps[dest.local] = join(reps[dest.local], produced);
                }
            }
        }
        if reps == before {
            break;
        }
    }
    reps.iter()
        .map(|r| match r {
            Rep::Top => Rep::Boxed,
            Rep::AnyInt => Rep::Int("i64"),
            other => *other,
        })
        .collect()
}

/// Calls `visit` with every operand an rvalue reads.
pub fn each_operand(rv: &Rvalue, visit: &mut dyn FnMut(&Operand)) {
    use Rvalue::*;
    match rv {
        Use(a) | Unary(_, _, a) | CastInt(a, _) | CastFloat(a, _) | Convert(a, _) | Some(a) | Ok(a) | Err(a) | Is(a, _)
        | Len(a) | Chars(a) | RangeStart(a, _) | Succ(a, _) | Push(_, a) => visit(a),
        Binary(_, _, a, b) | Repeat(a, b) | Slice(a, b) | RangeHas(a, b) | RangeLast(a, b) | Get(a, b) => {
            visit(a);
            visit(b);
        }
        Tuple(ops) | Struct(_, ops) | Enum(_, _, ops) | Array(ops) | Closure(_, ops) => ops.iter().for_each(visit),
        Format(parts) => parts.iter().for_each(|part| {
            if let FmtPart::Val(o) = part {
                visit(o)
            }
        }),
        Pop(_) | Const(_) | Capture(_) | Uninit | Moved => {}
    }
}

impl Cx<'_> {
    fn rep(&self, p: &Place) -> Rep {
        if p.proj.is_empty() {
            return self.reps[p.local];
        }
        part(&self.reps, self.recs, p).unwrap_or(Rep::Boxed)
    }

    /// The C name of a place that holds a machine value: a local, or a
    /// field of a struct of C.
    fn name(&self, p: &Place) -> String {
        match (self.reps[p.local], p.proj.as_slice()) {
            (Rep::Opt(_), [Proj::Field(_)]) => format!("s{}.v", p.local),
            (Rep::Rec(i), [Proj::Field(k)]) if kinds_of(self.recs, i)[0] == 6 => format!("s{}.f{}", p.local, k + 1),
            (_, [Proj::Field(k)]) => format!("s{}.f{k}", p.local),
            _ => format!("s{}", p.local),
        }
    }

    /// A C pointer to the tagged value a place holds.
    fn ptr(&self, local: usize, proj: &[Proj], write: bool) -> Option<String> {
        if self.reps[local] != Rep::Boxed {
            return None;
        }
        // A place that ends in steps only known at run time may be a bare
        // item of a packed array: it is read without taking its address.
        if !write && matches!(proj.last(), Some(Proj::Path(_))) {
            return None;
        }
        self.walk(local, proj, write)
    }

    /// An expression that takes the value out of a place and leaves the
    /// place empty, when the place is a local or a field of fields of one.
    fn moved(&self, p: &Place) -> Option<String> {
        if !p.proj.iter().all(|pr| matches!(pr, Proj::Field(_))) {
            return None;
        }
        self.walk(p.local, &p.proj, true).map(|ptr| format!("ktake({ptr})"))
    }

    /// A C pointer to a value that has parts: it is never a bare item.
    fn walk(&self, local: usize, proj: &[Proj], write: bool) -> Option<String> {
        if self.reps[local] != Rep::Boxed {
            return None;
        }
        let mut s = format!("&L[{local}]");
        for pr in proj {
            match pr {
                Proj::Path(l) => s = format!("kpath({s}, &L[{l}], {write})"),
                Proj::Field(k) => s = format!("kfield({s}, {k}, {write})"),
                Proj::Index(i) => match self.int(i) {
                    Ok(i) => s = format!("kitemp({s}, (i128)({i}), {write})"),
                    Err(_) => return None,
                },
            }
        }
        Some(s)
    }

    /// The array, the position and the field of a place that ends in a
    /// field of an item: `xs[i].f`.
    fn field_item(&self, p: &Place, write: bool) -> Result<Option<(String, String, usize)>, String> {
        let [before @ .., Proj::Index(i), Proj::Field(f)] = p.proj.as_slice() else {
            return Ok(None);
        };
        let Some(base) = self.walk(p.local, before, write) else {
            return Ok(None);
        };
        Ok(Some((base, format!("(i128)({})", self.int(i)?), *f)))
    }

    /// The array and the position of a place that ends in an index.
    fn item(&self, p: &Place, write: bool) -> Result<Option<(String, String)>, String> {
        let Some((Proj::Index(i), before)) = p.proj.split_last() else {
            return Ok(None);
        };
        let Some(base) = self.ptr(p.local, before, write) else {
            return Ok(None);
        };
        Ok(Some((base, format!("(i128)({})", self.int(i)?))))
    }

    /// The operand as a C integer, when it is known to be one without
    /// looking at a tag.
    fn known_int(&self, op: &Operand) -> Option<String> {
        match op {
            Operand::Const(Value::Int(i)) if i64::try_from(*i).is_ok() || u64::try_from(*i).is_ok() => Some(wide(*i)),
            Operand::Place(p) if matches!(self.rep(p), Rep::Int(_)) => Some(self.name(p)),
            _ => None,
        }
    }

    /// The operand as a C boolean, when it is known to be one without
    /// looking at a tag.
    fn known_bool(&self, op: &Operand) -> Option<String> {
        match op {
            Operand::Const(Value::Bool(b)) => Some((*b as u8).to_string()),
            Operand::Place(p) if self.rep(p) == Rep::Bool => Some(self.name(p)),
            _ => None,
        }
    }

    /// The operand as a C double, when it is known to be one without
    /// looking at a tag.
    fn known_f64(&self, op: &Operand) -> Option<String> {
        match op {
            Operand::Const(Value::Float(f)) if f.is_finite() => Some(format!("({f:?})")),
            Operand::Const(Value::Float(f)) => Some(format!("kdbl(0x{:016x}ULL)", f.to_bits())),
            Operand::Place(p) if self.rep(p) == Rep::F64 => Some(self.name(p)),
            _ => None,
        }
    }

    /// The operand as a C double, where the types say it is an `f64`.
    fn float(&self, op: &Operand) -> Result<String, String> {
        if let Some(known) = self.known_f64(op) {
            return Ok(known);
        }
        match op {
            Operand::Place(p) if p.proj.is_empty() && self.reps[p.local] == Rep::Boxed => Ok(format!("L[{}].u.f", p.local)),
            Operand::Place(p) => {
                if let Some((base, i)) = self.item(p, false)? {
                    return Ok(format!("kitem_f64({base}, {i})"));
                }
                if let Some((base, i, f)) = self.field_item(p, false)? {
                    return Ok(format!("kitem_field_f64({base}, {i}, {f})"));
                }
                match self.ptr(p.local, &p.proj, false) {
                    Some(ptr) => Ok(format!("{ptr}->u.f")),
                    None => Ok(format!("({}).u.f", operand(self, op)?)),
                }
            }
            _ => Ok(format!("({}).u.f", operand(self, op)?)),
        }
    }

    /// An rvalue as a C double.
    fn f64_rvalue(&self, rv: &Rvalue) -> Result<String, String> {
        Ok(match rv {
            Rvalue::Use(a) => self.float(a)?,
            Rvalue::Uninit | Rvalue::Moved => "0".into(),
            Rvalue::Binary(op @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div), Num::F64, a, b) => {
                let c = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    _ => "/",
                };
                format!("({} {c} {})", self.float(a)?, self.float(b)?)
            }
            Rvalue::Unary(UnOp::Neg, Num::F64, a) => format!("(-{})", self.float(a)?),
            Rvalue::CastFloat(a, false) | Rvalue::Convert(a, Conv::ToFloat(false)) if self.known_int(a).is_some() => {
                format!("(double)({})", self.known_int(a).unwrap_or_default())
            }
            _ => format!("({}).u.f", rvalue(self, rv)?),
        })
    }

    /// The operand as a C integer, where the types say it is an integer.
    fn int(&self, op: &Operand) -> Result<String, String> {
        if let Some(known) = self.known_int(op) {
            return Ok(known);
        }
        match op {
            Operand::Place(p) if p.proj.is_empty() && self.reps[p.local] == Rep::Boxed => Ok(format!("L[{}].u.i", p.local)),
            Operand::Place(p) => {
                if let Some((base, i)) = self.item(p, false)? {
                    return Ok(format!("kitem_int({base}, {i})"));
                }
                if let Some((base, i, f)) = self.field_item(p, false)? {
                    return Ok(format!("kitem_field_int({base}, {i}, {f})"));
                }
                match self.ptr(p.local, &p.proj, false) {
                    Some(ptr) => Ok(format!("{ptr}->u.i")),
                    None => Ok(format!("({}).u.i", operand(self, op)?)),
                }
            }
            _ => Ok(format!("({}).u.i", operand(self, op)?)),
        }
    }

    fn truth(&self, op: &Operand) -> Result<String, String> {
        match op {
            Operand::Const(Value::Bool(b)) => Ok((*b as u8).to_string()),
            Operand::Place(p) if self.rep(p) == Rep::Bool => Ok(self.name(p)),
            Operand::Place(p) if !p.proj.is_empty() => {
                if let Some((base, i)) = self.item(p, false)? {
                    return Ok(format!("kitem_bool({base}, {i})"));
                }
                if let Some((base, i, f)) = self.field_item(p, false)? {
                    return Ok(format!("kitem_field_bool({base}, {i}, {f})"));
                }
                match self.ptr(p.local, &p.proj, false) {
                    Some(ptr) => Ok(format!("ktruth(*{ptr})")),
                    None => Ok(format!("ktruth({})", operand(self, op)?)),
                }
            }
            _ => Ok(format!("ktruth({})", operand(self, op)?)),
        }
    }

    /// An rvalue as a C integer of kind `k`.
    fn int_rvalue(&self, rv: &Rvalue, k: &'static str) -> Result<String, String> {
        let t = ctype(k);
        let unboxed = |boxed: String| format!("({t})({boxed}).u.i");
        let slow = |cx: &Cx| Ok::<String, String>(unboxed(rvalue(cx, rv)?));
        Ok(match rv {
            Rvalue::Use(a) => format!("({t})({})", self.int(a)?),
            Rvalue::Uninit | Rvalue::Moved => "0".into(),
            Rvalue::Len(Operand::Place(p)) if self.ptr(p.local, &p.proj, false).is_some() => {
                format!("({t})klen_at({})", self.ptr(p.local, &p.proj, false).unwrap_or_default())
            }
            Rvalue::Binary(op, Num::Int(kind), a, b) if *kind == k => {
                let (x, y) = (self.int(a)?, self.int(b)?);
                // The slow path redoes the operation on tagged values, and
                // reports the overflow or the division by zero.
                let again = unboxed(format!("kbin({}, {}, kint((i128)(x_)), kint((i128)(y_)))", binop(*op), num(Num::Int(k))));
                let builtin = match op {
                    BinOp::Add => Some("add"),
                    BinOp::Sub => Some("sub"),
                    BinOp::Mul => Some("mul"),
                    _ => None,
                };
                let head = format!("{t} x_ = ({t})({x}), y_ = ({t})({y}), r_;");
                match (builtin, op) {
                    (Some(name), _) => {
                        format!("({{ {head} (__builtin_{name}_overflow(x_, y_, &r_){}) ? {again} : r_; }})", narrow(k, "r_"))
                    }
                    (None, BinOp::Div | BinOp::Rem) => {
                        let c = if *op == BinOp::Div { "/" } else { "%" };
                        let risky = if t == "int64_t" { "y_ == 0 || y_ == -1" } else { "y_ == 0" };
                        format!("({{ {head} ({risky}) ? {again} : (r_ = x_ {c} y_, r_); }})")
                    }
                    _ => format!("({{ {head} (void)r_; {again}; }})"),
                }
            }
            Rvalue::Succ(a, Num::Int(kind)) if *kind == k => {
                let x = self.int(a)?;
                let again = unboxed(format!("ksucc(kint((i128)(x_)), {})", num(Num::Int(k))));
                format!("({{ {t} x_ = ({t})({x}), r_; (__builtin_add_overflow(x_, 1, &r_){}) ? {again} : r_; }})", narrow(k, "r_"))
            }
            Rvalue::CastInt(a, kind) if *kind == k => match self.known_int(a) {
                Some(x) => {
                    let (lo, hi) = bounds(k);
                    let again = unboxed(format!("kcast_int(kint(v_), {})", self::kind(k)));
                    format!("({{ i128 v_ = (i128)({x}); (v_ < {} || v_ > {}) ? {again} : ({t})v_; }})", wide(lo), wide(hi))
                }
                None => slow(self)?,
            },
            _ => slow(self)?,
        })
    }

    /// An rvalue as a C boolean.
    fn bool_rvalue(&self, rv: &Rvalue) -> Result<String, String> {
        Ok(match rv {
            Rvalue::Use(a) => self.truth(a)?,
            Rvalue::Uninit | Rvalue::Moved => "0".into(),
            Rvalue::Unary(UnOp::Not, _, a) => format!("!{}", self.truth(a)?),
            // Whether an optional kept as a struct of C holds a value.
            Rvalue::Is(Operand::Place(p), tag) if matches!(self.rep(p), Rep::Opt(_)) => {
                format!("({}s{}.has)", if *tag == 1 { "" } else { "!" }, p.local)
            }
            // Which variant an enum kept as a struct of C is.
            Rvalue::Is(Operand::Place(p), tag) if p.proj.is_empty() && matches!(self.rep(p), Rep::Rec(_)) => {
                format!("(s{}.f0 == {tag})", p.local)
            }
            // A character against one that is written out: the common
            // case of reading text, done without the general comparison.
            Rvalue::Binary(op, _, Operand::Place(p), Operand::Const(Value::Char(c)))
                if is_cmp(*op) && p.proj.is_empty() && self.reps[p.local] == Rep::Boxed =>
            {
                let cmp = match op {
                    BinOp::Eq => "==",
                    BinOp::Ne => "!=",
                    BinOp::Lt => "<",
                    BinOp::Le => "<=",
                    BinOp::Gt => ">",
                    _ => ">=",
                };
                let l = p.local;
                format!("(L[{l}].tag == T_CHAR ? L[{l}].u.c {cmp} {}u : ktruth({}))", *c as u32, rvalue(self, rv)?)
            }
            Rvalue::Binary(op, _, a, b) if is_cmp(*op) => match (self.known_int(a), self.known_int(b)) {
                (Some(x), Some(y)) => {
                    let c = match op {
                        BinOp::Eq => "==",
                        BinOp::Ne => "!=",
                        BinOp::Lt => "<",
                        BinOp::Le => "<=",
                        BinOp::Gt => ">",
                        _ => ">=",
                    };
                    format!("((i128)({x}) {c} (i128)({y}))")
                }
                _ => match (self.known_f64(a), self.known_f64(b)) {
                    (Some(x), Some(y)) => {
                        let c = match op {
                            BinOp::Eq => "==",
                            BinOp::Ne => "!=",
                            BinOp::Lt => "<",
                            BinOp::Le => "<=",
                            BinOp::Gt => ">",
                            _ => ">=",
                        };
                        format!("({x} {c} {y})")
                    }
                    _ => format!("ktruth({})", rvalue(self, rv)?),
                },
            },
            // The range stays where it is: only its upper bound is read.
            Rvalue::RangeHas(Operand::Place(r), i) | Rvalue::RangeLast(Operand::Place(r), i)
                if r.proj.is_empty() && self.reps[r.local] == Rep::Boxed && self.known_int(i).is_some() =>
            {
                let helper = if matches!(rv, Rvalue::RangeHas(..)) { "krange_has_int" } else { "krange_last_int" };
                format!("{helper}(&L[{}], (i128)({}))", r.local, self.known_int(i).unwrap_or_default())
            }
            _ => format!("ktruth({})", rvalue(self, rv)?),
        })
    }

    /// The statement that gives a local its value.
    fn assign(&self, place: &Place, rv: &Rvalue) -> Result<String, String> {
        match self.rep(place) {
            Rep::Int(k) => Ok(format!("{} = {};", self.name(place), self.int_rvalue(rv, k)?)),
            Rep::Bool => Ok(format!("{} = {};", self.name(place), self.bool_rvalue(rv)?)),
            Rep::F64 => Ok(format!("{} = {};", self.name(place), self.f64_rvalue(rv)?)),
            Rep::Opt(k) => {
                let l = place.local;
                let name = agg(Rep::Opt(k)).unwrap_or_default();
                match rv {
                    Rvalue::Some(op) => Ok(format!("s{l} = (struct {name}){{true, {}}};", self.machine(opt_inner(k), op)?)),
                    Rvalue::Use(Operand::Const(Value::None)) | Rvalue::Uninit | Rvalue::Moved => Ok(format!("s{l} = (struct {name}){{0}};")),
                    Rvalue::Use(Operand::Place(q)) if self.rep(q) == Rep::Opt(k) => Ok(format!("s{l} = s{};", q.local)),
                    // What an array gives is read where it is, with no
                    // block made for the optional.
                    Rvalue::Get(..) | Rvalue::Pop(_) => {
                        let found = match rv {
                            Rvalue::Get(a, i) => format!("kget_raw({}, {}, &it_)", operand(self, a)?, operand(self, i)?),
                            Rvalue::Pop(p) => match self.ptr(p.local, &p.proj, true) {
                                Some(ptr) if p.proj.iter().any(|pr| matches!(pr, Proj::Path(_))) => format!("kpop_raw({ptr}, 0, NULL, &it_)"),
                                _ => format!("kpop_raw({}, &it_)", target(self, p)?),
                            },
                            _ => String::new(),
                        };
                        let item = match k {
                            1 => "it_.u.t",
                            3 => "it_.u.f",
                            4 => "(uint64_t)it_.u.i",
                            _ => "(int64_t)it_.u.i",
                        };
                        Ok(format!("{{ kv it_; if ({found}) s{l} = (struct {name}){{true, {item}}}; else s{l} = (struct {name}){{0}}; }}"))
                    }
                    _ => Ok(format!("s{l} = {name}_take({});", rvalue(self, rv)?)),
                }
            }
            Rep::Rec(i) => {
                let l = place.local;
                match rv {
                    Rvalue::Struct(j, ops) if *j == i && ops.len() == self.recs[i].len() => {
                        let mut parts = Vec::new();
                        for (op, kind) in ops.iter().zip(&self.recs[i]) {
                            parts.push(self.machine(field_rep(*kind), op)?);
                        }
                        Ok(format!("s{l} = (struct KS{i}){{{}}};", parts.join(", ")))
                    }
                    Rvalue::Enum(_, v, ops) if kinds_of(self.recs, i)[0] == 6 => {
                        let kinds = kinds_of(self.recs, i);
                        let mut parts = vec![v.to_string()];
                        for (op, kind) in ops.iter().zip(&kinds[1..]) {
                            parts.push(self.machine(field_rep(*kind), op)?);
                        }
                        Ok(format!("s{l} = (struct KS{i}){{{}}};", parts.join(", ")))
                    }
                    Rvalue::Use(Operand::Const(Value::Enum(_, v, items))) if items.is_empty() && kinds_of(self.recs, i)[0] == 6 => {
                        Ok(format!("s{l} = (struct KS{i}){{{v}}};"))
                    }
                    Rvalue::Tuple(ops) if i >= TUPLE_BASE && ops.len() == tuple_kinds(i).len() => {
                        let mut parts = Vec::new();
                        for (op, kind) in ops.iter().zip(&tuple_kinds(i)) {
                            parts.push(self.machine(field_rep(*kind), op)?);
                        }
                        Ok(format!("s{l} = (struct KS{i}){{{}}};", parts.join(", ")))
                    }
                    Rvalue::Use(Operand::Place(q)) if self.rep(q) == Rep::Rec(i) => Ok(format!("s{l} = s{};", q.local)),
                    Rvalue::Uninit | Rvalue::Moved => Ok(format!("s{l} = (struct KS{i}){{0}};")),
                    _ => Ok(format!("s{l} = KS{i}_take({});", rvalue(self, rv)?)),
                }
            }
            _ => {
                // A machine value goes into an array without being tagged.
                if let (Rvalue::Use(op), Some((base, i))) = (rv, self.item(place, true)?) {
                    if let Some(t) = self.known_bool(op) {
                        return Ok(format!("kitem_set_bool({base}, {i}, {t});"));
                    }
                    if let Some(x) = self.known_int(op) {
                        return Ok(format!("kitem_set_int({base}, {i}, (i128)({x}));"));
                    }
                    if let Some(x) = self.known_f64(op) {
                        return Ok(format!("kitem_set_f64({base}, {i}, {x});"));
                    }
                }
                if let (Rvalue::Use(op), Some((base, i, f))) = (rv, self.field_item(place, true)?) {
                    if let Some(t) = self.known_bool(op) {
                        return Ok(format!("kitem_field_set_bool({base}, {i}, {f}, {t});"));
                    }
                    if let Some(x) = self.known_int(op) {
                        return Ok(format!("kitem_field_set_int({base}, {i}, {f}, (i128)({x}));"));
                    }
                    if let Some(x) = self.known_f64(op) {
                        return Ok(format!("kitem_field_set_f64({base}, {i}, {f}, {x});"));
                    }
                }
                store(self, place, &rvalue(self, rv)?)
            }
        }
    }

    /// Stores a tagged value the runtime produced.
    fn take(&self, place: &Place, value: &str) -> Result<String, String> {
        match self.rep(place) {
            Rep::Int(k) => Ok(format!("{} = ({})({value}).u.i;", self.name(place), ctype(k))),
            Rep::Bool => Ok(format!("{} = ktruth({value});", self.name(place))),
            Rep::F64 => Ok(format!("{} = ({value}).u.f;", self.name(place))),
            r @ (Rep::Rec(_) | Rep::Opt(_)) => Ok(format!("s{} = {}_take({value});", place.local, agg(r).unwrap_or_default())),
            _ => store(self, place, value),
        }
    }

    /// A machine value of the given representation from an operand.
    fn machine(&self, r: Rep, op: &Operand) -> Result<String, String> {
        match r {
            Rep::Int(k) => Ok(format!("({})({})", ctype(k), self.int(op)?)),
            Rep::F64 => self.float(op),
            Rep::Rec(_) | Rep::Opt(_) => match op {
                Operand::Place(p) if self.rep(p) == r => Ok(format!("s{}", p.local)),
                Operand::Const(Value::None) => Ok(format!("(struct {}){{0}}", agg(r).unwrap_or_default())),
                _ => Ok(format!("{}_take({})", agg(r).unwrap_or_default(), operand(self, op)?)),
            },
            _ => self.truth(op),
        }
    }

    /// Stores a machine value a direct call produced.
    fn take_machine(&self, place: &Place, r: Rep, value: &str) -> Result<String, String> {
        match (self.rep(place), r) {
            (Rep::Int(k), Rep::Int(_)) => Ok(format!("{} = ({})({value});", self.name(place), ctype(k))),
            (Rep::Bool, Rep::Bool) | (Rep::F64, Rep::F64) => Ok(format!("{} = {value};", self.name(place))),
            (Rep::Rec(_) | Rep::Opt(_), got) if got == self.rep(place) => Ok(format!("s{} = {value};", place.local)),
            _ => self.take(place, &tagged(r, value)),
        }
    }

    /// A local as a tagged value.
    fn boxed(&self, l: usize) -> String {
        match self.reps[l] {
            Rep::Int(_) => format!("kint((i128)s{l})"),
            Rep::Bool => format!("kbool(s{l})"),
            Rep::F64 => format!("kf64(s{l})"),
            r @ (Rep::Rec(_) | Rep::Opt(_)) => format!("{}_box(s{l})", agg(r).unwrap_or_default()),
            _ => format!("L[{l}]"),
        }
    }
}

/// The C declaration of the machine version of a function.
fn direct_head(i: usize, params: &[Rep], ret: Rep) -> String {
    let ps: Vec<String> = params.iter().enumerate().map(|(l, r)| format!("{} s{l}", rep_type(*r))).collect();
    format!("static {} g{i}({})", rep_type(ret), if ps.is_empty() { "void".to_string() } else { ps.join(", ") })
}

fn function(out: &mut String, i: usize, f: &Func, fns: &[Func], externs: &[Extern], direct: &[Option<(Vec<Rep>, Rep)>], recs: &[Vec<u8>]) -> Result<(), String> {
    let cx = &Cx { recs, reps: infer(f, fns, recs), spent: spent(f) };
    let n = f.locals.max(1);
    let mine = direct[i].as_ref();
    // The name of the function in the program, for whoever reads the C.
    writeln!(out, "/* {} */", f.name.replace("*/", "* /")).unwrap();
    match mine {
        Some((params, ret)) => writeln!(out, "{} {{", direct_head(i, params, *ret)).unwrap(),
        None => writeln!(out, "static kv f{i}(kv *a, kv env) {{").unwrap(),
    }
    writeln!(out, "    kv L[{n}];").unwrap();
    for (l, rep) in cx.reps.iter().enumerate() {
        match rep {
            _ if l < f.params && mine.is_some() => {}
            Rep::Int(k) if l < f.params => writeln!(out, "    {} s{l} = ({})a[{l}].u.i;", ctype(k), ctype(k)).unwrap(),
            Rep::Int(k) => writeln!(out, "    {} s{l} = 0;", ctype(k)).unwrap(),
            Rep::Bool if l < f.params => writeln!(out, "    bool s{l} = a[{l}].u.t;").unwrap(),
            Rep::Bool => writeln!(out, "    bool s{l} = false;").unwrap(),
            Rep::F64 if l < f.params => writeln!(out, "    double s{l} = a[{l}].u.f;").unwrap(),
            Rep::F64 => writeln!(out, "    double s{l} = 0;").unwrap(),
            // The caller keeps the block the argument came in.
            r @ (Rep::Rec(_) | Rep::Opt(_)) => {
                let name = agg(*r).unwrap_or_default();
                if l < f.params {
                    writeln!(out, "    struct {name} s{l} = {name}_get(a[{l}]);").unwrap();
                } else {
                    writeln!(out, "    struct {name} s{l} = {{0}};").unwrap();
                }
            }
            _ => {}
        }
    }
    writeln!(out, "    kenter();").unwrap();
    if mine.is_none() {
        writeln!(out, "    (void)env;").unwrap();
    }
    // The slots of machine locals stay empty, so that releasing them is free.
    for l in 0..n {
        if cx.reps.get(l).is_some_and(|r| *r != Rep::Boxed) {
            continue;
        }
        let init = match (l < f.params, mine.is_some()) {
            // Lent by the caller: it is not released here.
            (true, true) => format!("s{l}"),
            (true, false) => format!("a[{l}]"),
            _ => "KUNIT".to_string(),
        };
        writeln!(out, "    L[{l}] = {init};").unwrap();
    }
    writeln!(out, "    goto b0;").unwrap();
    for (b, block) in f.blocks.iter().enumerate() {
        writeln!(out, "b{b}:;").unwrap();
        // Nothing inside a block moves the position but the block itself.
        let mut at = None;
        let mut mark = |out: &mut String, span: crate::lexer::Span| {
            if at != Some((span.line, span.col)) {
                at = Some((span.line, span.col));
                writeln!(out, "    KL({}, {});", span.line, ((i + 1) << 12) | (span.col as usize).min(4095)).unwrap();
            }
        };
        for st in &block.stmts {
            mark(out, st.span);
            if let Rvalue::Format(parts) = &st.rv {
                writeln!(out, "    {{").unwrap();
                writeln!(out, "        ksb sb = {{0}};").unwrap();
                for part in parts {
                    match part {
                        FmtPart::Lit(s) => writeln!(out, "        ksb_lit(&sb, {}, {});", quote(s), s.len()).unwrap(),
                        FmtPart::Val(op) => writeln!(out, "        ksb_val(&sb, {});", operand(cx, op)?).unwrap(),
                    }
                }
                writeln!(out, "        {}", cx.take(&st.place, "ksb_done(&sb)")?).unwrap();
                writeln!(out, "    }}").unwrap();
            } else {
                writeln!(out, "    {}", cx.assign(&st.place, &st.rv)?).unwrap();
            }
        }
        mark(out, block.span);
        match &block.term {
            Term::Goto(t) => writeln!(out, "    goto b{t};").unwrap(),
            Term::Branch(c, yes, no) => writeln!(out, "    if ({}) goto b{yes}; else goto b{no};", cx.truth(c)?).unwrap(),
            Term::Call { callee, args, writeback, dest, next } => {
                writeln!(out, "    {{").unwrap();
                let fast = match callee {
                    Callee::Fn(g) => direct[*g].as_ref().filter(|(params, _)| params.len() == args.len()),
                    _ => None,
                };
                if let (Some((params, ret)), Callee::Fn(g)) = (fast, callee) {
                    let mut values = Vec::new();
                    let mut temps = Vec::new();
                    // Temporaries only this call reads: let go of after it.
                    let mut done = Vec::new();
                    for (k, (r, a)) in params.iter().zip(args).enumerate() {
                        match (r, a) {
                            // A tagged local is lent as it is.
                            (Rep::Boxed, Operand::Place(p)) if p.proj.is_empty() && cx.reps[p.local] == Rep::Boxed => {
                                values.push(format!("L[{}]", p.local));
                                if cx.spent[p.local] {
                                    done.push(p.local);
                                }
                            }
                            (Rep::Boxed, _) => {
                                writeln!(out, "        kv t{k} = {};", operand(cx, a)?).unwrap();
                                values.push(format!("t{k}"));
                                temps.push(k);
                            }
                            _ => values.push(cx.machine(*r, a)?),
                        }
                    }
                    writeln!(out, "        {} r = g{g}({});", rep_type(*ret), values.join(", ")).unwrap();
                    for k in temps {
                        writeln!(out, "        krelease(t{k});").unwrap();
                    }
                    for l in done {
                        writeln!(out, "        kset(&L[{l}], KUNIT);").unwrap();
                    }
                    writeln!(out, "        {}", cx.take_machine(dest, *ret, "r")?).unwrap();
                    writeln!(out, "    }}").unwrap();
                    writeln!(out, "    goto b{next};").unwrap();
                    continue;
                }
                if let Callee::Print | Callee::Panic = callee {
                    let text = args.first().map_or(Ok("KUNIT".to_string()), |a| operand(cx, a))?;
                    let helper = if *callee == Callee::Print { "kprint" } else { "kpanic_with" };
                    writeln!(out, "        kv r = {helper}({text});").unwrap();
                } else if let Callee::Extern(x) = callee {
                    let x = &externs[*x];
                    let mut parts = Vec::new();
                    // The strings it is given are held until it returns.
                    let mut held = Vec::new();
                    // The structs it changes: the argument and the struct.
                    let mut changed = Vec::new();
                    // The arrays it is lent, and for the ones it may change, how
                    // their numbers are read back.
                    let mut lent: Vec<(usize, Option<String>)> = Vec::new();
                    for (k, (a, t)) in args.iter().zip(&x.params).enumerate() {
                        let v = operand(cx, a)?;
                        if let Some((how, kind)) = t.split_once(' ').filter(|(how, _)| matches!(*how, "buf" | "mutbuf")) {
                            let (code, ctype) = (format!("KB_{}", kind.to_uppercase()), buf_type(kind));
                            writeln!(out, "        kv e{k} = {v};").unwrap();
                            writeln!(out, "        void *b{k} = kbuf_make(e{k}, {code});").unwrap();
                            parts.push(format!("({}{ctype} *)b{k}", if how == "buf" { "const " } else { "" }));
                            held.push(k);
                            lent.push((k, (how == "mutbuf").then_some(code)));
                            continue;
                        }
                        if t.starts_with("cb ") {
                            let Operand::Const(Value::Fn(f)) = a else { return Err("the C backend needs a callback to be the name of a function".into()) };
                            parts.push(format!("KCB{f}"));
                            continue;
                        }
                        if let Some(i) = t.strip_prefix("struct ") {
                            writeln!(out, "        kv e{k} = {v};").unwrap();
                            parts.push(format!("KX{i}_get(e{k})"));
                            held.push(k);
                            continue;
                        }
                        if let Some(i) = t.strip_prefix("mut ") {
                            writeln!(out, "        kv e{k} = {v};").unwrap();
                            writeln!(out, "        KX{i}_t c{k} = KX{i}_get(e{k});").unwrap();
                            parts.push(format!("&c{k}"));
                            held.push(k);
                            changed.push((k, i));
                            continue;
                        }
                        if *t == "str" || *t == "ptr?" {
                            writeln!(out, "        kv e{k} = {v};").unwrap();
                            parts.push(if *t == "str" { format!("kbytes(e{k})") } else { format!("kptr_c_opt(e{k})") });
                            held.push(k);
                            continue;
                        }
                        parts.push(match *t {
                            "ptr" => format!("(void *)(uintptr_t)({v}).u.i"),
                            "bool" => format!("({v}).u.t"),
                            "f64" => format!("({v}).u.f"),
                            "f32" => format!("({v}).u.g"),
                            int => format!("({})({v}).u.i", c_int(int)),
                        });
                    }
                    let call = format!("{}({})", x.name, parts.join(", "));
                    let r = match x.ret {
                        "unit" => format!("({call}, KUNIT)"),
                        "bool" => format!("kbool({call})"),
                        "f64" => format!("kf64({call})"),
                        "f32" => format!("kf32({call})"),
                        "str" => format!("kstr_c({call}, {})", quote(&x.name)),
                        "str?" => format!("kstr_c_opt({call})"),
                        "ptr" => format!("kptr_v({call}, {})", quote(&x.name)),
                        "ptr?" => format!("kptr_v_opt({call})"),
                        t if t.starts_with("struct ") => format!("KX{}_box({call})", &t[7..]),
                        _ => format!("kint((i128)({call}))"),
                    };
                    writeln!(out, "        kv r = {r};").unwrap();
                    for (k, back) in lent {
                        if let (Some(code), Some((_, place))) = (back, writeback.iter().find(|(w, _)| *w == k)) {
                            writeln!(out, "        {}", cx.take(place, &format!("kbuf_back(b{k}, klen(e{k}), {code})"))?).unwrap();
                        }
                        writeln!(out, "        free(b{k});").unwrap();
                    }
                    for k in held {
                        writeln!(out, "        krelease(e{k});").unwrap();
                    }
                    // What C left in a struct goes back to where it came from.
                    for (k, i) in changed {
                        if let Some((_, place)) = writeback.iter().find(|(w, _)| *w == k) {
                            writeln!(out, "        {}", cx.take(place, &format!("KX{i}_box(c{k})"))?).unwrap();
                        }
                    }
                } else if let Callee::Native(name) = callee {
                    let parts: Result<Vec<String>, String> = args.iter().map(|a| operand(cx, a)).collect();
                    writeln!(out, "        kv r = k{}({});", name.replace('.', "_"), parts?.join(", ")).unwrap();
                } else {
                    // A `mut` argument is moved out of its place for the
                    // call and put back after it: were it only copied,
                    // the first write of the callee would find the value
                    // shared and copy all of it. The other arguments are
                    // read first, since they may read what is moved.
                    let lent = |k: usize| writeback.iter().find(|(i, _)| *i == k).and_then(|(_, place)| match &args[k] {
                        Operand::Place(p) if p == place => cx.moved(p),
                        _ => None,
                    });
                    let mut parts = vec![String::new(); args.len()];
                    for (k, a) in args.iter().enumerate() {
                        if lent(k).is_none() {
                            parts[k] = match a {
                                Operand::Place(p) if p.proj.is_empty() && cx.reps[p.local] == Rep::Boxed && cx.spent[p.local] => format!("ktake(&L[{}])", p.local),
                                _ => operand(cx, a)?,
                            };
                        }
                    }
                    for (k, part) in parts.iter_mut().enumerate() {
                        if let Some(taken) = lent(k) {
                            *part = taken;
                        }
                    }
                    for (k, part) in parts.iter().enumerate() {
                        writeln!(out, "        kv t{k} = {part};").unwrap();
                    }
                    let init = if parts.is_empty() { "KUNIT".to_string() } else { (0..parts.len()).map(|k| format!("t{k}")).collect::<Vec<_>>().join(", ") };
                    writeln!(out, "        kv A[{}] = {{{init}}};", parts.len().max(1)).unwrap();
                    match callee {
                        Callee::Fn(f) => writeln!(out, "        kv r = f{f}(A, KUNIT);").unwrap(),
                        Callee::Indirect(op, _) => writeln!(out, "        kv r = kcall({}, A);", operand(cx, op)?).unwrap(),
                        Callee::Print | Callee::Panic | Callee::Native(_) | Callee::Extern(_) => {}
                    }
                    // The callee left the final values of its parameters in `A`.
                    for k in 0..parts.len() {
                        match writeback.iter().find(|(i, _)| *i == k) {
                            Some((_, place)) => writeln!(out, "        {}", cx.take(place, &format!("A[{k}]"))?).unwrap(),
                            None => writeln!(out, "        krelease(A[{k}]);").unwrap(),
                        }
                    }
                }
                writeln!(out, "        {}", cx.take(dest, "r")?).unwrap();
                writeln!(out, "    }}").unwrap();
                writeln!(out, "    goto b{next};").unwrap();
            }
            Term::Return(op) => {
                writeln!(out, "    {{").unwrap();
                match mine {
                    Some((_, Rep::Boxed)) => writeln!(out, "        kv r = {};", operand(cx, op)?).unwrap(),
                    Some((_, ret)) => writeln!(out, "        {} r = {};", rep_type(*ret), cx.machine(*ret, op)?).unwrap(),
                    None => writeln!(out, "        kv r = {};", operand(cx, op)?).unwrap(),
                }
                for l in 0..n {
                    if l < f.params && mine.is_some() {
                        continue;
                    }
                    if l < f.params && matches!(cx.reps[l], Rep::Rec(_) | Rep::Opt(_)) {
                        // It was read from the block of the caller, which is still there.
                    } else if l < f.params {
                        writeln!(out, "        a[{l}] = {};", cx.boxed(l)).unwrap();
                    } else if cx.reps.get(l) == Some(&Rep::Boxed) {
                        writeln!(out, "        krelease(L[{l}]);").unwrap();
                    }
                }
                writeln!(out, "        return r;").unwrap();
                writeln!(out, "    }}").unwrap();
            }
            Term::Panic(msg) => writeln!(out, "    kpanic({});", quote(msg)).unwrap(),
            Term::Unreachable => writeln!(out, "    kinternal(\"reached a block that has no terminator\");").unwrap(),
        }
    }
    match mine {
        Some((params, ret)) => {
            let nothing = match ret {
                Rep::Boxed => "KUNIT".to_string(),
                Rep::Rec(_) | Rep::Opt(_) => format!("(struct {}){{0}}", agg(*ret).unwrap_or_default()),
                _ => "0".to_string(),
            };
            writeln!(out, "    return {nothing};").unwrap();
            writeln!(out, "}}\n").unwrap();
            // The tagged entry, for calls through a function value.
            let unboxed: Vec<String> = params
                .iter()
                .enumerate()
                .map(|(l, r)| match r {
                    Rep::Int(k) => format!("({})a[{l}].u.i", ctype(k)),
                    Rep::F64 => format!("a[{l}].u.f"),
                    Rep::Boxed => format!("a[{l}]"),
                    Rep::Rec(_) | Rep::Opt(_) => format!("{}_get(a[{l}])", agg(*r).unwrap_or_default()),
                    _ => format!("a[{l}].u.t"),
                })
                .collect();
            writeln!(out, "static kv f{i}(kv *a, kv env) {{").unwrap();
            writeln!(out, "    (void)env;").unwrap();
            writeln!(out, "    (void)a;").unwrap();
            writeln!(out, "    return {};", tagged(*ret, &format!("g{i}({})", unboxed.join(", ")))).unwrap();
            writeln!(out, "}}\n").unwrap();
        }
        None => {
            writeln!(out, "    return KUNIT;").unwrap();
            writeln!(out, "}}\n").unwrap();
        }
    }
    Ok(())
}

/// The C source of a program. `paths` are the files of its modules, which
/// panics report; the first is the program's own.
pub fn emit(p: &Program, paths: &[String]) -> Result<String, String> {
    let path = paths.first().map_or("program", |p| p.as_str());
    let main = p.main.filter(|m| p.fns[*m].params == 0).ok_or("the program has no `main` function without parameters")?;
    let mut out = String::new();
    let names = |items: &mut dyn Iterator<Item = String>| items.map(|s| format!("{s}, ")).collect::<String>();
    writeln!(out, "/* Generated by mou from {path}. */").unwrap();
    // A program that starts tasks needs the runtime that is safe between threads.
    let spawns = |f: &Func| f.blocks.iter().any(|b| matches!(&b.term, Term::Call { callee: Callee::Native("task.spawn"), .. }));
    if p.fns.iter().any(spawns) {
        writeln!(out, "#define KTHREADS 1").unwrap();
    }
    writeln!(out, "static const char *const KFILE = {};", quote(path)).unwrap();
    let files = paths.iter().map(|p| quote(p)).collect::<Vec<_>>().join(", ");
    writeln!(out, "static const char *const KFILES[] = {{{files}}};").unwrap();
    // Entry 0 is where the program is before its first statement.
    let funcs = std::iter::once("main").chain(p.fns.iter().map(|f| f.name.as_str())).map(quote).collect::<Vec<_>>().join(", ");
    writeln!(out, "static const char *const KFUNCS[] = {{{funcs}}};").unwrap();
    writeln!(out, "enum {{ KFILE_COUNT = {}, KFILE_SHIFT = {} }};", paths.len(), crate::lexer::FILE_SHIFT).unwrap();
    // The structs an array keeps in line, and how it keeps each field.
    for (i, kinds) in p.records.iter().enumerate().filter(|(_, k)| !k.is_empty()) {
        let kinds: Vec<String> = kinds.iter().map(|k| k.to_string()).collect();
        writeln!(out, "static const signed char KR{i}[] = {{{}}};", kinds.join(", ")).unwrap();
    }
    let tables = |f: &dyn Fn(usize, &Vec<u8>) -> String| p.records.iter().enumerate().map(|(i, k)| f(i, k) + ", ").collect::<String>();
    writeln!(out, "static const signed char *const KREC_K[] = {{{}0}};", tables(&|i, k| if k.is_empty() { "0".into() } else { format!("KR{i}") })).unwrap();
    writeln!(out, "static const int KREC_N[] = {{{}0}};", tables(&|_, k| k.len().to_string())).unwrap();
    writeln!(out, "enum {{ KREC_COUNT = {} }};", p.records.len()).unwrap();
    for (i, (_, fields)) in p.structs.iter().enumerate() {
        writeln!(out, "static const char *const KSF{i}[] = {{{}0}};", names(&mut fields.iter().map(|f| quote(f)))).unwrap();
    }
    writeln!(out, "static const char *const KSTRUCT_NAME[] = {{{}0}};", names(&mut p.structs.iter().map(|s| quote(&s.0)))).unwrap();
    writeln!(out, "static const char *const *const KSTRUCT_FIELDS[] = {{{}0}};", names(&mut (0..p.structs.len()).map(|i| format!("KSF{i}")))).unwrap();
    for (e, (_, variants)) in p.enums.iter().enumerate() {
        for (v, (_, fields)) in variants.iter().enumerate() {
            let fs = names(&mut fields.iter().map(|f| f.as_ref().map_or("0".to_string(), |f| quote(f))));
            writeln!(out, "static const char *const KEF{e}_{v}[] = {{{fs}0}};").unwrap();
        }
        writeln!(out, "static const char *const KEV{e}[] = {{{}0}};", names(&mut variants.iter().map(|v| quote(&v.0)))).unwrap();
        writeln!(out, "static const char *const *const KEF{e}[] = {{{}0}};", names(&mut (0..variants.len()).map(|v| format!("KEF{e}_{v}")))).unwrap();
    }
    writeln!(out, "static const char *const *const KENUM_VARIANTS[] = {{{}0}};", names(&mut (0..p.enums.len()).map(|e| format!("KEV{e}")))).unwrap();
    writeln!(out, "static const char *const *const *const KENUM_FIELDS[] = {{{}0}};", names(&mut (0..p.enums.len()).map(|e| format!("KEF{e}")))).unwrap();
    writeln!(out, "static const char *const KFN_NAME[] = {{{}0}};\n", names(&mut p.fns.iter().map(|f| quote(&f.name)))).unwrap();
    out.push_str(RUNTIME);
    let mut headers: Vec<&str> = p.externs.iter().map(|x| x.header.as_str()).collect();
    headers.sort();
    headers.dedup();
    for h in headers {
        writeln!(out, "#include \"{h}\"").unwrap();
    }
    writeln!(out, "\n/* ---- the program ---- */\n").unwrap();
    writeln!(out, "static kv KC[{}];", p.consts.len().max(1)).unwrap();
    for i in 0..p.fns.len() {
        writeln!(out, "static kv f{i}(kv *a, kv env);").unwrap();
    }
    writeln!(out, "static kv (*const KFN[])(kv *, kv) = {{{}0}};", names(&mut (0..p.fns.len()).map(|i| format!("f{i}")))).unwrap();
    writeln!(out, "static kv kdispatch(uint32_t id, kv *args, kv env) {{ return KFN[id](args, env); }}\n").unwrap();
    // The structs kept as structs of C: how each is read from a tagged
    // value and made into one.
    let mut kept: Vec<(usize, Vec<u8>)> = p.records.iter().cloned().enumerate().filter(|(_, k)| !k.is_empty() && !k.contains(&4)).collect();
    // And the tuples kept that way, by their number.
    let mut tuples: Vec<usize> = Vec::new();
    for f in &p.fns {
        for s in f.scalars.iter().chain(&f.hints).chain(std::iter::once(&f.ret)) {
            if let Some(Scalar::Rec(i)) = s {
                if *i >= TUPLE_BASE && !tuples.contains(i) {
                    tuples.push(*i);
                }
            }
        }
    }
    tuples.sort();
    kept.extend(tuples.into_iter().map(|i| (i, tuple_kinds(i))));
    for (i, kinds) in &kept {
        let (i, tag) = (*i, if *i >= TUPLE_BASE { "T_TUPLE".to_string() } else { "T_STRUCT".to_string() });
        let shown = if i >= TUPLE_BASE { 0 } else { i };
        let member = |k: &u8| rep_type(field_rep(*k));
        let fields: Vec<String> = kinds.iter().enumerate().map(|(f, k)| format!("{} f{f};", member(k))).collect();
        writeln!(out, "struct KS{i} {{ {} }};", fields.join(" ")).unwrap();
        let read: Vec<String> = kinds
            .iter()
            .enumerate()
            .map(|(f, k)| match k {
                1 => format!("it[{f}].u.t"),
                3 => format!("it[{f}].u.f"),
                5 => format!("(uint64_t)it[{f}].u.i"),
                _ => format!("(int64_t)it[{f}].u.i"),
            })
            .collect();
        writeln!(out, "static struct KS{i} KS{i}_get(kv v) {{ const kv *it = ITEMS(v.u.o); return (struct KS{i}){{{}}}; }}", read.join(", ")).unwrap();
        writeln!(out, "static struct KS{i} KS{i}_take(kv v) {{ struct KS{i} r = KS{i}_get(v); krelease(v); return r; }}").unwrap();
        let made: Vec<String> = kinds.iter().enumerate().map(|(f, k)| tagged(field_rep(*k), &format!("s.f{f}"))).collect();
        writeln!(out, "static kv KS{i}_box(struct KS{i} s) {{ return kagg({tag}, {shown}, 0, {}, (kv[]){{{}}}); }}", kinds.len(), made.join(", ")).unwrap();
    }
    // The enums kept as structs of C: which variant, and a field for
    // each position. How many fields a variant has says how many are read
    // from a tagged value and put in one.
    let recs_all: Vec<Vec<u8>> = p.records.iter().chain(&p.sums).cloned().collect();
    for (e, kinds) in p.sums.iter().enumerate().filter(|(_, k)| !k.is_empty()) {
        let i = p.records.len() + e;
        let fields: Vec<String> = kinds.iter().enumerate().map(|(f, k)| format!("{} f{f};", rep_type(field_rep(*k)))).collect();
        writeln!(out, "struct KS{i} {{ {} }};", fields.join(" ")).unwrap();
        writeln!(out, "static struct KS{i} KS{i}_get(kv v) {{").unwrap();
        writeln!(out, "    struct KS{i} r = {{0}};").unwrap();
        writeln!(out, "    int64_t n = v.u.o ? v.u.o->len : 0;").unwrap();
        writeln!(out, "    const kv *it = n ? ITEMS(v.u.o) : 0;").unwrap();
        writeln!(out, "    r.f0 = v.b;").unwrap();
        for (f, k) in kinds.iter().enumerate().skip(1) {
            let read = match k {
                1 => format!("it[{}].u.t", f - 1),
                3 => format!("it[{}].u.f", f - 1),
                5 => format!("(uint64_t)it[{}].u.i", f - 1),
                _ => format!("(int64_t)it[{}].u.i", f - 1),
            };
            writeln!(out, "    if (n > {}) r.f{f} = {read};", f - 1).unwrap();
        }
        writeln!(out, "    return r;").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out, "static struct KS{i} KS{i}_take(kv v) {{ struct KS{i} r = KS{i}_get(v); krelease(v); return r; }}").unwrap();
        let counts: Vec<String> = p.enums[e].1.iter().map(|(_, fields)| fields.len().to_string()).collect();
        let made: Vec<String> = kinds.iter().enumerate().skip(1).map(|(f, k)| tagged(field_rep(*k), &format!("s.f{f}"))).collect();
        writeln!(out, "static kv KS{i}_box(struct KS{i} s) {{").unwrap();
        writeln!(out, "    static const int8_t count[] = {{{}}};", counts.join(", ")).unwrap();
        writeln!(out, "    return kagg(T_ENUM, {e}, (uint32_t)s.f0, count[s.f0], (kv[]){{{}}});", made.join(", ")).unwrap();
        writeln!(out, "}}").unwrap();
    }
    // The functions C is given to call: each as a function of C that
    // makes tagged values of what it gets and calls the one of the program.
    let mut callbacks: Vec<(usize, &str)> = Vec::new();
    for f in &p.fns {
        for b in &f.blocks {
            if let Term::Call { callee: Callee::Extern(x), args, .. } = &b.term {
                for (a, t) in args.iter().zip(&p.externs[*x].params) {
                    if let (Operand::Const(Value::Fn(g)), Some(sig)) = (a, t.strip_prefix("cb ")) {
                        if !callbacks.iter().any(|(h, _)| h == g) {
                            callbacks.push((*g, sig));
                        }
                    }
                }
            }
        }
    }
    callbacks.sort();
    for (g, sig) in callbacks {
        let (params, ret) = sig.split_once('>').unwrap_or((sig, "unit"));
        let params: Vec<&str> = params.split(',').filter(|t| !t.is_empty()).collect();
        let ctype = |t: &str| match t {
            "unit" => "void".to_string(),
            "bool" => "bool".to_string(),
            "ptr" | "ptr?" => "void *".to_string(),
            other => buf_type(other),
        };
        let takes: Vec<String> = params.iter().enumerate().map(|(k, t)| format!("{} a{k}", ctype(t))).collect();
        let given: Vec<String> = params
            .iter()
            .enumerate()
            .map(|(k, t)| match *t {
                "bool" => format!("kbool(a{k})"),
                "f64" => format!("kf64(a{k})"),
                "f32" => format!("kf32(a{k})"),
                "ptr?" => format!("kptr_v_opt(a{k})"),
                _ => format!("kint((i128)(a{k}))"),
            })
            .collect();
        writeln!(out, "static {} KCB{g}({}) {{", ctype(ret), if takes.is_empty() { "void".to_string() } else { takes.join(", ") }).unwrap();
        writeln!(out, "    kv A[{}] = {{{}}};", params.len().max(1), if given.is_empty() { "KUNIT".to_string() } else { given.join(", ") }).unwrap();
        writeln!(out, "    kv r = f{g}(A, KUNIT);").unwrap();
        for k in 0..params.len() {
            writeln!(out, "    krelease(A[{k}]);").unwrap();
        }
        let back = match ret {
            "unit" => None,
            "bool" => Some("r.u.t".to_string()),
            "f64" => Some("r.u.f".to_string()),
            "f32" => Some("r.u.g".to_string()),
            "ptr" => Some("(void *)(uintptr_t)r.u.i".to_string()),
            "ptr?" => Some("kptr_c_opt(r)".to_string()),
            int => Some(format!("({})r.u.i", c_int(int))),
        };
        match back {
            Some(value) => {
                writeln!(out, "    {} out = {value};", ctype(ret)).unwrap();
                writeln!(out, "    krelease(r);").unwrap();
                writeln!(out, "    return out;").unwrap();
            }
            None => writeln!(out, "    krelease(r);").unwrap(),
        }
        writeln!(out, "}}").unwrap();
    }
    // The structs of C: how each is read from a tagged value and made
    // into one, member by member.
    for (i, name, kinds) in &p.cstructs {
        let fields = &p.structs[*i].1;
        writeln!(out, "typedef {name} KX{i}_t;").unwrap();
        writeln!(out, "static KX{i}_t KX{i}_get(kv v) {{").unwrap();
        writeln!(out, "    const kv *it = ITEMS(v.u.o);").unwrap();
        writeln!(out, "    KX{i}_t c;").unwrap();
        writeln!(out, "    memset(&c, 0, sizeof c);").unwrap();
        let mut made = Vec::new();
        for (k, (f, kind)) in fields.iter().zip(kinds).enumerate() {
            let (read, back) = match *kind {
                "bool" => (format!("it[{k}].u.t"), format!("kbool(c.{f})")),
                "f64" => (format!("it[{k}].u.f"), format!("kf64(c.{f})")),
                "f32" => (format!("it[{k}].u.g"), format!("kf32(c.{f})")),
                "ptr" => (format!("(void *)(uintptr_t)it[{k}].u.i"), format!("kptr_v(c.{f}, {})", quote(f))),
                "ptr?" => (format!("kptr_c_opt(it[{k}])"), format!("kptr_v_opt(c.{f})")),
                int => (format!("({})it[{k}].u.i", c_int(int)), format!("kint((i128)(c.{f}))")),
            };
            writeln!(out, "    c.{f} = {read};").unwrap();
            made.push(back);
        }
        writeln!(out, "    return c;").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out, "static kv KX{i}_box(KX{i}_t c) {{ return kagg(T_STRUCT, {i}, 0, {}, (kv[]){{{}}}); }}", kinds.len(), made.join(", ")).unwrap();
    }
    let direct: Vec<Option<(Vec<Rep>, Rep)>> = p.fns.iter().map(|f| direct_sig(f, &p.fns, &recs_all)).collect();
    for (i, sig) in direct.iter().enumerate() {
        if let Some((params, ret)) = sig {
            writeln!(out, "{};", direct_head(i, params, *ret)).unwrap();
        }
    }
    for (i, f) in p.fns.iter().enumerate() {
        function(&mut out, i, f, &p.fns, &p.externs, &direct, &recs_all).map_err(|e| format!("{}: {e}", f.name))?;
    }
    writeln!(out, "int main(int argc, char **argv) {{").unwrap();
    writeln!(out, "    char base;").unwrap();
    writeln!(out, "    kargc = argc;").unwrap();
    writeln!(out, "    kargv = argv;").unwrap();
    writeln!(out, "    kv A[1] = {{KUNIT}};").unwrap();
    writeln!(out, "    kstack_base = &base;").unwrap();
    for (k, f) in p.consts.iter().enumerate() {
        // A constant computed while compiling is written as its value.
        match p.values.get(k).and_then(|v| v.as_ref()).and_then(|v| constant(v).ok()) {
            Some(value) => writeln!(out, "    KC[{k}] = {value};").unwrap(),
            None => writeln!(out, "    KC[{k}] = f{f}(A, KUNIT);").unwrap(),
        }
    }
    writeln!(out, "    int status = kfinish(f{main}(A, KUNIT));").unwrap();
    writeln!(out, "    for (int i = 0; i < {}; i++) krelease(KC[i]);", p.consts.len()).unwrap();
    writeln!(out, "    return status;").unwrap();
    writeln!(out, "}}").unwrap();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::{check, interp, mir, own};
    use std::process::Command;

    /// Builds a program with the C backend and runs it. `None` when there is
    /// no C compiler to test with.
    fn native(name: &str, src: &str) -> Option<(String, i32)> {
        native_at(name, "test.mou", src)
    }

    /// The same, for a program that sits in a file: its modules are next to it.
    fn native_at(name: &str, path: &str, src: &str) -> Option<(String, i32)> {
        let file = crate::load::load(path, src).1.unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let (_, info) = check::analyze(&file).unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let mut program = mir::lower(&file, &info).unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        assert!(own::check(&program).is_empty());
        // As in `mou build`: constants become values when they can be computed.
        let _ = crate::comptime::eval(&mut program);
        let dir = std::env::temp_dir().join(format!("mou-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (c, exe) = (dir.join(format!("{name}.c")), dir.join(name));
        std::fs::write(&c, super::emit(&program, &["test.mou".to_string()]).unwrap()).unwrap();
        let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
        let built = Command::new(cc).arg("-O1").arg("-pthread").arg("-o").arg(&exe).arg(&c).arg("-lm").output().ok()?;
        assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
        let run = Command::new(&exe).output().unwrap();
        // Nothing is left behind in the directory of temporary files.
        let _ = std::fs::remove_file(&c);
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::remove_dir(&dir);
        Some((String::from_utf8(run.stdout).unwrap(), run.status.code().unwrap_or(-1)))
    }

    /// What the interpreter prints and the exit status `mou run` would give.
    fn interpreted(src: &str) -> (String, i32) {
        let file = crate::load::load("test.mou", src).1.unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let (_, info) = check::analyze(&file).unwrap();
        let program = mir::lower(&file, &info).unwrap();
        let mut out = Vec::new();
        let status = match interp::run(&program, program.main.unwrap(), &mut out) {
            Ok(mir::Value::Int(i)) => (i & 0xFF) as i32,
            Ok(mir::Value::Err(_)) => 1,
            Ok(_) => 0,
            Err(_) => 101,
        };
        (String::from_utf8(out).unwrap(), status)
    }

    /// The programs of `tests/c` that have a `main` call real functions of
    /// C. Only the native program can run them, so what each one prints
    /// is compared with what is written next to it.
    #[test]
    fn c_programs_run() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/c");
        let mut paths: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "out")).collect();
        paths.sort();
        assert!(paths.len() >= 5, "the expected results are missing");
        for expected in paths {
            let name = expected.file_stem().unwrap().to_string_lossy().to_string();
            let path = expected.with_extension("mou");
            let src = std::fs::read_to_string(&path).unwrap();
            let Some(got) = native_at(&format!("c-{name}"), &path.to_string_lossy(), &src) else {
                eprintln!("no C compiler: the programs that call C were not run");
                return;
            };
            assert_eq!(got, (std::fs::read_to_string(&expected).unwrap(), 0), "{name}");
        }
    }

    #[test]
    fn c_functions_are_called() {
        let src = "@profile(system)
import c \"math.h\" as m {
    fn sqrt(x: f64) -> f64
    fn floor(x: f64) -> f64
}
import c \"stdlib.h\" as lib {
    fn abs(n: i32) -> i32
    fn labs(n: i64) -> i64
}
import c \"ctype.h\" as ct { fn isdigit(c: i32) -> i32 }
fn digit(c: u8) -> bool { unsafe { ct.isdigit(c as i32) != 0 } }
fn main() -> int {
    let root = unsafe { m.floor(m.sqrt(50.0)) as int }
    let far = unsafe { lib.labs(-9000000000) - 9000000000 }
    var n = root + far + unsafe { lib.abs(-30) as int }
    if digit(53) && !digit(65) { n += 5 }
    n
}";
        if let Some((out, status)) = native("cfuncs", src) {
            assert_eq!((out.as_str(), status), ("", 42));
        }
        // The interpreter says that it cannot call them.
        let file = crate::load::load("test.mou", src).1.unwrap();
        let (_, info) = check::analyze(&file).unwrap();
        let program = mir::lower(&file, &info).unwrap();
        let stopped = interp::run(&program, program.main.unwrap(), &mut Vec::new()).unwrap_err();
        assert!(stopped.msg.contains("can only be called in a compiled program"), "{}", stopped.msg);
    }

    #[test]
    fn native_programs_match_the_interpreter() {
        let programs = [
            ("core", include_str!("../examples/core.mou").to_string()),
            ("shapes", include_str!("../examples/shapes.mou").to_string()),
            ("tour", include_str!("../examples/tour.mou").to_string()),
            ("traits", include_str!("../examples/traits.mou").to_string()),
            ("closures", include_str!("../examples/closures.mou").to_string()),
            ("values", include_str!("../examples/values.mou").to_string()),
            ("generic_types", include_str!("../examples/generic_types.mou").to_string()),
            ("library", include_str!("../examples/library.mou").to_string()),
            ("overflow", "fn f(a: u8, b: u8) -> u8 { a + b }\nfn main() -> int { f(200, 100) as int }".to_string()),
            ("bounds", "fn main() -> int {\n let xs = [1, 2, 3]\n var i = 0\n while true { i += xs[i] }\n i\n}".to_string()),
            ("status", "fn main() -> int { 42 }".to_string()),
            ("failure", "enum E { Bad }\nfn g() ! E { return E.Bad }\nfn main() ! E { g()? }".to_string()),
        ];
        // Arithmetic at the limits of each integer kind, calls between
        // functions of machine integers, and what must panic.
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/native");
        let mut programs: Vec<(String, String)> = programs.iter().map(|(n, s)| (n.to_string(), s.clone())).collect();
        let mut edge: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        edge.sort();
        assert!(edge.len() >= 28, "the edge cases are missing");
        for path in edge {
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            programs.push((name, std::fs::read_to_string(&path).unwrap()));
        }
        for (name, src) in &programs {
            let Some(got) = native(name, src) else {
                eprintln!("no C compiler: the C backend was not tested");
                return;
            };
            assert_eq!(got, interpreted(src), "{name}");
        }
    }
}
