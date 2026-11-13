//! Effects: what running a function can do besides computing its result.
//!
//! Four effects are inferred for every function, on the MIR: `alloc`
//! (it can create or grow data on the heap), `panic` (it can stop the
//! program), `io` (it can reach outside the program) and `unsafe` (it
//! runs a function of C). A function has the effects of its own
//! operations and of everything it calls.
//!
//! `@noalloc`, `@nopanic` and `@noio` on a declaration rule an effect out,
//! and a `system` module rules out `alloc` for all its functions. Each
//! operation that would have the effect is then an error.
//!
//! An operation that panics only for some values (an index, `+`, a
//! division, `as`) does not count when the values are proved to be good.
//! The proof is a forward data-flow analysis that knows, at each point,
//! a range for every integer, which integers are below the length of
//! which arrays, and a minimum for the length of arrays. It learns from
//! the conditions of `if`, `while`, `for`, `assert` and `where`.

use crate::lexer::{Error, Span};
use crate::mir::*;

pub const ALLOC: u8 = NOALLOC;
pub const PANIC: u8 = NOPANIC;
pub const IO: u8 = NOIO;
/// It runs code the compiler does not check: a function of C.
pub const UNSAFE: u8 = NOUNSAFE;
/// It can wait: for a task, or for input.
pub const BLOCK: u8 = NOBLOCK;

const MIN: i128 = i128::MIN;
const MAX: i128 = i128::MAX;
/// No array is longer than this.
const LEN_MAX: i128 = i64::MAX as i128;
const TOP: (i128, i128) = (MIN, MAX);
const WHERE: &str = "a `where` condition of";

/// An array the analysis can name: a local, or a field of a field of one.
type Key = (Local, Vec<usize>);
type Cmp = (BinOp, Operand, Operand);

#[derive(Clone, PartialEq)]
struct State {
    /// Range of each local, when it holds an integer.
    iv: Vec<(i128, i128)>,
    /// Locals known not to be zero.
    nz: Vec<bool>,
    /// Another local that holds the same value.
    eq: Vec<Option<Local>>,
    /// The array whose length the local holds.
    len_of: Vec<Option<Key>>,
    /// The comparison whose result the local holds.
    cmp: Vec<Option<Cmp>>,
    /// Locals that are below the length of an array.
    below: Vec<(Local, Key)>,
    /// Arrays known to have at least so many items.
    min_len: Vec<(Key, i128)>,
    /// Range of the items of each local, when it holds an array of integers.
    items: Vec<(i128, i128)>,
}

/// What is known about a value that is about to be stored.
struct Val {
    iv: (i128, i128),
    nz: bool,
    eq: Option<Local>,
    len_of: Option<Key>,
    cmp: Option<Cmp>,
    below: Vec<Key>,
    /// For a new array, the least number of items it has.
    min_len: Option<i128>,
    /// The array it is a copy of.
    from: Option<Key>,
    /// For an array of integers, the range of its items.
    items: (i128, i128),
}

impl Val {
    fn top() -> Val {
        Val { iv: TOP, nz: false, eq: None, len_of: None, cmp: None, below: Vec::new(), min_len: None, from: None, items: TOP }
    }

    fn range(iv: (i128, i128)) -> Val {
        Val { iv, ..Val::top() }
    }
}

fn key(p: &Place) -> Option<Key> {
    let fields: Option<Vec<usize>> = p
        .proj
        .iter()
        .map(|s| match s {
            Proj::Field(k) => Some(*k),
            _ => None,
        })
        .collect();
    Some((p.local, fields?))
}

fn bare(op: &Operand) -> Option<Local> {
    match op {
        Operand::Place(p) if p.proj.is_empty() => Some(p.local),
        _ => None,
    }
}

fn simple(op: &Operand) -> bool {
    matches!(op, Operand::Const(_)) || bare(op).is_some()
}

fn mentions(op: &Operand, l: Local) -> bool {
    bare(op) == Some(l)
}

fn meet(a: (i128, i128), b: (i128, i128)) -> (i128, i128) {
    (a.0.max(b.0), a.1.min(b.1))
}

fn inside(a: (i128, i128), b: (i128, i128)) -> bool {
    b.0 <= a.0 && a.1 <= b.1
}

fn negated(op: BinOp) -> Option<BinOp> {
    use BinOp::*;
    Some(match op {
        Lt => Ge,
        Le => Gt,
        Gt => Le,
        Ge => Lt,
        Eq => Ne,
        Ne => Eq,
        _ => return None,
    })
}

impl State {
    fn top(locals: usize) -> State {
        State {
            iv: vec![TOP; locals],
            nz: vec![false; locals],
            eq: vec![None; locals],
            len_of: vec![None; locals],
            cmp: vec![None; locals],
            below: Vec::new(),
            min_len: Vec::new(),
            items: vec![TOP; locals],
        }
    }

    fn root(&self, l: Local) -> Local {
        self.eq[l].unwrap_or(l)
    }

    /// The locals known to hold the same value as `l`, itself included.
    fn class(&self, l: Local) -> Vec<Local> {
        let r = self.root(l);
        (0..self.eq.len()).filter(|x| self.root(*x) == r).collect()
    }

    fn least_len(&self, k: &Key) -> i128 {
        self.min_len.iter().find(|(q, _)| q == k).map_or(0, |(_, n)| *n)
    }

    fn set_min_len(&mut self, k: Key, n: i128) {
        if n <= 0 {
            return;
        }
        match self.min_len.iter_mut().find(|(q, _)| *q == k) {
            Some((_, old)) => *old = (*old).max(n),
            None => self.min_len.push((k, n)),
        }
    }

    fn is_below(&self, l: Local, k: &Key) -> bool {
        self.below.iter().any(|(x, q)| *x == l && q == k)
    }

    fn add_below(&mut self, l: Local, k: Key) {
        if !self.is_below(l, &k) {
            self.below.push((l, k));
        }
    }

    fn val(&self, op: &Operand) -> Val {
        match op {
            Operand::Const(Value::Int(n)) => Val { nz: *n != 0, ..Val::range((*n, *n)) },
            Operand::Const(_) => Val::top(),
            Operand::Place(p) if p.proj.is_empty() => {
                let l = p.local;
                Val {
                    iv: self.iv[l],
                    nz: self.nz[l],
                    eq: Some(self.root(l)),
                    len_of: self.len_of[l].clone(),
                    cmp: self.cmp[l].clone(),
                    below: self.below.iter().filter(|(x, _)| *x == l).map(|(_, k)| k.clone()).collect(),
                    min_len: None,
                    from: Some((l, Vec::new())),
                    items: self.items[l],
                }
            }
            Operand::Place(p) => Val { from: key(p), ..Val::top() },
        }
    }

    fn forget_key(&mut self, dead: impl Fn(&Key) -> bool) {
        self.below.retain(|(_, k)| !dead(k));
        self.min_len.retain(|(k, _)| !dead(k));
        for l in &mut self.len_of {
            if l.as_ref().is_some_and(&dead) {
                *l = None;
            }
        }
    }

    /// A write to a place may change the length of the arrays it holds.
    fn kill_keys(&mut self, p: &Place) {
        let local = p.local;
        if p.proj.iter().any(|s| matches!(s, Proj::Path(_))) {
            self.forget_key(|k| k.0 == local);
        } else if let Some((_, fields)) = key(p) {
            self.forget_key(|k| k.0 == local && k.1.starts_with(&fields));
        }
    }

