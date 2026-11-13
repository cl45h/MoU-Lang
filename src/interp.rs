//! Interpreter for the MIR.
//!
//! Calls use an explicit stack of frames, so deep recursion in a program is
//! reported as a panic instead of overflowing the interpreter's own stack.
//!
//! Known limit: integers are held in 128 signed bits, so `u128` values of
//! 2^127 or more are reported as a panic instead of computed.

use std::cmp::Ordering;
use std::io::Write;
use std::rc::Rc;

use crate::lexer::Span;
use crate::mir::*;

#[derive(Debug)]
pub struct Panic {
    pub msg: String,
    pub span: Span,
    /// Names of the active functions, innermost first.
    pub stack: Vec<String>,
}

struct Frame {
    func: usize,
    block: usize,
    locals: Vec<Value>,
    /// Values captured by the closure this frame runs.
    env: Rc<Vec<Value>>,
}

const MAX_DEPTH: usize = 100_000;

/// Runs the constants and then `main`. Returns what `main` returned.
pub fn run(p: &Program, main: usize, out: &mut dyn Write) -> Result<Value, Panic> {
    let mut consts = Vec::new();
    for (k, &f) in p.consts.iter().enumerate() {
        // A constant computed while compiling is not computed again.
        let v = match p.values.get(k) {
            Some(Some(v)) => v.clone(),
            _ => exec(p, &consts, f, out)?,
        };
        consts.push(v);
    }
    exec(p, &consts, main, out)
}

/// Computes a constant, given the ones before it. What it prints is lost.
pub fn constant(p: &Program, consts: &[Value], f: usize) -> Result<Value, Panic> {
    exec(p, consts, f, &mut std::io::sink())
}

