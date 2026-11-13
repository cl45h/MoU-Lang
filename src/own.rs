//! Initialization and move checking, as a forward data-flow analysis over
//! the MIR.
//!
//! For every variable of the program it tracks whether, at each point, it
//! may hold a value, may never have been given one, or may have been moved
//! away with `own`. Reading a variable that is not certainly holding a value
//! is an error, and so is assigning twice to an immutable one.
//!
//! The law of exclusivity for the arguments of a call is checked earlier, on
//! the syntax tree, by the type checker.

use std::collections::HashMap;

use crate::lexer::{Error, Span};
use crate::mir::*;

const INIT: u8 = 1;
const UNINIT: u8 = 2;
const MOVED: u8 = 4;

pub fn check(p: &Program) -> Vec<Error> {
    let mut errors: Vec<Error> = Vec::new();
    for f in &p.fns {
        function(f, &mut errors);
    }
    errors.sort_by_key(|e| (e.span.line, e.span.col));
    errors.dedup_by(|a, b| a.span == b.span && a.msg == b.msg);
    errors
}

/// Collects the locals an operand reads.
fn operand(op: &Operand, out: &mut Vec<Local>) {
    if let Operand::Place(p) = op {
        place(p, out);
    }
}

fn place(p: &Place, out: &mut Vec<Local>) {
    out.push(p.local);
    for pr in &p.proj {
        match pr {
            Proj::Index(i) => operand(i, out),
            Proj::Path(l) => out.push(*l),
            Proj::Field(_) => {}
        }
    }
}

fn reads(rv: &Rvalue, out: &mut Vec<Local>) {
    use Rvalue::*;
    match rv {
        Use(a) | Unary(_, _, a) | CastInt(a, _) | CastFloat(a, _) | Convert(a, _) | Some(a) | Ok(a) | Err(a) | Is(a, _)
        | Len(a) | Chars(a) | RangeStart(a, _) | Succ(a, _) => operand(a, out),
        Push(p, a) => {
            place(p, out);
            operand(a, out);
        }
        Pop(p) => place(p, out),
        Binary(_, _, a, b) | Repeat(a, b) | Slice(a, b) | RangeHas(a, b) | RangeLast(a, b) | Get(a, b) => {
            operand(a, out);
            operand(b, out);
        }
        Tuple(ops) | Struct(_, ops) | Enum(_, _, ops) | Array(ops) | Closure(_, ops) => ops.iter().for_each(|o| operand(o, out)),
        Format(parts) => parts.iter().for_each(|part| {
            if let FmtPart::Val(o) = part {
                operand(o, out)
            }
        }),
        Const(_) | Capture(_) | Uninit | Moved => {}
    }
}

/// What a write to `place` reads: everything when it is a partial write.
fn written(p: &Place, out: &mut Vec<Local>) {
    if !p.proj.is_empty() {
        place(p, out);
    }
}