    fn kill_local(&mut self, l: Local) {
        self.iv[l] = TOP;
        self.items[l] = TOP;
        self.nz[l] = false;
        self.len_of[l] = None;
        self.cmp[l] = None;
        for x in 0..self.eq.len() {
            if self.eq[x] == Some(l) || x == l {
                self.eq[x] = None;
            }
            if self.cmp[x].as_ref().is_some_and(|(_, a, b)| mentions(a, l) || mentions(b, l)) {
                self.cmp[x] = None;
            }
        }
        self.below.retain(|(x, _)| *x != l);
        self.forget_key(|k| k.0 == l);
    }

    fn write(&mut self, p: &Place, v: Val, hint: Option<Scalar>) {
        if !p.proj.is_empty() {
            self.kill_keys(p);
            let l = p.local;
            self.items[l] = TOP;
            for x in 0..self.eq.len() {
                if self.eq[x] == Some(l) || x == l {
                    self.eq[x] = None;
                }
            }
            return;
        }
        let l = p.local;
        // What is known about the array it copies holds for the copy.
        let mut below = Vec::new();
        let mut lens = Vec::new();
        if let Some((src, fields)) = v.from.as_ref().filter(|k| k.0 != l) {
            let under = |k: &Key| (k.0 == *src && k.1.starts_with(fields)).then(|| k.1[fields.len()..].to_vec());
            below.extend(self.below.iter().filter(|(x, _)| *x != l).filter_map(|(x, k)| Some((*x, under(k)?))));
            lens.extend(self.min_len.iter().filter_map(|(k, n)| Some((under(k)?, *n))));
        }
        self.kill_local(l);
        self.iv[l] = match hint {
            Some(Scalar::Int(k)) => {
                let m = meet(v.iv, bounds(k));
                if m.0 <= m.1 { m } else { bounds(k) }
            }
            _ => v.iv,
        };
        self.nz[l] = v.nz;
        self.items[l] = v.items;
        self.eq[l] = v.eq.filter(|r| *r != l);
        self.len_of[l] = v.len_of.filter(|k| k.0 != l);
        self.cmp[l] = v.cmp.filter(|(_, a, b)| !mentions(a, l) && !mentions(b, l));
        for k in v.below {
            if k.0 != l {
                self.add_below(l, k);
            }
        }
        for (x, fields) in below {
            self.add_below(x, (l, fields));
        }
        for (fields, n) in lens {
            self.set_min_len((l, fields), n);
        }
        if let Some(n) = v.min_len {
            self.set_min_len((l, Vec::new()), n);
        }
    }

    /// Narrows the range of an operand and of what equals it.
    fn bound(&mut self, op: &Operand, lo: Option<i128>, hi: Option<i128>) {
        let Some(l) = bare(op) else { return };
        for x in self.class(l) {
            if let Some(lo) = lo {
                self.iv[x].0 = self.iv[x].0.max(lo);
            }
            if let Some(hi) = hi {
                self.iv[x].1 = self.iv[x].1.min(hi);
            }
            if let Some(k) = self.len_of[x].clone() {
                self.set_min_len(k, self.iv[x].0);
            }
        }
    }

    /// Takes in that a comparison holds.
    fn refine(&mut self, op: BinOp, a: &Operand, b: &Operand) {
        let (ia, ib) = (self.val(a).iv, self.val(b).iv);
        let fin = |n: i128| (n != MIN && n != MAX).then_some(n);
        match op {
            BinOp::Lt => {
                self.bound(a, None, fin(ib.1).map(|n| n - 1));
                self.bound(b, fin(ia.0).map(|n| n + 1), None);
                let len = bare(b).and_then(|l| self.len_of[l].clone());
                if let (Some(l), Some(k)) = (bare(a), len) {
                    for x in self.class(l) {
                        self.add_below(x, k.clone());
                    }
                }
            }
            BinOp::Le => {
                self.bound(a, None, fin(ib.1));
                self.bound(b, fin(ia.0), None);
            }
            BinOp::Gt => self.refine(BinOp::Lt, b, a),
            BinOp::Ge => self.refine(BinOp::Le, b, a),
            BinOp::Eq => {
                self.bound(a, fin(ib.0), fin(ib.1));
                self.bound(b, fin(ia.0), fin(ia.1));
                if let (Some(x), Some(y)) = (bare(a), bare(b)) {
                    let facts: Vec<(Local, Key)> = self.below.iter().filter(|(l, _)| *l == x || *l == y).cloned().collect();
                    for (l, k) in facts {
                        self.add_below(if l == x { y } else { x }, k);
                    }
                    if self.nz[x] || self.nz[y] {
                        (self.nz[x], self.nz[y]) = (true, true);
                    }
                }
            }
            BinOp::Ne => {
                for (x, ix, iy) in [(a, ia, ib), (b, ib, ia)] {
                    if iy.0 != iy.1 {
                        continue;
                    }
                    let n = iy.0;
                    if let (0, Some(l)) = (n, bare(x)) {
                        for c in self.class(l) {
                            self.nz[c] = true;
                        }
                    }
                    if ix.0 == n {
                        self.bound(x, Some(n + 1), None);
                    }
                    if ix.1 == n {
                        self.bound(x, None, Some(n - 1));
                    }
                }
            }
            _ => {}
        }
    }

    /// Whether a comparison is known to hold or not to hold.
    fn decide(&self, op: BinOp, a: &Operand, b: &Operand) -> Option<bool> {
        let (x, y) = (self.val(a), self.val(b));
        match op {
            BinOp::Lt => {
                let len = bare(b).and_then(|l| self.len_of[l].as_ref());
                if x.iv.1 < y.iv.0 || matches!((bare(a), len), (Some(l), Some(k)) if self.is_below(l, k)) {
                    Some(true)
                } else if x.iv.0 >= y.iv.1 {
                    Some(false)
                } else {
                    None
                }
            }
            BinOp::Le => {
                if x.iv.1 <= y.iv.0 {
                    Some(true)
                } else if x.iv.0 > y.iv.1 {
                    Some(false)
                } else {
                    None
                }
            }
            BinOp::Gt => self.decide(BinOp::Lt, b, a),
            BinOp::Ge => self.decide(BinOp::Le, b, a),
            BinOp::Eq => {
                let zero = |v: &Val| v.iv == (0, 0);
                if x.iv.0 == x.iv.1 && x.iv == y.iv {
                    Some(true)
                } else if x.iv.1 < y.iv.0 || y.iv.1 < x.iv.0 || (x.nz && zero(&y)) || (y.nz && zero(&x)) {
                    Some(false)
                } else {
                    None
                }
            }
            BinOp::Ne => self.decide(BinOp::Eq, a, b).map(|r| !r),
            _ => None,
        }
    }

    fn feasible(&self) -> bool {
        self.iv.iter().all(|(lo, hi)| lo <= hi)
    }

    fn join(&self, o: &State) -> State {
        let same = |a: &Option<Cmp>, b: &Option<Cmp>| if a == b { a.clone() } else { None };
        State {
            iv: self.iv.iter().zip(&o.iv).map(|(a, b)| (a.0.min(b.0), a.1.max(b.1))).collect(),
            nz: self.nz.iter().zip(&o.nz).map(|(a, b)| *a && *b).collect(),
            eq: self.eq.iter().zip(&o.eq).map(|(a, b)| if a == b { *a } else { None }).collect(),
            len_of: self.len_of.iter().zip(&o.len_of).map(|(a, b)| if a == b { a.clone() } else { None }).collect(),
            cmp: self.cmp.iter().zip(&o.cmp).map(|(a, b)| same(a, b)).collect(),
            below: self.below.iter().filter(|f| o.below.contains(f)).cloned().collect(),
            min_len: self.min_len.iter().filter_map(|(k, n)| Some((k.clone(), (*n).min(o.least_len(k)))).filter(|(_, n)| *n > 0)).collect(),
            items: self.items.iter().zip(&o.items).map(|(a, b)| (a.0.min(b.0), a.1.max(b.1))).collect(),
        }
    }