thread_local! {
    /// How many more blocks may run. Only what runs while compiling has a limit.
    pub static FUEL: std::cell::Cell<u64> = const { std::cell::Cell::new(u64::MAX) };
    /// Whether what runs is a constant being computed, and not the program.
    pub static COMPILING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

thread_local! {
    /// The arguments of the program being run, for `io.args`.
    pub static ARGS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// The channels of the program being run: for each, the values it
    /// holds and whether it is closed. A channel is its number here.
    static CHANS: std::cell::RefCell<Vec<(std::collections::VecDeque<Value>, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}

thread_local! {
    /// The values tasks share: for each, the value and whether an
    /// `update` of it is running. A `Mutex` is its number here.
    static CELLS: std::cell::RefCell<Vec<(Value, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// What a `Mutex` does. Nothing waits here: the only one that can be
/// updating a value is whoever asks for it.
fn shared(name: &str, args: &[Value]) -> Result<Value, String> {
    if COMPILING.get() {
        return Err("a `Mutex` cannot be used while the program is compiled".into());
    }
    let given = |k: usize| args.get(k).cloned().unwrap_or(Value::Unit);
    CELLS.with_borrow_mut(|cs| {
        if name == "mutex.new" {
            cs.push((given(0), false));
            return Ok(Value::Int(cs.len() as i128 - 1));
        }
        let cell = match args.first() {
            Some(Value::Int(c)) => cs.get_mut(*c as usize),
            _ => None,
        };
        let Some((value, held)) = cell else { return Err(internal("a `Mutex` that is not one")) };
        if *held && name != "mutex.put" {
            return Err("this `Mutex` is used inside its own `update`".into());
        }
        Ok(match name {
            "mutex.get" => value.clone(),
            "mutex.take" => {
                *held = true;
                std::mem::replace(value, Value::Unit)
            }
            _ => {
                *value = given(1);
                *held = false;
                Value::Unit
            }
        })
    })
}

/// What a channel does. Here a task runs whole where it is started, so a
/// channel holds whatever is sent and nothing can wait.
fn channel(name: &str, args: &[Value]) -> Result<Value, String> {
    if COMPILING.get() {
        return Err("a channel cannot be used while the program is compiled".into());
    }
    if name == "chan.new" {
        return match args.first() {
            Some(Value::Int(n)) if *n < 1 => Err("a channel holds at least one value".into()),
            Some(Value::Int(n)) if *n > 1 << 24 => Err("a channel cannot hold that many values".into()),
            _ => CHANS.with_borrow_mut(|cs| {
                cs.push(Default::default());
                Ok(Value::Int(cs.len() as i128 - 1))
            }),
        };
    }
    let Some(Value::Int(c)) = args.first() else { return Err(internal("a channel that is not one")) };
    CHANS.with_borrow_mut(|cs| {
        let Some((items, closed)) = cs.get_mut(*c as usize) else { return Err(internal("a channel that does not exist")) };
        match name {
            "chan.send" if *closed => Err("`send` on a closed channel".into()),
            "chan.send" => {
                items.push_back(args.get(1).cloned().unwrap_or(Value::Unit));
                Ok(Value::Unit)
            }
            "chan.recv" => match items.pop_front() {
                Some(v) => Ok(Value::Some(Box::new(v))),
                None if *closed => Ok(Value::None),
                None => Err("`recv` would wait forever: with `mou run` a task runs whole where it is started, so nobody is left to send; build the program to run its tasks at once".into()),
            },
            _ => {
                *closed = true;
                Ok(Value::Unit)
            }
        }
    })
}

/// The hash of a text: FNV-1a, 64 bits.
pub fn hash(text: &str) -> i64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    h as i64
}

/// Spreads the bits of an integer: the last step of SplitMix64.
pub fn mix(n: i64) -> i64 {
    let mut z = (n as u64).wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    (z ^ (z >> 31)) as i64
}

/// Runs a function of a built-in module.
fn native(name: &str, args: &[Value], out: &mut dyn Write) -> Result<Value, String> {
    use std::io::BufRead;
    let text = |k: usize| match args.get(k) {
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(internal("a built-in function given a value that is not a string")),
    };
    let items = |k: usize| match args.get(k) {
        Some(Value::Array(xs)) => Ok(xs.clone()),
        _ => Err(internal("a built-in function given a value that is not an array")),
    };
    let done = |r: std::io::Result<Value>| match r {
        Ok(v) => Value::Ok(Box::new(v)),
        Err(e) => Value::Err(Box::new(Value::Str(e.to_string().into()))),
    };
    Ok(match name {
        _ if name.starts_with("chan.") => channel(name, args)?,
        _ if name.starts_with("mutex.") => shared(name, args)?,
        "task.open" => Value::Int(0),
        "task.close" | "task.cancel" | "arena.close" => Value::Unit,
        // Here memory is asked for the same way inside an arena and outside.
        "arena.open" => Value::Int(0),
        // Here a task is done before anything can ask it to stop.
        "task.cancelled" => Value::Bool(false),
        "task.wait" => match args.first() {
            Some(v) => v.clone(),
            None => return Err(internal("`wait` without a task")),
        },
        "io.eprint" => {
            let _ = out.flush();
            done(writeln!(std::io::stderr(), "{}", text(0)?).map(|()| Value::Unit))
        }
        "io.read_line" => {
            let _ = out.flush();
            let mut line = String::new();
            done(std::io::stdin().lock().read_line(&mut line).map(|n| {
                if n == 0 {
                    return Value::None;
                }
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Value::Some(Box::new(Value::Str(line.into())))
            }))
        }
        "io.read_file" => done(std::fs::read_to_string(&*text(0)?).map(|s| Value::Str(s.into()))),
        "io.write_file" => done(std::fs::write(&*text(0)?, text(1)?.as_bytes()).map(|()| Value::Unit)),
        "io.args" => Value::Array(Rc::new(ARGS.with(|a| a.borrow().iter().map(|s| Value::Str(s.as_str().into())).collect()))),
        "raw.concat" => {
            let mut s = String::new();
            for part in items(0)?.iter() {
                match part {
                    Value::Str(p) => s.push_str(p),
                    _ => return Err(internal("`concat` of a value that is not a string")),
                }
            }
            Value::Str(s.into())
        }
        "raw.from_chars" => {
            let mut s = String::new();
            for c in items(0)?.iter() {
                match c {
                    Value::Char(c) => s.push(*c),
                    _ => return Err(internal("`from_chars` of a value that is not a character")),
                }
            }
            Value::Str(s.into())
        }
        "path.join" => {
            let mut all = items(0)?.to_vec();
            all.extend(items(1)?.iter().cloned());
            Value::Array(Rc::new(all))
        }
        "raw.to_char" => match args.first() {
            Some(Value::Int(i)) => match u32::try_from(*i).ok().and_then(char::from_u32) {
                Some(c) => Value::Char(c),
                None => return Err(format!("`as`: the value {i} is not a character")),
            },
            _ => return Err(internal("a cast to a character of a value that is not an integer")),
        },
        "raw.parse_float" => {
            let s = text(0)?;
            match float_shape(&s).then(|| s.parse::<f64>().ok()).flatten() {
                Some(f) => Value::Some(Box::new(Value::Float(f))),
                None => Value::None,
            }
        }
        "raw.hash" => Value::Int(hash(&text(0)?) as i128),
        "raw.mix" => match args.first() {
            Some(Value::Int(n)) => Value::Int(mix(*n as i64) as i128),
            _ => return Err(internal("`mix` of a value that is not an integer")),
        },
        _ => return Err(internal("an unknown built-in function")),
    })
}

/// Whether a text is a number with an optional sign, fraction and
/// exponent, and nothing else: `-12.5e3`.
pub fn float_shape(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > start
    };
    if b.first() == Some(&b'-') {
        i += 1;
    }
    if !digits(&mut i) {
        return false;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !digits(&mut i) {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !digits(&mut i) {
            return false;
        }
    }
    i == b.len()
}

fn internal(what: &str) -> String {
    format!("internal error in the interpreter: {what}")
}

fn frame(p: &Program, func: usize, mut locals: Vec<Value>, env: Rc<Vec<Value>>) -> Frame {
    locals.resize(p.fns[func].locals.max(locals.len()), Value::Unit);
    Frame { func, block: 0, locals, env }
}

fn exec(p: &Program, consts: &[Value], func: usize, out: &mut dyn Write) -> Result<Value, Panic> {
    let mut stack = vec![frame(p, func, Vec::new(), Rc::default())];
    let fail = |stack: &[Frame], msg: String, span: Span| Panic {
        msg,
        span,
        stack: stack.iter().rev().map(|f| p.fns[f.func].name.clone()).collect(),
    };
    // For each function reached, the temporaries a call gives away.
    let mut spent: Vec<Option<Vec<bool>>> = vec![None; p.fns.len()];
    loop {
        let top = stack.len() - 1;
        let bb = &p.fns[stack[top].func].blocks[stack[top].block];
        let fuel = FUEL.get();
        if fuel != u64::MAX {
            if fuel == 0 {
                return Err(fail(&stack, "it runs for too long to be computed while compiling".into(), bb.span));
            }
            FUEL.set(fuel - 1);
        }
        for st in &bb.stmts {
            let fr = &mut stack[top];
            let done = match &st.rv {
                // The two operations that change an array where it lives.
                Rvalue::Push(target, item) => operand(fr, item).and_then(|v| match slot(fr, target)? {
                    Value::Array(xs) => {
                        Rc::make_mut(xs).push(v);
                        Ok(Value::Unit)
                    }
                    _ => Err(internal("`push` on a value that is not an array")),
                }),
                Rvalue::Pop(target) => slot(fr, target).and_then(|a| match a {
                    Value::Array(xs) if xs.is_empty() => Ok(Value::None),
                    Value::Array(xs) => Ok(Rc::make_mut(xs).pop().map_or(Value::None, |v| Value::Some(Box::new(v)))),
                    _ => Err(internal("`pop` on a value that is not an array")),
                }),
                rv => rvalue(p, consts, fr, rv),
            }
            .and_then(|v| write(fr, &st.place, v));
            if let Err(msg) = done {
                return Err(fail(&stack, msg, st.span));
            }
        }
        macro_rules! tri {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(msg) => return Err(fail(&stack, msg, bb.span)),
                }
            };
        }
        match &bb.term {
            Term::Goto(b) => stack[top].block = *b,
            Term::Branch(c, yes, no) => match tri!(operand(&stack[top], c)) {
                Value::Bool(c) => stack[top].block = if c { *yes } else { *no },
                _ => tri!(Err(internal("a branch on a value that is not a bool"))),
            },
            Term::Call { callee, args, writeback, dest, next } => {
                // A `mut` argument is moved out of its place for the call,
                // which puts it back, and a temporary only the call reads
                // is given away: a copy would leave the value shared, and
                // the first write to it would copy all of it.
                let func = stack[top].func;
                let spent = &*spent[func].get_or_insert_with(|| crate::cgen::spent(&p.fns[func]));
                let lent = |k: usize| writeback.iter().any(|(i, place)| *i == k && matches!(&args[k], Operand::Place(p) if p == place));
                let mut vals: Vec<Value> = Vec::with_capacity(args.len());
                for (k, a) in args.iter().enumerate() {
                    vals.push(match a {
                        _ if lent(k) => Value::Unit,
                        Operand::Place(p) if p.proj.is_empty() && spent[p.local] => std::mem::replace(&mut stack[top].locals[p.local], Value::Unit),
                        _ => tri!(operand(&stack[top], a)),
                    });
                }
                for (k, a) in args.iter().enumerate() {
                    if let (true, Operand::Place(p)) = (lent(k), a) {
                        vals[k] = std::mem::replace(tri!(slot(&mut stack[top], p)), Value::Unit);
                    }
                }
                let target = match callee {
                    Callee::Fn(f) => Some((*f, Rc::default())),
                    Callee::Indirect(op) => match tri!(operand(&stack[top], op)) {
                        Value::Fn(f) => Some((f, Rc::default())),
                        Value::Closure(f, env) => Some((f, env)),
                        _ => tri!(Err(internal("a call to a value that is not a function"))),
                    },
                    // Here a task runs where it is started, and is its own result.
                    Callee::Native("task.spawn") => match vals.pop() {
                        Some(Value::Closure(f, env)) => Some((f, env)),
                        _ => tri!(Err(internal("a task that is not a closure"))),
                    },
                    Callee::Print => None,
                    Callee::Native(name) => {
                        let result = tri!(native(name, &vals, out));
                        tri!(write(&mut stack[top], dest, result));
                        stack[top].block = *next;
                        continue;
                    }
                    Callee::Panic => {
                        let text = vals.first().map_or(String::new(), |v| show(p, v, true));
                        tri!(Err(text))
                    }
                    Callee::Extern(x) => {
                        let name = &p.externs[*x].name;
                        tri!(Err(format!("the C function `{name}` can only be called in a compiled program: use `mou build`")))
                    }
                };
                match target {
                    Some((f, env)) => {
                        if stack.len() >= MAX_DEPTH {
                            tri!(Err(format!("stack overflow: more than {MAX_DEPTH} nested calls")));
                        }
                        stack.push(frame(p, f, vals, env));
                    }
                    None => {
                        let text = vals.first().map_or(String::new(), |v| show(p, v, true));
                        let result = match writeln!(out, "{text}") {
                            Ok(()) => Value::Ok(Box::new(Value::Unit)),
                            Err(e) => Value::Err(Box::new(Value::Str(e.to_string().into()))),
                        };
                        tri!(write(&mut stack[top], dest, result));
                        stack[top].block = *next;
                    }
                }
            }
            Term::Return(op) => {
                let v = tri!(operand(&stack[top], op));
                let Some(done) = stack.pop() else { return Ok(v) };
                let Some(caller) = stack.last_mut() else { return Ok(v) };
                let Term::Call { writeback, dest, next, .. } = &p.fns[caller.func].blocks[caller.block].term else {
                    tri!(Err(internal("a return to a block that did not call")))
                };
                let copied = writeback.iter().try_for_each(|(i, place)| write(caller, place, done.locals[*i].clone()));
                let stored = copied.and_then(|()| write(caller, dest, v));
                caller.block = *next;
                tri!(stored);
            }
            Term::Panic(msg) => tri!(Err(msg.clone())),
            Term::Unreachable => tri!(Err(internal("reached a block that has no terminator"))),
        }
    }
}

// ---- places ----

fn operand(fr: &Frame, op: &Operand) -> Result<Value, String> {
    match op {
        Operand::Const(v) => Ok(v.clone()),
        Operand::Place(p) => read(fr, p),
    }
}

/// The steps of a place, each a field (`false`) or an index (`true`).
fn steps(fr: &Frame, place: &Place) -> Result<Vec<(bool, usize)>, String> {
    let index = |i: i128| usize::try_from(i).map_err(|_| format!("index {i} is negative"));
    let mut out = Vec::with_capacity(place.proj.len());
    for pr in &place.proj {
        match pr {
            Proj::Field(k) => out.push((false, *k)),
            Proj::Index(op) => match operand(fr, op)? {
                Value::Int(i) => out.push((true, index(i)?)),
                _ => return Err(internal("an index that is not an integer")),
            },
            // The steps a function that returns `ref mut` computed.
            Proj::Path(l) => {
                let Value::Array(pairs) = &fr.locals[*l] else { return Err(internal("a path that is not an array")) };
                for pair in pairs.chunks(2) {
                    match pair {
                        [Value::Int(0), Value::Int(k)] => out.push((false, *k as usize)),
                        [Value::Int(_), Value::Int(i)] => out.push((true, index(*i)?)),
                        _ => return Err(internal("a path with a step that is not a pair of integers")),
                    }
                }
            }
        }
    }
    Ok(out)
}

fn out_of_range(k: usize, len: usize) -> String {
    format!("index {k} is out of range for length {len}")
}

fn read(fr: &Frame, place: &Place) -> Result<Value, String> {
    let mut v = &fr.locals[place.local];
    for (index, k) in steps(fr, place)? {
        v = match (index, v) {
            (false, Value::Tuple(xs) | Value::Struct(_, xs) | Value::Enum(_, _, xs)) => {
                xs.get(k).ok_or_else(|| internal("a field that does not exist"))?
            }
            (false, Value::Some(x) | Value::Ok(x) | Value::Err(x)) => x,
            (true, Value::Array(xs)) => xs.get(k).ok_or_else(|| out_of_range(k, xs.len()))?,
            _ => return Err(internal("a projection on the wrong kind of value")),
        };
    }
    Ok(v.clone())
}

fn write(fr: &mut Frame, place: &Place, value: Value) -> Result<(), String> {
    *slot(fr, place)? = value;
    Ok(())
}

/// The value a place denotes, ready to be changed: shared arrays on the
/// way are copied first.
fn slot<'f>(fr: &'f mut Frame, place: &Place) -> Result<&'f mut Value, String> {
    let steps = steps(fr, place)?;
    let mut v = &mut fr.locals[place.local];
    for (index, k) in steps {
        v = match (index, v) {
            (false, Value::Tuple(xs) | Value::Struct(_, xs) | Value::Enum(_, _, xs)) => {
                xs.get_mut(k).ok_or_else(|| internal("a field that does not exist"))?
            }
            (false, Value::Some(x) | Value::Ok(x) | Value::Err(x)) => &mut **x,
            (true, Value::Array(xs)) => {
                let len = xs.len();
                Rc::make_mut(xs).get_mut(k).ok_or_else(|| out_of_range(k, len))?
            }
            _ => return Err(internal("a projection on the wrong kind of value")),
        };
    }
    Ok(v)
}