fn function(f: &Func, errors: &mut Vec<Error>) {
    let named = |l: Local| f.names.get(l).and_then(|n| n.as_deref());
    // Nothing to track without declarations lacking a value or moves.
    let tracked = f.blocks.iter().any(|b| b.stmts.iter().any(|s| matches!(s.rv, Rvalue::Uninit | Rvalue::Moved)));
    if !tracked {
        return;
    }
    let mut moved_at: HashMap<Local, Span> = HashMap::new();
    for st in f.blocks.iter().flat_map(|b| &b.stmts) {
        if matches!(st.rv, Rvalue::Moved) {
            moved_at.entry(st.place.local).or_insert(st.span);
        }
    }
    let moved_at = moved_at;
    let mut entry: Vec<Option<Vec<u8>>> = vec![None; f.blocks.len()];
    entry[0] = Some(vec![INIT; f.locals]);
    let mut work = vec![0];
    // `report` is only set on the final pass, once the states are stable.
    let run = |b: usize, state: &mut Vec<u8>, mut report: Option<&mut Vec<Error>>| -> Vec<usize> {
        let block = &f.blocks[b];
        let check = |ls: &[Local], state: &[u8], span: Span, report: &mut Option<&mut Vec<Error>>| {
            let Some(errors) = report else { return };
            for &l in ls {
                let (Some(name), s) = (named(l), state[l]) else { continue };
                let msg = if s & MOVED != 0 {
                    let at = moved_at.get(&l).map_or(String::new(), |sp| format!(" on line {}", sp.local_line()));
                    format!("`{name}` is used after it was moved with `own`{at}")
                } else if s == UNINIT {
                    format!("`{name}` is used before it is given a value")
                } else if s & UNINIT != 0 {
                    format!("`{name}` may not have a value here: it is not assigned on every path")
                } else {
                    continue;
                };
                errors.push(Error { msg, span });
            }
        };
        for st in &block.stmts {
            let mut ls = Vec::new();
            reads(&st.rv, &mut ls);
            written(&st.place, &mut ls);
            check(&ls, state, st.span, &mut report);
            if st.place.proj.is_empty() {
                let l = st.place.local;
                match st.rv {
                    Rvalue::Uninit => state[l] = UNINIT,
                    Rvalue::Moved => state[l] = MOVED,
                    _ => {
                        if let (Some(errors), Some(name), true) = (&mut report, named(l), f.once.contains(&l)) {
                            if state[l] & INIT != 0 {
                                let msg = format!("`{name}` is immutable and may already have a value here");
                                errors.push(Error { msg, span: st.span });
                            }
                        }
                        state[l] = INIT;
                    }
                }
            }
        }
        let mut ls = Vec::new();
        let next = match &block.term {
            Term::Goto(t) => vec![*t],
            Term::Branch(c, a, b) => {
                operand(c, &mut ls);
                vec![*a, *b]
            }
            Term::Call { callee, args, writeback, dest, next } => {
                if let Callee::Indirect(op, _) = callee {
                    operand(op, &mut ls);
                }
                args.iter().for_each(|a| operand(a, &mut ls));
                writeback.iter().for_each(|(_, p)| written(p, &mut ls));
                written(dest, &mut ls);
                vec![*next]
            }
            Term::Return(op) => {
                operand(op, &mut ls);
                Vec::new()
            }
            Term::Panic(_) | Term::Unreachable => Vec::new(),
        };
        check(&ls, state, block.span, &mut report);
        if let Term::Call { writeback, dest, .. } = &block.term {
            for p in writeback.iter().map(|(_, p)| p).chain([dest]) {
                if p.proj.is_empty() {
                    state[p.local] = INIT;
                }
            }
        }
        next
    };
    while let Some(b) = work.pop() {
        let Some(mut state) = entry[b].clone() else { continue };
        for t in run(b, &mut state, None) {
            let merged = match &entry[t] {
                None => state.clone(),
                Some(old) => old.iter().zip(&state).map(|(x, y)| x | y).collect(),
            };
            if entry[t].as_ref() != Some(&merged) {
                entry[t] = Some(merged);
                work.push(t);
            }
        }
    }
    for b in 0..f.blocks.len() {
        if let Some(mut state) = entry[b].clone() {
            run(b, &mut state, Some(errors));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{check, mir, parser};

    fn errors(body: &str) -> Vec<String> {
        let decls = "struct B { d: [int] }\nfn take(b: own B) -> int { 0 }\nfn peek(b: B) -> int { 0 }\n";
        let src = format!("{decls}fn f(c: bool) -> int {{\n{body}\n}}");
        let file = parser::parse_file(&src).unwrap();
        let (_, info) = check::analyze(&file).unwrap_or_else(|es| panic!("{body}: {:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let program = mir::lower(&file, &info).unwrap_or_else(|es| panic!("{body}: {:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        super::check(&program).into_iter().map(|e| e.msg).collect()
    }

    fn ok(body: &str) {
        assert_eq!(errors(body), Vec::<String>::new(), "{body}");
    }

    fn bad(body: &str, needle: &str) {
        let es = errors(body);
        assert!(es.iter().any(|e| e.contains(needle)), "{body}: expected `{needle}` in {es:?}");
    }

    #[test]
    fn initialization() {
        ok("let x: int\n if c { x = 1 } else { x = 2 }\n x");
        ok("var x: int\n x = 1\n x = 2\n x");
        ok("let x: int\n if c { return 0 } else { x = 2 }\n x");
        bad("let x: int\n x", "`x` is used before it is given a value");
        bad("let x: int\n if c { x = 1 }\n x", "`x` may not have a value here");
        bad("var x: int\n while c { x = 1 }\n x", "`x` may not have a value here");
        bad("let x: int\n x = 1\n x = 2\n x", "`x` is immutable and may already have a value");
        bad("let x: int\n while c { x = 1 }\n 0", "`x` is immutable and may already have a value");
        bad("var b: B\n b.d = [1]\n 0", "`b` is used before it is given a value");
        bad("let x: int\n let g = fn() -> int { x }\n g()", "`x` is used before it is given a value");
    }

    #[test]
    fn moves() {
        ok("var b = B([1])\n take(own b)");
        ok("var b = B([1])\n let n = take(own b)\n b = B([2])\n n + peek(b)");
        ok("var n = 0\n for k in 0..3 {\n var b = B([k])\n n += take(own b)\n }\n n");
        ok("var b = B([1])\n if c { return take(own b) }\n peek(b)");
        bad("var b = B([1])\n take(own b)\n peek(b)", "`b` is used after it was moved with `own` on line 6");
        bad("var b = B([1])\n take(own b) + take(own b)", "`b` is used after it was moved");
        bad("var b = B([1])\n if c { take(own b) }\n peek(b)", "`b` is used after it was moved");
        bad("var b = B([1])\n var n = 0\n while n < 3 { n += take(own b) }\n n", "`b` is used after it was moved");
    }
}