    /// Gives up on what keeps changing around a loop.
    fn widen(&mut self, old: &State) {
        for (new, old) in self.iv.iter_mut().zip(&old.iv) {
            if new.0 < old.0 {
                new.0 = MIN;
            }
            if new.1 > old.1 {
                new.1 = MAX;
            }
        }
        self.min_len.retain(|(k, n)| *n >= old.least_len(k));
        for (new, old) in self.items.iter_mut().zip(&old.items) {
            if *new != *old {
                *new = TOP;
            }
        }
    }
}

/// Range of the result of an integer operation, and whether it can panic.
fn arith(op: BinOp, kind: &'static str, x: (i128, i128), y: (i128, i128), y_nz: bool) -> ((i128, i128), bool) {
    use BinOp::*;
    let b = bounds(kind);
    let clamp = |v: (i128, i128)| {
        let m = meet(v, b);
        if m.0 <= m.1 { m } else { b }
    };
    let (x, y) = (clamp(x), clamp(y));
    let wide = matches!(kind, "lit" | "i128" | "u128");
    let bits: i128 = match kind {
        "lit" => 127,
        "i128" | "u128" => 128,
        "isize" | "usize" => 64,
        _ => kind[1..].parse().unwrap_or(64),
    };
    let exact: Option<(i128, i128)> = match op {
        Add | AddWrap | AddSat => x.0.checked_add(y.0).zip(x.1.checked_add(y.1)),
        Sub | SubWrap | SubSat => x.0.checked_sub(y.1).zip(x.1.checked_sub(y.0)),
        Mul | MulWrap | MulSat => {
            let ps = [x.0.checked_mul(y.0), x.0.checked_mul(y.1), x.1.checked_mul(y.0), x.1.checked_mul(y.1)];
            ps.iter().copied().collect::<Option<Vec<i128>>>().map(|v| (v.iter().copied().min().unwrap_or(0), v.iter().copied().max().unwrap_or(0)))
        }
        _ => None,
    };
    match op {
        Add | Sub | Mul => match exact {
            Some(e) if inside(e, b) => (e, false),
            Some(e) => (clamp(e), true),
            None => (b, true),
        },
        AddWrap | SubWrap | MulWrap => (exact.filter(|e| inside(*e, b)).unwrap_or(b), kind == "u128"),
        AddSat | SubSat | MulSat => match exact {
            Some(e) => ((e.0.clamp(b.0, b.1), e.1.clamp(b.0, b.1)), kind == "u128"),
            None => (b, wide),
        },
        Div | Rem => {
            let zero = !y_nz && y.0 <= 0 && 0 <= y.1;
            let wraps = b.0 < 0 && x.0 == b.0 && y.0 <= -1 && -1 <= y.1;
            let r = if y.0 >= 1 && x.0 >= 0 {
                if op == Div { (0, x.1) } else { (0, x.1.min(y.1 - 1)) }
            } else if y.0 >= 1 {
                (x.0.min(0), x.1.max(0))
            } else {
                b
            };
            (r, zero || wraps)
        }
        Shl | Shr => {
            let bad = y.0 < 0 || y.1 >= bits;
            let r = if op == Shr && x.0 >= 0 { (0, x.1) } else { b };
            (r, bad || (op == Shl && kind == "lit"))
        }
        BitAnd if y.0 >= 0 => ((0, if x.0 >= 0 { x.1.min(y.1) } else { y.1 }), false),
        BitAnd if x.0 >= 0 => ((0, x.1), false),
        _ => (b, false),
    }
}

fn has_fit(c: &Conv) -> bool {
    match c {
        Conv::Fit(_) => true,
        Conv::ToFloat(_) => false,
        Conv::Wrap(c) => c.as_deref().is_some_and(has_fit),
        Conv::Inside(c) | Conv::Elems(c) => has_fit(c),
        Conv::Tuple(cs) => cs.iter().flatten().any(has_fit),
        Conv::Res(a, b) => a.as_deref().is_some_and(has_fit) || b.as_deref().is_some_and(has_fit),
    }
}

fn has_elems(c: &Conv) -> bool {
    match c {
        Conv::Elems(_) => true,
        Conv::Fit(_) | Conv::ToFloat(_) => false,
        Conv::Wrap(c) => c.as_deref().is_some_and(has_elems),
        Conv::Inside(c) => has_elems(c),
        Conv::Tuple(cs) => cs.iter().flatten().any(has_elems),
        Conv::Res(a, b) => a.as_deref().is_some_and(has_elems) || b.as_deref().is_some_and(has_elems),
    }
}

/// An operation that has an effect.
struct Site {
    span: Span,
    effect: u8,
    what: String,
}

#[derive(Default)]
struct Found {
    sites: Vec<Site>,
    /// The functions it calls, and where.
    calls: Vec<(Span, usize)>,
    /// Where it calls a function value: the calls from there are the
    /// functions that value can be.
    values: Vec<Span>,
    /// Whether one of its `where` conditions can fail.
    unproved: bool,
}

type Notes = Vec<(u8, String)>;

/// The indexes of a place that are not proved to be in range.
fn indexes(st: &State, p: &Place, notes: &mut Notes) {
    for (j, step) in p.proj.iter().enumerate() {
        match step {
            Proj::Field(_) => {}
            Proj::Path(_) => notes.push((PANIC, "a place computed by a `ref mut` function is not proved to exist".into())),
            Proj::Index(i) => {
                reads(st, i, notes);
                let base = Place { local: p.local, proj: p.proj[..j].to_vec() };
                let proved = key(&base).is_some_and(|k| {
                    let v = st.val(i);
                    v.iv.0 >= 0 && (v.iv.1 < st.least_len(&k) || v.below.contains(&k))
                });
                if !proved {
                    notes.push((PANIC, "this index is not proved to be in range: compare it with the length first, or use `get`".into()));
                }
            }
        }
    }
}

fn reads(st: &State, op: &Operand, notes: &mut Notes) {
    if let Operand::Place(p) = op {
        indexes(st, p, notes);
    }
}

struct Walk<'a> {
    p: &'a Program,
    f: &'a Func,
    wheres: &'a [bool],
    /// The functions the program uses as values, in order.
    taken: &'a [(usize, Option<(usize, usize)>)],
}