// ---- rvalues ----

fn rvalue(p: &Program, consts: &[Value], fr: &Frame, rv: &Rvalue) -> Result<Value, String> {
    let get = |op: &Operand| operand(fr, op);
    let all = |ops: &[Operand]| ops.iter().map(|o| operand(fr, o)).collect::<Result<Vec<Value>, String>>();
    Ok(match rv {
        Rvalue::Use(op) => get(op)?,
        Rvalue::Unary(op, num, v) => unary(*op, *num, get(v)?)?,
        Rvalue::Binary(op, num, a, b) => binary(*op, *num, get(a)?, get(b)?)?,
        Rvalue::CastInt(v, kind) => {
            let (lo, hi) = bounds(kind);
            let fits = |i: i128, shown: String| {
                if lo <= i && i <= hi {
                    Ok(Value::Int(i))
                } else {
                    Err(format!("`as`: the value {shown} does not fit in `{kind}`"))
                }
            };
            match get(v)? {
                Value::Int(i) => fits(i, i.to_string())?,
                Value::Float(f) => float_to_int(f, lo, hi).ok_or_else(|| format!("`as`: the value {f:?} does not fit in `{kind}`"))?,
                Value::F32(f) => float_to_int(f as f64, lo, hi).ok_or_else(|| format!("`as`: the value {f:?} does not fit in `{kind}`"))?,
                Value::Char(c) => fits(c as i128, format!("{c:?}"))?,
                Value::Bool(b) => Value::Int(b as i128),
                _ => return Err(internal("a cast of a value that is not a number")),
            }
        }
        Rvalue::CastFloat(v, single) => match get(v)? {
            Value::Int(i) => float(i as f64, *single),
            Value::Float(f) => float(f, *single),
            Value::F32(f) => float(f as f64, *single),
            _ => return Err(internal("a cast of a value that is not a number")),
        },
        Rvalue::Convert(v, c) => convert(c, get(v)?)?,
        Rvalue::Tuple(ops) => Value::Tuple(all(ops)?),
        Rvalue::Struct(i, ops) => Value::Struct(*i, all(ops)?),
        Rvalue::Enum(e, v, ops) => Value::Enum(*e, *v, all(ops)?),
        Rvalue::Array(ops) => Value::Array(Rc::new(all(ops)?)),
        Rvalue::Repeat(v, n) => match get(n)? {
            Value::Int(n) if (0..=100_000_000).contains(&n) => Value::Array(Rc::new(vec![get(v)?; n as usize])),
            Value::Int(n) => return Err(format!("an array of length {n} cannot be created")),
            _ => return Err(internal("an array length that is not an integer")),
        },
        Rvalue::Some(v) => Value::Some(Box::new(get(v)?)),
        Rvalue::Ok(v) => Value::Ok(Box::new(get(v)?)),
        Rvalue::Err(v) => Value::Err(Box::new(get(v)?)),
        Rvalue::Is(v, tag) => Value::Bool(match get(v)? {
            Value::Enum(_, variant, _) => variant == *tag,
            Value::None | Value::Ok(_) => *tag == 0,
            Value::Some(_) | Value::Err(_) => *tag == 1,
            _ => return Err(internal("a variant test on a value without variants")),
        }),
        Rvalue::Len(v) => Value::Int(match get(v)? {
            Value::Array(xs) => xs.len() as i128,
            Value::Str(s) => s.len() as i128,
            _ => return Err(internal("the length of a value that has none")),
        }),
        Rvalue::Get(base, index) => match (get(base)?, get(index)?) {
            (Value::Array(xs), Value::Int(i)) => {
                let item = usize::try_from(i).ok().and_then(|i| xs.get(i));
                item.map_or(Value::None, |v| Value::Some(Box::new(v.clone())))
            }
            _ => return Err(internal("`get` on a value that is not an array")),
        },
        Rvalue::Push(..) | Rvalue::Pop(_) => return Err(internal("an array operation outside a statement")),
        Rvalue::Slice(base, range) => {
            let range = get(range)?;
            match get(base)? {
                Value::Array(xs) => {
                    let (lo, hi) = slice_bounds(&range, xs.len())?;
                    Value::Array(Rc::new(xs[lo..hi].to_vec()))
                }
                Value::Str(s) => {
                    let (lo, hi) = slice_bounds(&range, s.len())?;
                    let part = s.get(lo..hi).ok_or_else(|| format!("the range {lo}..{hi} splits a character of the string"))?;
                    Value::Str(part.into())
                }
                _ => return Err(internal("a slice of a value that cannot be sliced")),
            }
        }
        Rvalue::Format(parts) => {
            let mut s = String::new();
            for part in parts {
                match part {
                    FmtPart::Lit(text) => s.push_str(text),
                    FmtPart::Val(op) => s.push_str(&show(p, &get(op)?, true)),
                }
            }
            Value::Str(s.into())
        }
        Rvalue::Chars(v) => match get(v)? {
            Value::Str(s) => Value::Array(Rc::new(s.chars().map(Value::Char).collect())),
            _ => return Err(internal("the characters of a value that is not a string")),
        },
        Rvalue::Const(i) => consts.get(*i).cloned().ok_or_else(|| internal("a constant read before it is computed"))?,
        Rvalue::RangeStart(r, _) => match range_parts(&get(r)?)? {
            (Some(lo), _, _) => lo,
            _ => return Err("a range without a start cannot be iterated".into()),
        },
        Rvalue::RangeHas(r, i) => {
            let (_, hi, inclusive) = range_parts(&get(r)?)?;
            Value::Bool(match hi {
                None => true,
                Some(hi) => match compare(&get(i)?, &hi)? {
                    Some(Ordering::Less) => true,
                    Some(Ordering::Equal) => inclusive,
                    _ => false,
                },
            })
        }
        Rvalue::RangeLast(r, i) => {
            let (_, hi, inclusive) = range_parts(&get(r)?)?;
            Value::Bool(inclusive && hi == Some(get(i)?))
        }
        Rvalue::Closure(f, ops) => Value::Closure(*f, Rc::new(all(ops)?)),
        Rvalue::Uninit | Rvalue::Moved => Value::Unit,
        Rvalue::Capture(k) => fr.env.get(*k).cloned().ok_or_else(|| internal("a capture that does not exist"))?,
        Rvalue::Succ(v, num) => match (get(v)?, num) {
            (Value::Int(i), Num::Int(kind)) => Value::Int(int_op(BinOp::Add, kind, i, 1)?),
            (Value::Char(c), _) => {
                let next = if c as u32 == 0xD7FF { 0xE000 } else { c as u32 + 1 };
                Value::Char(char::from_u32(next).ok_or("there is no character after the last one")?)
            }
            _ => return Err(internal("the successor of a value that has none")),
        },
    })
}