/// The functions that are used as a value somewhere in the program: by
/// their name, as a closure, or in the table of a `dyn` value. A call to
/// a function value can only reach one of these.
fn taken(p: &Program) -> Vec<(usize, Option<(usize, usize)>)> {
    let mut seen = vec![false; p.fns.len()];
    let mut note = |o: &Operand| {
        if let Operand::Const(Value::Fn(f)) = o {
            seen[*f] = true;
        }
    };
    let mut closures = Vec::new();
    // The methods a `dyn` value carries: the function, and which method
    // of which `dyn` it is there.
    let mut methods: Vec<(usize, Option<(usize, usize)>)> = Vec::new();
    for f in &p.fns {
        for b in &f.blocks {
            for s in &b.stmts {
                match &s.rv {
                    Rvalue::Struct(i, ops) if *i >= p.dyn_from => {
                        for (k, o) in ops.iter().enumerate().skip(1) {
                            match o {
                                Operand::Const(Value::Fn(g)) if !methods.contains(&(*g, Some((*i, k - 1)))) => methods.push((*g, Some((*i, k - 1)))),
                                Operand::Const(Value::Fn(_)) => {}
                                other => note(other),
                            }
                        }
                        note(&ops[0]);
                    }
                    Rvalue::Closure(g, ops) => {
                        // The closure of a task is run by `spawn`, which
                        // counts it there: nothing else can call it.
                        let spawned = matches!(&b.term, Term::Call { callee: Callee::Native("task.spawn"), args, .. } if args.first() == Some(&Operand::Place(s.place.clone())));
                        if !spawned {
                            closures.push(*g);
                        }
                        ops.iter().for_each(&mut note);
                    }
                    rv => crate::cgen::each_operand(rv, &mut note),
                }
            }
            match &b.term {
                Term::Branch(o, _, _) | Term::Return(o) => note(o),
                Term::Call { callee, args, .. } => {
                    args.iter().for_each(&mut note);
                    if let Callee::Indirect(o, _) = callee {
                        note(o);
                    }
                }
                _ => {}
            }
        }
    }
    for g in closures {
        seen[g] = true;
    }
    let mut out: Vec<(usize, Option<(usize, usize)>)> = (0..p.fns.len()).filter(|f| seen[*f]).map(|f| (f, None)).collect();
    methods.sort();
    out.extend(methods);
    out
}