fn float(f: f64, single: bool) -> Value {
    if single {
        Value::F32(f as f32)
    } else {
        Value::Float(f)
    }
}

fn float_to_int(f: f64, lo: i128, hi: i128) -> Option<Value> {
    let t = f.trunc();
    (t.is_finite() && t >= lo as f64 && t <= hi as f64).then(|| Value::Int(t as i128)).filter(|v| match v {
        Value::Int(i) => lo <= *i && *i <= hi,
        _ => false,
    })
}

fn range_parts(r: &Value) -> Result<(Option<Value>, Option<Value>, bool), String> {
    let bound = |v: &Value| match v {
        Value::Some(x) => Some((**x).clone()),
        _ => None,
    };
    match r {
        Value::Tuple(parts) if parts.len() == 3 => Ok((bound(&parts[0]), bound(&parts[1]), parts[2] == Value::Bool(true))),
        _ => Err(internal("a range with the wrong shape")),
    }
}

/// Start and end offsets a range selects in something of length `len`.
fn slice_bounds(range: &Value, len: usize) -> Result<(usize, usize), String> {
    let (lo, hi, inclusive) = range_parts(range)?;
    let offset = |v: Option<Value>, default: usize| match v {
        None => Ok(default),
        Some(Value::Int(i)) => usize::try_from(i).map_err(|_| format!("the range bound {i} is negative")),
        Some(_) => Err(internal("a range bound that is not an integer")),
    };
    let closed = inclusive && hi.is_some();
    let lo = offset(lo, 0)?;
    let hi = offset(hi, len)? + closed as usize;
    if lo > hi || hi > len {
        return Err(format!("the range {lo}..{hi} is out of range for length {len}"));
    }
    Ok((lo, hi))
}

fn convert(c: &Conv, v: Value) -> Result<Value, String> {
    let opt = |c: &Option<Box<Conv>>, v: Value| match c {
        Some(c) => convert(c, v),
        None => Ok(v),
    };
    Ok(match (c, v) {
        (Conv::Fit(kind), Value::Int(i)) => {
            let (lo, hi) = bounds(kind);
            if i < lo || i > hi {
                return Err(format!("the value {i} is out of range for `{kind}`"));
            }
            Value::Int(i)
        }
        (Conv::ToFloat(single), Value::Int(i)) => float(i as f64, *single),
        (Conv::ToFloat(single), Value::Float(f)) => float(f, *single),
        (Conv::ToFloat(_), v @ Value::F32(_)) => v,
        (Conv::Wrap(inner), v) => Value::Some(Box::new(opt(inner, v)?)),
        (Conv::Inside(c), Value::Some(x)) => Value::Some(Box::new(convert(c, *x)?)),
        (Conv::Inside(_), Value::None) => Value::None,
        (Conv::Elems(c), Value::Array(xs)) => {
            Value::Array(Rc::new(xs.iter().map(|x| convert(c, x.clone())).collect::<Result<_, _>>()?))
        }
        (Conv::Tuple(cs), Value::Tuple(xs)) => Value::Tuple(
            xs.into_iter()
                .zip(cs)
                .map(|(x, c)| match c {
                    Some(c) => convert(c, x),
                    None => Ok(x),
                })
                .collect::<Result<_, _>>()?,
        ),
        (Conv::Res(ok, _), Value::Ok(x)) => Value::Ok(Box::new(opt(ok, *x)?)),
        (Conv::Res(_, err), Value::Err(x)) => Value::Err(Box::new(opt(err, *x)?)),
        _ => return Err(internal("a conversion applied to the wrong kind of value")),
    })
}

// ---- arithmetic ----

fn compare(a: &Value, b: &Value) -> Result<Option<Ordering>, String> {
    Ok(match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::F32(x), Value::F32(y)) => x.partial_cmp(y),
        (Value::Char(x), Value::Char(y)) => Some(x.cmp(y)),
        (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
        _ => return Err(internal("a comparison of values that have no order")),
    })
}

fn unary(op: UnOp, num: Num, v: Value) -> Result<Value, String> {
    Ok(match (op, num, v) {
        (UnOp::Neg, Num::Int(kind), Value::Int(i)) => Value::Int(int_op(BinOp::Sub, kind, 0, i)?),
        (UnOp::Neg, _, Value::Float(f)) => Value::Float(-f),
        (UnOp::Neg, _, Value::F32(f)) => Value::F32(-f),
        (UnOp::Not, _, Value::Bool(b)) => Value::Bool(!b),
        (UnOp::BitNot, Num::Int(kind), Value::Int(i)) => {
            let (lo, hi) = bounds(kind);
            if lo < 0 {
                Value::Int(!i)
            } else if kind == "u128" {
                return Err(U128_LIMIT.into());
            } else {
                Value::Int(hi - i)
            }
        }
        _ => return Err(internal("a unary operator applied to the wrong kind of value")),
    })
}