impl Walk<'_> {
    fn hint(&self, l: Local) -> Option<Scalar> {
        self.f.hints.get(l).copied().flatten().or_else(|| self.f.scalars.get(l).copied().flatten())
    }

    fn eval(&self, st: &State, rv: &Rvalue, notes: &mut Notes) -> Val {
        let int_range = |num: &Num| match num {
            Num::Int(k) => bounds(k),
            _ => TOP,
        };
        match rv {
            Rvalue::Use(a) => {
                reads(st, a, notes);
                st.val(a)
            }
            Rvalue::Unary(op, num, a) => {
                reads(st, a, notes);
                let v = st.val(a);
                match (op, num) {
                    (UnOp::Neg, Num::Int(k)) => {
                        let (r, bad) = arith(BinOp::Sub, k, (0, 0), v.iv, false);
                        if bad {
                            notes.push((PANIC, format!("`-` on `{k}` can overflow here")));
                        }
                        Val { nz: v.nz, ..Val::range(r) }
                    }
                    (UnOp::Not, _) => Val { cmp: v.cmp.and_then(|(op, a, b)| Some((negated(op)?, a, b))), ..Val::top() },
                    (UnOp::BitNot, Num::Int("u128")) => {
                        notes.push((PANIC, "`~` on `u128` can panic here".into()));
                        Val::top()
                    }
                    _ => Val::range(int_range(num)),
                }
            }
            Rvalue::Binary(op, num, a, b) => {
                reads(st, a, notes);
                reads(st, b, notes);
                if negated(*op).is_some() {
                    let cmp = (simple(a) && simple(b)).then(|| (*op, a.clone(), b.clone()));
                    return Val { cmp, ..Val::top() };
                }
                let Num::Int(k) = num else { return Val::top() };
                let (x, y) = (st.val(a), st.val(b));
                let y_nz = y.nz || y.iv.0 > 0 || y.iv.1 < 0;
                let (r, bad) = arith(*op, k, x.iv, y.iv, y_nz);
                if bad {
                    let what = match op {
                        BinOp::Div | BinOp::Rem if !y_nz => format!("this `{}` can divide by zero: check the divisor first", op.symbol()),
                        BinOp::Shl | BinOp::Shr => format!("the shift of this `{}` is not proved to be in range", op.symbol()),
                        BinOp::Add | BinOp::Sub | BinOp::Mul => {
                            let s = op.symbol();
                            format!("`{s}` on `{k}` can overflow here: use `{s}%` or `{s}|`, or check the range first")
                        }
                        _ => format!("`{}` on `{k}` can overflow here", op.symbol()),
                    };
                    notes.push((PANIC, what));
                }
                let mut v = Val::range(r);
                v.nz = r.0 > 0 || r.1 < 0;
                let len = bare(b).and_then(|l| st.len_of[l].clone());
                // `i % xs.len` and `xs.len - 1` are below the length.
                match (op, len, x.len_of) {
                    (BinOp::Rem, Some(k), _) if x.iv.0 >= 0 => v.below.push(k),
                    (BinOp::Sub, _, Some(k)) if y.iv.0 >= 1 => v.below.push(k),
                    _ => {}
                }
                v
            }
            Rvalue::CastInt(a, k) => {
                reads(st, a, notes);
                let v = st.val(a);
                let b = bounds(k);
                if !inside(v.iv, b) {
                    notes.push((PANIC, format!("the value of this `as` is not proved to fit in `{k}`")));
                }
                let m = meet(v.iv, b);
                Val { iv: if m.0 <= m.1 { m } else { b }, nz: v.nz, eq: v.eq, below: v.below, ..Val::top() }
            }
            Rvalue::Convert(a, conv) => {
                reads(st, a, notes);
                let v = st.val(a);
                if has_elems(conv) {
                    notes.push((ALLOC, "an array is converted here, which makes a new one".into()));
                }
                match conv {
                    Conv::Fit(k) => {
                        let b = bounds(k);
                        if !inside(v.iv, b) {
                            notes.push((PANIC, format!("this number is not proved to fit in `{k}`")));
                        }
                        let m = meet(v.iv, b);
                        Val { iv: if m.0 <= m.1 { m } else { b }, nz: v.nz, eq: v.eq, below: v.below, ..Val::top() }
                    }
                    Conv::Elems(c) if matches!(**c, Conv::Fit(k) if inside(v.items, bounds(k))) => Val::top(),
                    c => {
                        if has_fit(c) {
                            notes.push((PANIC, "a number inside this value is not proved to fit in its type".into()));
                        }
                        Val::top()
                    }
                }
            }
            Rvalue::CastFloat(a, _) | Rvalue::Some(a) | Rvalue::Ok(a) | Rvalue::Err(a) | Rvalue::Is(a, _) => {
                reads(st, a, notes);
                Val::top()
            }
            Rvalue::Tuple(ops) => {
                ops.iter().for_each(|o| reads(st, o, notes));
                Val::top()
            }
            Rvalue::Struct(i, ops) => {
                ops.iter().for_each(|o| reads(st, o, notes));
                if self.p.boxed.0.get(*i).copied().unwrap_or(true) {
                    let what = if *i >= self.p.dyn_from { "a `dyn` value is created here" } else { "a value of a recursive struct is created here" };
                    notes.push((ALLOC, what.into()));
                }
                Val::top()
            }
            Rvalue::Enum(i, _, ops) => {
                ops.iter().for_each(|o| reads(st, o, notes));
                if !ops.is_empty() && self.p.boxed.1.get(*i).copied().unwrap_or(true) {
                    notes.push((ALLOC, "a value of a recursive enum is created here".into()));
                }
                Val::top()
            }
            Rvalue::Array(ops) => {
                ops.iter().for_each(|o| reads(st, o, notes));
                if !ops.is_empty() {
                    notes.push((ALLOC, "an array is created here".into()));
                }
                let items = ops.iter().map(|o| st.val(o).iv).reduce(|a, b| (a.0.min(b.0), a.1.max(b.1))).unwrap_or(TOP);
                Val { min_len: Some(ops.len() as i128), items, ..Val::top() }
            }
            Rvalue::Repeat(a, n) => {
                reads(st, a, notes);
                reads(st, n, notes);
                notes.push((ALLOC, "an array is created here".into()));
                let len = st.val(n).iv;
                if !inside(len, (0, 100_000_000)) {
                    notes.push((PANIC, "the length of this array is not proved to be one an array can have".into()));
                }
                Val { min_len: Some(len.0), ..Val::top() }
            }
            Rvalue::Len(a) => {
                reads(st, a, notes);
                let k = match a {
                    Operand::Place(p) => key(p),
                    _ => None,
                };
                let lo = k.as_ref().map_or(0, |k| st.least_len(k));
                Val { len_of: k, nz: lo > 0, ..Val::range((lo, LEN_MAX)) }
            }
            Rvalue::Slice(a, r) => {
                reads(st, a, notes);
                reads(st, r, notes);
                notes.push((ALLOC, "a part of an array is copied here".into()));
                notes.push((PANIC, "this range is not proved to be inside the array".into()));
                Val::top()
            }
            Rvalue::Format(parts) => {
                let mut built = false;
                for part in parts {
                    if let FmtPart::Val(o) = part {
                        reads(st, o, notes);
                        built = true;
                    }
                }
                if built {
                    notes.push((ALLOC, "a string is built here".into()));
                }
                Val::top()
            }
            Rvalue::Chars(a) => {
                reads(st, a, notes);
                notes.push((ALLOC, "an array of the characters of a string is created here".into()));
                Val::top()
            }
            Rvalue::RangeStart(a, _) => {
                reads(st, a, notes);
                notes.push((PANIC, "this range is not proved to have a start".into()));
                Val::top()
            }
            Rvalue::RangeHas(a, b) | Rvalue::RangeLast(a, b) | Rvalue::Get(a, b) => {
                reads(st, a, notes);
                reads(st, b, notes);
                Val::top()
            }
            Rvalue::Succ(a, num) => {
                reads(st, a, notes);
                match num {
                    Num::Int(k) => {
                        let (r, bad) = arith(BinOp::Add, k, st.val(a).iv, (1, 1), true);
                        if bad {
                            notes.push((PANIC, format!("the counter of this loop can overflow `{k}`")));
                        }
                        Val::range(r)
                    }
                    _ => {
                        notes.push((PANIC, "this loop can go past the last character".into()));
                        Val::top()
                    }
                }
            }
            Rvalue::Push(p, a) => {
                indexes(st, p, notes);
                reads(st, a, notes);
                notes.push((ALLOC, "`push` can make the array grow".into()));
                Val::top()
            }
            Rvalue::Pop(p) => {
                indexes(st, p, notes);
                Val::top()
            }
            Rvalue::Closure(_, ops) => {
                ops.iter().for_each(|o| reads(st, o, notes));
                if !ops.is_empty() {
                    notes.push((ALLOC, "a closure that captures values is created here".into()));
                }
                Val::top()
            }
            Rvalue::Const(_) | Rvalue::Capture(_) | Rvalue::Uninit | Rvalue::Moved => Val::top(),
        }
    }

    /// Runs a block from a state. Returns where it goes on, and in which state.
    fn step(&self, b: BlockId, mut st: State, mut found: Option<&mut Found>) -> Vec<(BlockId, State)> {
        let block = &self.f.blocks[b];
        let report = |found: &mut Option<&mut Found>, span: Span, notes: Notes| {
            if let Some(found) = found {
                found.sites.extend(notes.into_iter().map(|(effect, what)| Site { span, effect, what }));
            }
        };
        for s in &block.stmts {
            let mut notes = Vec::new();
            indexes(&st, &s.place, &mut notes);
            let v = self.eval(&st, &s.rv, &mut notes);
            if let Rvalue::Push(p, _) | Rvalue::Pop(p) = &s.rv {
                st.kill_keys(p);
                st.items[p.local] = TOP;
            }
            st.write(&s.place, v, self.hint(s.place.local));
            report(&mut found, s.span, notes);
        }
        let mut notes = Vec::new();
        let next = match &block.term {
            Term::Goto(t) => vec![(*t, st)],
            Term::Branch(c, t, e) => {
                reads(&st, c, &mut notes);
                let cmp = bare(c).and_then(|l| st.cmp[l].clone());
                match (c, cmp) {
                    (Operand::Const(Value::Bool(yes)), _) => vec![(if *yes { *t } else { *e }, st)],
                    (_, Some((op, x, y))) => {
                        let known = st.decide(op, &x, &y);
                        let mut out = Vec::new();
                        if known != Some(false) {
                            let mut s = st.clone();
                            s.refine(op, &x, &y);
                            out.push((*t, s));
                        }
                        if known != Some(true) {
                            if let Some(neg) = negated(op) {
                                st.refine(neg, &x, &y);
                            }
                            out.push((*e, st));
                        }
                        out.retain(|(_, s)| s.feasible());
                        out
                    }
                    _ => vec![(*t, st.clone()), (*e, st)],
                }
            }
            Term::Call { callee, args, writeback, dest, next } => {
                args.iter().for_each(|a| reads(&st, a, &mut notes));
                writeback.iter().for_each(|(_, p)| indexes(&st, p, &mut notes));
                indexes(&st, dest, &mut notes);
                let mut ret = TOP;
                let mut returns = true;
                match callee {
                    Callee::Fn(g) => {
                        let callee = &self.p.fns[*g];
                        if let Some(Scalar::Int(k)) = callee.ret {
                            ret = bounds(k);
                        }
                        if let Some(found) = &mut found {
                            found.calls.push((block.span, *g));
                            if self.wheres[*g] && !self.holds(callee, &st, args) {
                                let name = callee.name.trim_end_matches(" (place)");
                                notes.push((PANIC, format!("a `where` condition of `{name}` is not proved to hold here")));
                            }
                        }
                    }
                    Callee::Indirect(op, slot) => {
                        reads(&st, op, &mut notes);
                        // What is called is one of the functions the program
                        // uses as a value, that takes this many arguments.
                        if let Some(found) = &mut found {
                            found.values.push(block.span);
                            for (g, _) in self.taken.iter().filter(|(g, of)| of == slot && self.p.fns[*g].params == args.len()) {
                                found.calls.push((block.span, *g));
                            }
                        }
                    }
                    Callee::Extern(x) => {
                        let name = &self.p.externs[*x].name;
                        // What it does is up to who wrote the `unsafe` around it.
                        notes.push((UNSAFE, format!("the C function `{name}` is called here")));
                        // What C calls back counts as called here.
                        for a in args {
                            if let (Operand::Const(Value::Fn(f)), Some(found)) = (a, &mut found) {
                                found.calls.push((block.span, *f));
                            }
                        }
                    }
                    Callee::Print => notes.push((IO, "`io.print` writes to the output".into())),
                    Callee::Native(name) => {
                        if *name == "raw.to_char" {
                            let iv = args.first().map_or(TOP, |a| st.val(a).iv);
                            if !inside(iv, (0, 0xD7FF)) && !inside(iv, (0xE000, 0x10FFFF)) {
                                notes.push((PANIC, "this number is not proved to be a character".into()));
                            }
                        }
                        if *name == "task.spawn" {
                            notes.push((ALLOC, "`spawn` creates the task on the heap".into()));
                            // What the task does counts as done here: its
                            // closure is made in this block, right before.
                            let run = args.first().and_then(|a| match a {
                                Operand::Place(p) => block.stmts.iter().rev().find_map(|s| match &s.rv {
                                    Rvalue::Closure(f, _) if s.place == *p => Some(*f),
                                    _ => None,
                                }),
                                Operand::Const(_) => None,
                            });
                            match run {
                                Some(f) => {
                                    if let Some(found) = &mut found {
                                        found.calls.push((block.span, f));
                                    }
                                }
                                None => notes.push((ALLOC | PANIC | IO | UNSAFE | BLOCK, "a task is started here, and what it does is not known".into())),
                            }
                        }
                        match *name {
                            "task.wait" => notes.push((BLOCK, "`wait` waits for a task".into())),
                            "task.close" => notes.push((BLOCK, "a `scope` waits for its tasks where it ends".into())),
                            "io.read_line" => notes.push((BLOCK, "`io.read_line` waits for input".into())),
                            "chan.new" => {
                                notes.push((ALLOC, "a channel is created on the heap".into()));
                                let iv = args.first().map_or(TOP, |a| st.val(a).iv);
                                if !inside(iv, (1, 1 << 24)) {
                                    notes.push((PANIC, "this capacity is not proved to be between 1 and 16777216".into()));
                                }
                            }
                            "mutex.new" => notes.push((ALLOC, "a `Mutex` is created on the heap".into())),
                            "mutex.get" | "mutex.set" | "mutex.take" => {
                                notes.push((BLOCK, "this waits while another task updates the `Mutex`".into()));
                                notes.push((PANIC, "this panics inside an `update` of the same `Mutex`".into()));
                            }
                            "chan.send" => {
                                notes.push((BLOCK, "`send` waits while the channel is full".into()));
                                notes.push((PANIC, "`send` panics on a closed channel".into()));
                            }
                            "chan.recv" => {
                                notes.push((BLOCK, "`recv` waits while the channel is empty".into()));
                                notes.push((ALLOC, "`recv` creates its result on the heap".into()));
                            }
                            _ => {}
                        }
                        if name.starts_with("io.") {
                            notes.push((IO, format!("`{name}` reaches outside the program")));
                        }
                        if matches!(*name, "io.read_line" | "io.read_file" | "io.args" | "raw.concat" | "raw.from_chars" | "path.join") {
                            let shown = if *name == "path.join" { "a call that returns `ref mut`".to_string() } else { format!("`{name}`") };
                            notes.push((ALLOC, format!("{shown} creates its result on the heap")));
                        }
                    }
                    Callee::Panic => {
                        notes.push((PANIC, "the program can stop here: this `panic` or `assert` is not proved to be out of reach".into()));
                        returns = false;
                    }
                }
                if returns {
                    for (_, p) in writeback {
                        st.write(p, Val::top(), self.hint(p.local));
                    }
                    st.write(dest, Val::range(ret), self.hint(dest.local));
                    vec![(*next, st)]
                } else {
                    Vec::new()
                }
            }
            Term::Return(op) => {
                reads(&st, op, &mut notes);
                Vec::new()
            }
            Term::Panic(msg) => {
                if msg.starts_with(WHERE) {
                    if let Some(found) = &mut found {
                        found.unproved = true;
                    }
                } else if !msg.starts_with("no arm of this `match`") {
                    // The type checker proved that an arm of a `match` matches.
                    notes.push((PANIC, format!("the program stops here when {msg}")));
                }
                Vec::new()
            }
            Term::Unreachable => Vec::new(),
        };
        report(&mut found, block.span, notes);
        next
    }

    /// Analyzes the function from a state of its entry, up to a block.
    fn run(&self, entry: State, stop: Option<BlockId>) -> Found {
        let n = self.f.blocks.len();
        let mut at: Vec<Option<State>> = vec![None; n];
        let mut visits = vec![0u32; n];
        at[0] = Some(entry);
        let heads = loop_heads(self.f);
        let mut work = vec![0];
        while let Some(b) = work.pop() {
            if Some(b) == stop {
                continue;
            }
            let Some(st) = at[b].clone() else { continue };
            for (t, s) in self.step(b, st, None) {
                let merged = match &at[t] {
                    None => s,
                    Some(old) => {
                        let mut j = old.join(&s);
                        // Every loop goes through the block it jumps back to.
                        if heads[t] && visits[t] > 3 {
                            j.widen(old);
                        }
                        j
                    }
                };
                if at[t].as_ref() != Some(&merged) {
                    visits[t] += 1;
                    at[t] = Some(merged);
                    work.push(t);
                }
            }
        }
        let mut found = Found::default();
        for (b, st) in at.into_iter().enumerate() {
            if let (Some(st), true) = (st, Some(b) != stop) {
                self.step(b, st, Some(&mut found));
            }
        }
        found
    }

    /// Whether the `where` conditions of a function hold for the
    /// arguments of a call, given what is known where the call is.
    fn holds(&self, g: &Func, st: &State, args: &[Operand]) -> bool {
        if g.body == 0 {
            return false;
        }
        let mut entry = start(g);
        let vals: Vec<Val> = args.iter().map(|a| st.val(a)).collect();
        for (j, v) in vals.iter().enumerate().take(g.params) {
            let m = meet(entry.iv[j], v.iv);
            if m.0 <= m.1 {
                entry.iv[j] = m;
            }
            entry.nz[j] = v.nz;
            if let Some((r, _)) = vals[..j].iter().enumerate().find(|(_, o)| o.eq.is_some() && o.eq == v.eq) {
                entry.eq[j] = Some(entry.root(r));
            }
            let Some((src, fields)) = &v.from else { continue };
            let under = |k: &Key| (k.0 == *src && k.1.starts_with(fields)).then(|| k.1[fields.len()..].to_vec());
            for (k, n) in &st.min_len {
                if let Some(rest) = under(k) {
                    entry.set_min_len((j, rest), *n);
                }
            }
            for (i, o) in vals.iter().enumerate().take(g.params) {
                for k in &o.below {
                    if let Some(rest) = under(k) {
                        entry.add_below(i, (j, rest));
                    }
                }
            }
        }
        let walk = Walk { p: self.p, f: g, wheres: self.wheres, taken: self.taken };
        !walk.run(entry, Some(g.body)).unproved
    }
}