fn binary(op: BinOp, num: Num, a: Value, b: Value) -> Result<Value, String> {
    use BinOp::*;
    Ok(match op {
        Eq => Value::Bool(a == b),
        Ne => Value::Bool(a != b),
        Lt | Le | Gt | Ge => {
            let ord = compare(&a, &b)?;
            Value::Bool(match op {
                Lt => ord == Some(Ordering::Less),
                Le => matches!(ord, Some(Ordering::Less | Ordering::Equal)),
                Gt => ord == Some(Ordering::Greater),
                _ => matches!(ord, Some(Ordering::Greater | Ordering::Equal)),
            })
        }
        BitAnd | BitOr | BitXor => match (a, b) {
            (Value::Int(x), Value::Int(y)) => Value::Int(match op {
                BitAnd => x & y,
                BitOr => x | y,
                _ => x ^ y,
            }),
            (Value::Bool(x), Value::Bool(y)) => Value::Bool(match op {
                BitAnd => x & y,
                BitOr => x | y,
                _ => x ^ y,
            }),
            _ => return Err(internal("a bit operator applied to the wrong kind of value")),
        },
        _ => match (num, a, b) {
            (Num::Int(kind), Value::Int(x), Value::Int(y)) => Value::Int(int_op(op, kind, x, y)?),
            (_, Value::Float(x), Value::Float(y)) => Value::Float(float_op(op, x, y)?),
            // Computing in `f64` and rounding once gives the correctly rounded `f32` result.
            (_, Value::F32(x), Value::F32(y)) => Value::F32(float_op(op, x as f64, y as f64)? as f32),
            _ => return Err(internal("an arithmetic operator applied to the wrong kind of value")),
        },
    })
}

fn float_op(op: BinOp, x: f64, y: f64) -> Result<f64, String> {
    Ok(match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div => x / y,
        BinOp::Rem => x % y,
        _ => return Err(internal("an integer operator applied to floats")),
    })
}

const U128_LIMIT: &str = "the interpreter only supports `u128` values below 2^127";

fn int_op(op: BinOp, kind: &'static str, a: i128, b: i128) -> Result<i128, String> {
    use BinOp::*;
    let (lo, hi) = bounds(kind);
    let bits: u32 = match kind {
        "lit" | "i128" | "u128" => 128,
        "isize" | "usize" => 64,
        _ => kind[1..].parse().unwrap_or(64),
    };
    let overflow = || match kind {
        "u128" => U128_LIMIT.to_string(),
        "lit" => format!("integer overflow: the result of `{}` does not fit in 128 bits", op.symbol()),
        _ => format!("integer overflow: the result of `{}` does not fit in `{kind}`", op.symbol()),
    };
    let fit = |r: Option<i128>| match r {
        Some(r) if lo <= r && r <= hi => Ok(r),
        _ => Err(overflow()),
    };
    let wrap = |r: i128| {
        if bits == 128 {
            return if kind == "u128" && r < 0 { Err(overflow()) } else { Ok(r) };
        }
        let low = r & ((1i128 << bits) - 1);
        Ok(if lo < 0 && low > hi { low - (1i128 << bits) } else { low })
    };
    // Saturation: `None` means the exact result does not even fit in 128 bits.
    let saturate = |r: Option<i128>, up: bool| match r {
        Some(r) if kind == "u128" && r >= 0 => Ok(r),
        Some(r) if kind != "u128" => Ok(r.clamp(lo, hi)),
        None if kind != "u128" => Ok(if up { hi } else { lo }),
        Some(_) => Ok(0),
        None => Err(overflow()),
    };
    match op {
        Add => fit(a.checked_add(b)),
        Sub => fit(a.checked_sub(b)),
        Mul => fit(a.checked_mul(b)),
        Div | Rem if b == 0 => Err("division by zero".into()),
        Div => fit(a.checked_div(b)),
        Rem => fit(a.checked_rem(b)),
        AddWrap => wrap(a.wrapping_add(b)),
        SubWrap => wrap(a.wrapping_sub(b)),
        MulWrap => wrap(a.wrapping_mul(b)),
        AddSat => saturate(a.checked_add(b), b > 0),
        SubSat => saturate(a.checked_sub(b), b < 0),
        MulSat => saturate(a.checked_mul(b), (a < 0) == (b < 0)),
        Shl | Shr => {
            let width = if kind == "lit" { 127 } else { bits };
            if b < 0 || b >= width as i128 {
                return Err(format!("a shift by {b} is out of range"));
            }
            if op == Shr {
                Ok(a >> b)
            } else if kind == "lit" {
                let r = a << b;
                if r >> b == a { Ok(r) } else { Err(overflow()) }
            } else {
                wrap(a.wrapping_shl(b as u32))
            }
        }
        _ => Err(internal("a comparison treated as arithmetic")),
    }
}

// ---- display ----

/// Text of a value. Strings and characters are quoted unless at the top level.
pub fn show(p: &Program, v: &Value, top: bool) -> String {
    let list = |xs: &[Value]| xs.iter().map(|x| show(p, x, false)).collect::<Vec<_>>().join(", ");
    match v {
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format!("{f:?}"),
        Value::F32(f) => format!("{f:?}"),
        Value::Bool(b) => b.to_string(),
        Value::Char(c) if top => c.to_string(),
        Value::Char(c) => format!("{c:?}"),
        Value::Str(s) if top => s.to_string(),
        Value::Str(s) => format!("{s:?}"),
        Value::Unit => "()".into(),
        Value::Tuple(xs) => format!("({})", list(xs)),
        Value::Struct(i, xs) if p.structs[*i].0.starts_with("dyn ") && !xs.is_empty() => show(p, &xs[0], top),
        Value::Struct(i, xs) => {
            let (name, fields) = &p.structs[*i];
            let parts: Vec<String> = fields.iter().zip(xs).map(|(f, x)| format!("{f}: {}", show(p, x, false))).collect();
            format!("{name}({})", parts.join(", "))
        }
        Value::Enum(e, variant, xs) => {
            let (name, fields) = &p.enums[*e].1[*variant];
            if xs.is_empty() {
                return name.clone();
            }
            let parts: Vec<String> = fields
                .iter()
                .zip(xs)
                .map(|(f, x)| match f {
                    Some(f) => format!("{f}: {}", show(p, x, false)),
                    None => show(p, x, false),
                })
                .collect();
            format!("{name}({})", parts.join(", "))
        }
        Value::Array(xs) => format!("[{}]", list(xs)),
        Value::None => "None".into(),
        Value::Some(x) => format!("Some({})", show(p, x, false)),
        Value::Ok(x) => show(p, x, top),
        Value::Err(x) => format!("error {}", show(p, x, false)),
        Value::Fn(f) | Value::Closure(f, _) => format!("fn {}", p.fns[*f].name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, mir};

    /// Runs a program. Returns what it printed and what `main` returned.
    fn run_src(src: &str) -> Result<(String, Value), String> {
        let join = |es: Vec<crate::lexer::Error>| es.into_iter().map(|e| e.msg).collect::<Vec<_>>().join("; ");
        let file = crate::load::load("test.mou", src).1.map_err(join)?;
        let (_, info) = check::analyze(&file).map_err(join)?;
        let program = mir::lower(&file, &info).map_err(join)?;
        let main = program.main.ok_or("no main")?;
        let mut out = Vec::new();
        let v = run(&program, main, &mut out).map_err(|p| format!("panic: {}", p.msg))?;
        Ok((String::from_utf8(out).unwrap(), v))
    }

    /// Value of an expression of type `int`.
    fn int(body: &str) -> i128 {
        let generic = "fn first[T](a: T, b: T) -> T { a }\nfn count[T](xs: [T]) -> int {\n var n = 0\n for x in xs { n += 1 }\n n\n}\n";
        match run_src(&format!("{generic}fn main() -> int {{\n{body}\n}}")) {
            Ok((_, Value::Int(i))) => i,
            other => panic!("{body}: {other:?}"),
        }
    }

    fn printed(src: &str) -> String {
        run_src(&format!("import std.io\n{src}")).unwrap_or_else(|e| panic!("{src}: {e}")).0
    }

    fn fails(src: &str, needle: &str) {
        match run_src(src) {
            Err(e) if e.contains(needle) => {}
            other => panic!("{src}: expected an error with `{needle}`, got {other:?}"),
        }
    }

    #[test]
    fn examples_run() {
        let core = printed(&include_str!("../examples/core.mou").replace("import std.io", ""));
        assert_eq!(
            core,
            "final balance: 69\nchecksum: 1091\nneeds 50, has 20\nbalance of 2: 250\nbalance of 9: 0\n\
             account: Account(id: 2, balance: 250, frozen: false)\n"
        );
        let shapes = run_src(include_str!("../examples/shapes.mou")).unwrap().0;
        assert_eq!(shapes, "area = 3.14159\narea = 6.0\n");
        let tour = run_src(include_str!("../examples/tour.mou")).unwrap().0;
        assert!(tour.contains("fib(20) = 6765\n") && tour.ends_with("opt None true\n"), "{tour}");
    }

    #[test]
    fn integer_arithmetic_traps() {
        assert_eq!(int("7 / 2 + -7 % 3 + (1 << 4)"), 18);
        fails("fn f(a: u8, b: u8) -> u8 { a + b }\nfn main() -> int { f(200, 100) as int }", "does not fit in `u8`");
        fails("fn f(a: i8) -> i8 { -a }\nfn main() -> int { f(-128) as int }", "does not fit in `i8`");
        fails("fn f(a: int, b: int) -> int { a / b }\nfn main() -> int { f(1, 0) }", "division by zero");
        fails("fn f(a: int, b: int) -> int { a << b }\nfn main() -> int { f(1, 64) }", "shift by 64");
        fails("fn main() -> int {\n let x: u8 = 200 + 100\n x as int\n}", "out of range for `u8`");
        fails("fn main() -> int { 300 as u8 as int }", "does not fit in `u8`");
        fails("fn main() -> int { -1.5e30 as int }", "does not fit in `i64`");
    }

    #[test]
    fn wrapping_and_saturating() {
        assert_eq!(int("let x: u8 = 250\n (x +% 10) as int"), 4);
        assert_eq!(int("let x: u8 = 250\n (x +| 10) as int"), 255);
        assert_eq!(int("let x: u8 = 3\n (x -| 10) as int"), 0);
        assert_eq!(int("let x: i8 = 127\n (x +% 1) as int"), -128);
        assert_eq!(int("let x: i8 = -100\n (x *| 100) as int"), -128);
        assert_eq!(int("let x: u32 = 0xFFFF_FFFF\n (x *% x) as int"), 1);
        assert_eq!(int("let x: u8 = 0b1010_0000\n (x << 1) as int + (~x) as int"), 0x40 + 0x5F);
    }

    #[test]
    fn mut_arguments_write_back() {
        let decls = "struct P { x: int, y: int }\nfn bump(p: mut P, by: int = 10) { p.x += by }\n\
                     fn swap(a: mut int, b: mut int) {\n let t = a\n a = b\n b = t\n}\n\
                     impl P { fn flip(mut self) { swap(mut self.x, mut self.y) } }\n";
        let src = format!(
            "{decls}fn main() ! io.Error {{\n var ps = [P(0, 1), P(5, 6)]\n bump(mut ps[1])\n bump(mut ps[0], 2)\n \
             swap(mut ps[0].y, mut ps[1].y)\n ps[1].flip()\n io.print(\"{{ps}}\")?\n}}"
        );
        assert_eq!(printed(&src), "[P(x: 2, y: 6), P(x: 1, y: 15)]\n");
    }

    #[test]
    fn patterns_and_control_flow() {
        let classify = "fn c(n: int) -> int {\n match n {\n 0 => 10\n 1 | 2 => 20\n 3..6 => 30\n x if x < 0 => 40\n _ => 50\n }\n}\n";
        let src = format!("{classify}fn main() -> int {{ c(0) + c(2) * 10 + c(5) * 100 + c(-1) * 1000 + c(6) * 10000 }}");
        assert_eq!(run_src(&src).unwrap().1, Value::Int(10 + 200 + 3000 + 40000 + 500000));
        assert_eq!(int("var t = 0\n for i in 1..=4 { t += i }\n for i in 0..3 { t += 10 }\n t"), 40);
        assert_eq!(int("var t = 0\n for i in 0..10 {\n if i % 2 == 0 { continue }\n if i > 6 { break }\n t += i\n }\n t"), 9);
        assert_eq!(int("var n = 0\n let r = loop {\n n += 3\n if n > 10 { break n * 2 }\n }\n r"), 24);
        assert_eq!(int("var o: int? = 5\n var steps = 0\n while let Some(n) = o {\n steps += 1\n o = if n > 3 { n - 1 } else { None }\n }\n steps"), 3);
        assert_eq!(int("let (a, b) = (3, 4)\n let xs = [a, b, 5]\n match xs {\n [x, .., z] => x * z\n _ => 0\n }"), 15);
        assert_eq!(int("var n = 0\n for c in \"año\" { n += c as int }\n n"), 97 + 241 + 111);
        assert_eq!(int("let t = true\n if t && (false || t) { 1 } else { 2 }"), 1);
    }

    #[test]
    fn optionals_results_and_defer() {
        let decls = "enum E { Neg(n: int), Zero }\nfn check(n: int) -> int ! E {\n if n < 0 { return E.Neg(n) }\n \
                     if n == 0 { return E.Zero }\n n\n}\nfn both(a: int, b: int) -> int ! E { check(a)? + check(b)? }\n\
                     fn first(xs: [int]) -> int? {\n for x in xs { return x }\n None\n}\n\
                     fn twice(xs: [int]) -> int? {\n let v = first(xs)?\n v * 2\n}\n";
        let src = format!(
            "{decls}fn main() ! io.Error {{\n let a = both(1, 2) catch e {{ 0 }}\n let b = both(1, -5) catch e {{\n match e {{\n \
             Neg(n) => n\n Zero => 0\n }}\n }}\n io.print(\"{{a}} {{b}} {{twice([4, 5])}} {{twice([])}}\")?\n}}"
        );
        assert_eq!(printed(&src), "3 -5 Some(8) None\n");
        let order = "fn f() ! io.Error {\n defer io.print(\"one\") catch e {}\n defer io.print(\"two\") catch e {}\n io.print(\"body\")?\n}\n\
                     fn main() ! io.Error { f()? }";
        assert_eq!(printed(order), "body\ntwo\none\n");
        let err = run_src("enum E { Bad }\nfn g() ! E { return E.Bad }\nfn main() ! E { g()? }").unwrap().1;
        assert!(matches!(err, Value::Err(_)), "{err:?}");
    }

    #[test]
    fn floats_strings_and_values() {
        let src = "struct P { x: f64 }\nenum S { A, B(n: int) }\nlet N: u8 = 200\nfn main() ! io.Error {\n let f: f32 = 0.1\n let s = \"hola mundo\"\n let t = (1, \"a\", 'c')\n \
                   io.print(\"{7.0 / 2} {f} {1.0 / 3.0} {3 as f64} {3.99 as int}\")?\n \
                   io.print(\"{s[..4]}|{s[5..]}|{s.len}|{[1, 2, 3, 4][1..=2]}|{N}\")?\n \
                   io.print(\"{P(1)} {S.A} {S.B(2)} {t} {S.A == S.A}\")?\n}";
        assert_eq!(
            printed(src),
            "3.5 0.1 0.3333333333333333 3.0 3\nhola|mundo|10|[2, 3]|200\nP(x: 1.0) A B(n: 2) (1, \"a\", 'c') true\n"
        );
        fails("fn main() -> int {\n let s = \"ñu\"\n s[1..].len as int\n}", "splits a character");
    }

    #[test]
    fn runtime_failures_are_reported() {
        fails("fn main() -> int {\n let xs = [1, 2, 3]\n var i = 0\n while true { i += xs[i] }\n i\n}", "index 3 is out of range for length 3");
        fails("fn f(n: int) -> int { f(n + 1) }\nfn main() -> int { f(0) }", "stack overflow");
        fails("fn m(xs: [f64]) -> f64 where xs.len > 0 { xs[0] }\nfn main() { m([]) }", "`where` condition of `m`");
        fails("fn main() {\n let f = fn(x) { x }\n}", "parameters have no type");
        fails("fn main() { panic(\"stop {1 + 1}\") }", "stop 2");
        fails("import std.list\nfn main() {\n var l = list.List.of([1])\n l.insert(3, 0)\n}", "List.insert: position 3 is out of range for length 1");
        fails("import std.text\nfn main() -> int { text.nope(1) }", "the module `std.text` has no `nope`");
        fails("import std.text\nfn main() -> int { text.width('a') }", "`width` is private to the module `std.text`");
        fails("import std.nope\nfn main() { }", "the standard library has no module `std.nope`");
    }

    #[test]
    fn generics_traits_and_closures() {
        let traits = run_src(include_str!("../examples/traits.mou")).unwrap().0;
        assert_eq!(
            traits,
            "9 7\nVersion(major: 1, minor: 10)\nSome(8) Some(Version(major: 2, minor: 0))\n(1, 'x') (2.5, true)\nlargest is v0.2\n"
        );
        let closures = run_src(include_str!("../examples/closures.mou")).unwrap().0;
        assert_eq!(closures, "101 201 15\nitem 1 of 100\nitem 2 of 100\nn = 101\n");
        let generic = run_src(include_str!("../examples/generic_types.mou")).unwrap().0;
        assert_eq!(
            generic,
            "Box <3>\n3 10\nn=10\nBox(v: Box(v: 2.5)) 2.5\n7 4 false\nJust(9) 9\n44\nNothing\n\
             4 Node(Node(Node(Leaf, 1, Leaf), 2, Leaf), 5, Node(Leaf, 8, Leaf))\n3\n"
        );
        let library = run_src(include_str!("../examples/library.mou")).unwrap().0;
        assert!(library.starts_with("5 [5, 3, 9, 1, 7] Some(5) Some(7) None\n5 [100, 3, 9, 1, 7]\n[1, 3, 7, 9, 100] Some(3) false Some(1) Some(100)\n"));
        assert!(library.contains("\nand: 2\nbat: 1\ncat: 1\nhat: 1\nthe: 3\n"));
        assert!(library.ends_with("Some(9223372036854775807) None Some(-9223372036854775808)\n[1, 2, 3]\n"));
        // Each use of a generic function with new types is its own instance.
        assert_eq!(int("first(3, 4) + first(2.5, 1.5) as int + count([1, 2, 3]) + count(['a'])"), 3 + 2 + 3 + 1);
    }

    #[test]
    fn functions_are_values() {
        let src = "fn twice(f: fn(int) -> int, x: int) -> int { f(f(x)) }\nfn inc(x: int) -> int { x + 1 }\nfn main() -> int { twice(inc, 5) }";
        assert_eq!(run_src(src).unwrap().1, Value::Int(7));
        assert_eq!(run_src("fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\nfn main() -> int { fib(20) }").unwrap().1, Value::Int(6765));
    }
}