/// The blocks a loop jumps back to: the ones reached again while what
/// follows from them is still being walked.
fn loop_heads(f: &Func) -> Vec<bool> {
    let n = f.blocks.len();
    let next = |b: BlockId| -> Vec<BlockId> {
        match &f.blocks[b].term {
            Term::Goto(t) => vec![*t],
            Term::Branch(_, t, e) => vec![*t, *e],
            Term::Call { next, .. } => vec![*next],
            _ => Vec::new(),
        }
    };
    // 0 not seen, 1 being walked, 2 done.
    let mut state = vec![0u8; n];
    let mut heads = vec![false; n];
    let mut stack: Vec<(BlockId, Vec<BlockId>)> = Vec::new();
    if n > 0 {
        state[0] = 1;
        stack.push((0, next(0)));
    }
    while let Some((b, left)) = stack.last_mut() {
        match left.pop() {
            Some(t) if state[t] == 0 => {
                state[t] = 1;
                stack.push((t, next(t)));
            }
            Some(t) => {
                if state[t] == 1 {
                    heads[t] = true;
                }
            }
            None => {
                state[*b] = 2;
                stack.pop();
            }
        }
    }
    heads
}

/// What is known when a function starts: the ranges of its parameters.
fn start(f: &Func) -> State {
    let mut st = State::top(f.locals);
    for (j, s) in f.scalars.iter().enumerate() {
        if let Some(Scalar::Int(k)) = s {
            st.iv[j] = bounds(k);
        }
    }
    st
}

pub struct Effects {
    /// The effects of each function of the program.
    pub of: Vec<u8>,
    found: Vec<Found>,
}

pub fn infer(p: &Program) -> Effects {
    let wheres: Vec<bool> =
        p.fns.iter().map(|f| f.blocks.iter().any(|b| matches!(&b.term, Term::Panic(m) if m.starts_with(WHERE)))).collect();
    let taken = taken(p);
    let mut found: Vec<Found> = p
        .fns
        .iter()
        .map(|f| {
            let mut found = Walk { p, f, wheres: &wheres, taken: &taken }.run(start(f), None);
            let copy = "a value that holds an array is copied here";
            found.sites.extend(f.copies.iter().map(|span| Site { span: *span, effect: ALLOC, what: copy.into() }));
            found
        })
        .collect();
    // A function that can call itself can exhaust the stack.
    let n = p.fns.len();
    let callees: Vec<Vec<usize>> = found.iter().map(|f| f.calls.iter().map(|(_, g)| *g).collect()).collect();
    for f in 0..n {
        let mut seen = vec![false; n];
        let mut work = callees[f].clone();
        while let Some(g) = work.pop() {
            if !std::mem::replace(&mut seen[g], true) {
                work.extend(&callees[g]);
            }
        }
        if !seen[f] {
            continue;
        }
        // The calls of `f` that lead back to it.
        for (span, g) in found[f].calls.clone() {
            let mut back = vec![false; n];
            let mut work = vec![g];
            while let Some(h) = work.pop() {
                if !std::mem::replace(&mut back[h], true) {
                    work.extend(&callees[h]);
                }
            }
            if back[f] {
                let what = "this call is recursive, and the nesting of calls has a limit".to_string();
                found[f].sites.push(Site { span, effect: PANIC, what });
            }
        }
    }
    let mut of: Vec<u8> = found.iter().map(|f| f.sites.iter().fold(0, |e, s| e | s.effect)).collect();
    loop {
        let mut changed = false;
        for f in 0..n {
            let all = callees[f].iter().fold(of[f], |e, g| e | of[*g]);
            changed |= all != of[f];
            of[f] = all;
        }
        if !changed {
            break;
        }
    }
    Effects { of, found }
}

impl Effects {
    /// Why a function has an effect: the operation at the end of the chain.
    pub fn cause(&self, p: &Program, f: usize, effect: u8, seen: &mut Vec<usize>) -> String {
        if let Some(s) = self.found[f].sites.iter().find(|s| s.effect & effect != 0) {
            return format!("{} (line {} of its file)", s.what, s.span.local_line());
        }
        seen.push(f);
        match self.found[f].calls.iter().find(|(_, g)| self.of[*g] & effect != 0 && !seen.contains(g)) {
            Some((_, g)) => format!("it calls `{}`, where {}", shown(&p.fns[*g].name), self.cause(p, *g, effect, seen)),
            None => "one of the functions it calls does".to_string(),
        }
    }
}

/// The effects of every function of a program as text, in the form the
/// analysis written in Mou (`compiler/effects.mou`) prints them.
pub fn dump(p: &Program) -> String {
    use std::fmt::Write;
    let fx = infer(p);
    let mut out = String::new();
    for (i, f) in p.fns.iter().enumerate() {
        let _ = writeln!(out, "fn {i} {}: {}", f.name, names(fx.of[i]));
        for s in &fx.found[i].sites {
            let _ = writeln!(out, "  site {}:{} {} {}", s.span.line, s.span.col, s.effect, s.what);
        }
        for (span, g) in &fx.found[i].calls {
            let _ = writeln!(out, "  call {}:{} {g}", span.line, span.col);
        }
        if fx.found[i].unproved {
            let _ = writeln!(out, "  unproved");
        }
    }
    out
}

fn shown(name: &str) -> &str {
    name.trim_end_matches(" (place)")
}

pub fn names(effects: u8) -> String {
    let all = [(ALLOC, "alloc"), (PANIC, "panic"), (IO, "io"), (UNSAFE, "unsafe"), (BLOCK, "block")];
    let on: Vec<&str> = all.iter().filter(|(e, _)| effects & e != 0).map(|(_, n)| *n).collect();
    if on.is_empty() { "none".to_string() } else { on.join(" ") }
}

/// Checks the functions that rule effects out.
pub fn check(p: &Program) -> Vec<Error> {
    if p.fns.iter().all(|f| f.limits == 0) {
        return Vec::new();
    }
    let fx = infer(p);
    let mut errors: Vec<(Span, String, String)> = Vec::new();
    for (i, f) in p.fns.iter().enumerate() {
        if f.name.ends_with(" (place)") {
            continue;
        }
        let rules = [
            (ALLOC, if f.limits & SYSTEM != 0 { "in a `system` module, which has no allocator" } else { "`@noalloc`" }, "allocates"),
            (PANIC, "`@nopanic`", "can panic"),
            (IO, "`@noio`", "does input or output"),
            (BLOCK, "`@noblock`", "can wait"),
            (UNSAFE, "`@nounsafe`", "runs code the compiler does not check"),
        ];
        for (effect, rule, does) in rules {
            if f.limits & effect == 0 {
                continue;
            }
            let head = format!("`{}` is {rule}, but ", f.name);
            for s in fx.found[i].sites.iter().filter(|s| s.effect & effect != 0) {
                errors.push((s.span, head.clone(), s.what.clone()));
            }
            for (span, g) in &fx.found[i].calls {
                // One reason for each call is enough.
                let told = fx.found[i].sites.iter().any(|s| s.span == *span && s.effect & effect != 0);
                if fx.of[*g] & effect != 0 && !told {
                    let why = fx.cause(p, *g, effect, &mut vec![i]);
                    if !fx.found[i].values.contains(span) {
                        errors.push((*span, head.clone(), format!("it calls `{}`, which {does}: {why}", shown(&p.fns[*g].name))));
                    } else if !errors.iter().any(|e| e.0 == *span && e.1 == head && e.2.starts_with("the function value")) {
                        // Of all it can be, the first one is enough to say why.
                        let name = shown(&p.fns[*g].name);
                        errors.push((*span, head.clone(), format!("the function value or `dyn` method called here can be `{name}`, which {does}: {why}")));
                    }
                }
            }
        }
    }
    errors.sort_by(|a, b| (a.0.line, a.0.col, &a.2).cmp(&(b.0.line, b.0.col, &b.2)));
    // An instance of a generic function repeats the errors of another.
    errors.dedup_by(|a, b| a.0 == b.0 && a.2 == b.2);
    errors.into_iter().map(|(span, head, what)| Error { msg: format!("{head}{what}"), span }).collect()
}

#[cfg(test)]
mod tests {
    use crate::{check, mir, parser};

    const DECLS: &str = "import std.io
struct P { x: int, tags: [str] }
enum Tree { Leaf, Node(Tree?, int) }
fn first(xs: [int]) -> int where xs.len > 0 { xs[0] }
fn item(xs: [int], i: usize) -> int where i < xs.len { xs[i] }
fn twice(n: int) -> int { n * 2 }
fn say() ! io.Error { io.print(\"x\")? }
";

    fn errors(rule: &str, body: &str) -> Vec<String> {
        let src = format!("{DECLS}{rule}\nfn f(xs: [int], i: usize, n: int, p: P, g: fn(int) -> int) -> int {{\n{body}\n}}");
        let file = parser::parse_file(&src).unwrap_or_else(|e| panic!("{body}: {}", e.msg));
        let (_, info) = check::analyze(&file).unwrap_or_else(|es| panic!("{body}: {:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let program = mir::lower(&file, &info).unwrap_or_else(|es| panic!("{body}: {:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        super::check(&program).into_iter().map(|e| e.msg).collect()
    }

    fn ok(rule: &str, body: &str) {
        assert_eq!(errors(rule, body), Vec::<String>::new(), "{body}");
    }

    fn bad(rule: &str, body: &str, needle: &str) {
        let es = errors(rule, body);
        assert!(es.iter().any(|e| e.contains(needle)), "{body}: expected `{needle}` in {es:?}");
    }

    #[test]
    fn panics() {
        let r = "@nopanic";
        // Arithmetic.
        ok(r, "n +% 1");
        ok(r, "n *| 2");
        ok(r, "n / 2");
        ok(r, "if n > 0 && n < 1000 { n * n + 1 } else { 0 }");
        ok(r, "if n != 0 { 100 / n } else { 0 }");
        ok(r, "if n > -100 { 0 - n } else { 0 }");
        ok(r, "if n >= 0 && n < 256 { (n as u8) as int } else { 0 }");
        bad(r, "n + 1", "`+` on `i64` can overflow here");
        bad(r, "0 - n", "`-` on `i64` can overflow here");
        bad(r, "100 / n", "can divide by zero");
        bad(r, "100 % n", "can divide by zero");
        bad(r, "(n as u8) as int", "is not proved to fit in `u8`");
        bad(r, "1 << n", "shift");
        // Indexes.
        ok(r, "if i < xs.len { xs[i] } else { 0 }");
        ok(r, "if xs.len > 2 { xs[2] } else { 0 }");
        ok(r, "if xs.len == 0 { return 0 }\n xs[xs.len - 1]");
        ok(r, "if xs.len == 0 { return 0 }\n xs[i % xs.len]");
        ok(r, "var s = 0\n for x in xs { s = s +% x }\n s");
        ok(r, "var s = 0\n for k in 0..xs.len { s = s +% xs[k] }\n s");
        ok(r, "var s = 0\n var k: usize = 0\n while k < xs.len {\n s = s +% xs[k]\n k += 1\n }\n s");
        ok(r, "let len = xs.len\n var s = 0\n for k in 0..len { s = s +% xs[k] }\n s");
        ok(r, "let ys = [1, 2, 3]\n ys[2]");
        ok(r, "let ys: [u8] = [1, 2, 255]\n ys.len as int");
        ok(r, "match xs.get(i) {\n Some(v) => v\n None => 0\n }");
        ok(r, "if i < p.tags.len && p.tags[i] == \"a\" { 1 } else { 0 }");
        bad(r, "xs[i]", "this index is not proved to be in range");
        bad(r, "xs[0]", "this index is not proved to be in range");
        bad(r, "if i <= xs.len { xs[i] } else { 0 }", "this index is not proved to be in range");
        bad(r, "if n >= 0 { xs[n as usize] } else { 0 }", "this index is not proved to be in range");
        // What is known about an array ends when it can change.
        bad(r, "var ys = xs\n if i < ys.len {\n let _ = ys.pop()\n return ys[i]\n }\n 0", "this index is not proved to be in range");
        ok(r, "var ys = xs\n if i < ys.len {\n ys[i] = 7\n return ys[i]\n }\n 0");
        // Stops.
        ok(r, "if n > 0 { assert(n > 0) }\n n");
        bad(r, "assert(n > 0)\n n", "is not proved to be out of reach");
        bad(r, "if n < 0 { panic(\"no\") }\n n", "is not proved to be out of reach");
        // Calls.
        ok(r, "if xs.len > 0 { first(xs) } else { 0 }");
        ok(r, "if i < xs.len { item(xs, i) +% first(xs) } else { 0 }");
        bad(r, "first(xs)", "a `where` condition of `first` is not proved to hold here");
        bad(r, "item(xs, 0)", "a `where` condition of `item` is not proved to hold here");
        bad(r, "twice(n)", "it calls `twice`, which can panic: `*` on `i64` can overflow here");
        // A function value can only be one the program uses as a value.
        ok(r, "g(n)");
        ok(r, "let h = fn(k: int) -> int { k +% 1 }\n g(n)");
        bad(r, "let h = twice\n g(n)", "the function value or `dyn` method called here can be `twice`, which can panic");
        bad(r, "let h = fn(k: int) -> int { k + 1 }\n g(n)", "called here can be `closure`, which can panic");
        // One that takes another number of arguments is not it.
        ok(r, "let h = fn(a: int, b: int) -> int { a + b }\n g(n)");
        // A call inside a loop does not lose what is known of the loop.
        ok(r, "var t = 0\n for x in xs { t = t +% g(x) }\n for k in 0..xs.len { t = t +% g(xs[k]) }\n t");
        bad(r, "if n <= 0 { 0 } else { f(xs, i, n - 1, p, g) }", "this call is recursive");
    }

    #[test]
    fn allocations() {
        let r = "@noalloc";
        ok(r, "n + xs.len as int + p.x");
        ok(r, "let q = P(1, [])\n q.x");
        ok(r, "let s = \"abc\"\n let t = s\n n");
        ok(r, "let t = Tree.Leaf\n n");
        bad(r, "let ys = [1, 2]\n n", "an array is created here");
        bad(r, "let ys = [0; 8]\n n", "an array is created here");
        bad(r, "var ys = xs\n n", "a value that holds an array is copied here");
        bad(r, "let q = p\n n", "a value that holds an array is copied here");
        bad(r, "var ys: [int] = []\n ys.push(1)\n n", "`push` can make the array grow");
        bad(r, "let s = \"n is {n}\"\n n", "a string is built here");
        bad(r, "let ys = xs[0..1]\n n", "a part of an array is copied here");
        bad(r, "let t = Tree.Node(None, 1)\n n", "a value of a recursive enum is created here");
        bad(r, "let h = fn(k: int) -> int { k + n }\n n", "a closure that captures values is created here");
        ok(r, "g(n)");
        bad(r, "let h = fn(k: int) -> int {\n let ys = [k]\n k\n }\n g(n)", "called here can be `closure`, which allocates");
    }

    #[test]
    fn input_and_output() {
        let r = "@noio";
        ok(r, "twice(n) + xs[i]");
        bad(r, "say() catch {}\n n", "it calls `say`, which does input or output: `io.print` writes to the output");
        bad(r, "io.print(\"x\") catch {}\n n", "`io.print` writes to the output");
    }

    #[test]
    fn rules() {
        ok("@nopanic @noalloc @noio", "if i < xs.len { xs[i] } else { n }");
        // Without a rule nothing is an error.
        ok("", "xs[i] + n + twice(n)");
    }

    #[test]
    fn system_modules_do_not_allocate() {
        let check = |src: &str| {
            let file = parser::parse_file(src).unwrap();
            let (_, info) = check::analyze(&file).unwrap();
            let program = mir::lower(&file, &info).unwrap();
            super::check(&program).into_iter().map(|e| e.msg).collect::<Vec<_>>()
        };
        assert_eq!(check("@profile(system)\nfn f(xs: mut [int]) { for i in 0..xs.len { xs[i] = 0 } }"), Vec::<String>::new());
        let es = check("@profile(system)\nfn f(xs: mut [int]) { xs.push(0) }");
        assert!(es[0].contains("`f` is in a `system` module, which has no allocator, but `push` can make the array grow"), "{es:?}");
        assert_eq!(check("@profile(app)\nfn f(xs: mut [int]) { xs.push(0) }"), Vec::<String>::new());
    }
}
