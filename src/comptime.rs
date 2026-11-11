//! Compile-time evaluation: the global constants and the `comptime`
//! expressions of a program are computed by the interpreter while the
//! program is compiled, and what is left in the program is their value.
//!
//! What runs here has no input or output, so that compiling a program
//! twice gives the same program, and it has a limit of steps. A panic
//! becomes an error of the compilation.

use crate::effects;
use crate::interp;
use crate::lexer::Error;
use crate::mir::*;

/// How many blocks all the constants of a program may run.
const LIMIT: u64 = 100_000_000;

fn shown(name: &str) -> String {
    match name.strip_prefix("constant ") {
        Some(n) => format!("the constant `{n}`"),
        None => "this `comptime` expression".to_string(),
    }
}

/// Whether a value is written where it is used, instead of read from the
/// table of constants.
fn small(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Float(_) | Value::F32(_) | Value::Bool(_) | Value::Char(_) | Value::Str(_) | Value::Unit | Value::None)
}

pub fn eval(p: &mut Program) -> Vec<Error> {
    if p.consts.is_empty() {
        return Vec::new();
    }
    let fx = effects::infer(p);
    let mut errors = Vec::new();
    for &f in &p.consts {
        if fx.of[f] & effects::IO != 0 {
            let why = fx.cause(p, f, effects::IO, &mut Vec::new());
            let msg = format!("{} is computed when the program is compiled, where there is no input or output: {why}", shown(&p.fns[f].name));
            errors.push(Error { msg, span: p.fns[f].span });
        }
    }
    if !errors.is_empty() {
        return errors;
    }
    let mut values: Vec<Value> = Vec::new();
    interp::FUEL.set(LIMIT);
    for &f in &p.consts {
        match interp::constant(p, &values, f) {
            Ok(v) => values.push(v),
            Err(panic) => {
                let msg = format!("{} cannot be computed: {}", shown(&p.fns[f].name), panic.msg);
                errors.push(Error { msg, span: panic.span });
                break;
            }
        }
    }
    interp::FUEL.set(u64::MAX);
    if !errors.is_empty() {
        return errors;
    }
    for st in p.fns.iter_mut().flat_map(|f| &mut f.blocks).flat_map(|b| &mut b.stmts) {
        if let Rvalue::Const(k) = st.rv {
            if small(&values[k]) {
                st.rv = Rvalue::Use(Operand::Const(values[k].clone()));
            }
        }
    }
    p.values = values.into_iter().map(Some).collect();
    errors
}

#[cfg(test)]
mod tests {
    use crate::mir::{Operand, Rvalue, Value};
    use crate::{check, mir, parser};

    fn compile(src: &str) -> Result<mir::Program, Vec<String>> {
        let file = parser::parse_file(src).unwrap_or_else(|e| panic!("{src}: {}", e.msg));
        let (_, info) = check::analyze(&file).unwrap_or_else(|es| panic!("{src}: {:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let mut program = mir::lower(&file, &info).map_err(|es| es.into_iter().map(|e| e.msg).collect::<Vec<_>>())?;
        let errors = super::eval(&mut program);
        if errors.is_empty() { Ok(program) } else { Err(errors.into_iter().map(|e| e.msg).collect()) }
    }

    fn bad(src: &str, needle: &str) {
        let es = compile(src).err().unwrap_or_default();
        assert!(es.iter().any(|e| e.contains(needle)), "{src}: expected `{needle}` in {es:?}");
    }

    #[test]
    fn constants_are_values() {
        let src = "fn fib(n: int) -> int { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }\n\
                   let F: int = fib(15)\nlet XS: [int] = [F, F + 1]\n\
                   fn main() -> int { F + XS[1] + comptime fib(10) }";
        let p = compile(src).unwrap();
        assert_eq!(p.values, vec![
            Some(Value::Int(610)),
            Some(Value::Array(std::rc::Rc::new(vec![Value::Int(610), Value::Int(611)]))),
            Some(Value::Int(55))
        ]);
        // What is left where a small constant is used is its value.
        let main = &p.fns[p.main.unwrap()];
        let uses: Vec<&Rvalue> = main.blocks.iter().flat_map(|b| &b.stmts).map(|s| &s.rv).collect();
        assert!(uses.contains(&&Rvalue::Use(Operand::Const(Value::Int(610)))), "{uses:?}");
        assert!(uses.contains(&&Rvalue::Use(Operand::Const(Value::Int(55)))), "{uses:?}");
        assert!(uses.contains(&&Rvalue::Const(1)), "{uses:?}");
    }

    #[test]
    fn what_cannot_be_computed_is_an_error() {
        bad("let A: int = 1 / (2 - 2)\nfn main() {}", "the constant `A` cannot be computed: division by zero");
        bad("fn f(xs: [int]) -> int { xs[3] }\nfn main() -> int { comptime f([1]) }", "this `comptime` expression cannot be computed: index 3 is out of range");
        bad("import std.io\nfn n() -> int {\n io.print(\"x\") catch {}\n 1\n}\nlet C: int = n()\nfn main() {}", "where there is no input or output");
        bad("fn main() -> int {\n let k = 3\n comptime k + 1\n}", "cannot use a variable of the function it is in");
        bad("fn spin() -> int {\n var n = 0\n while true { n = n +% 1 }\n n\n}\nfn main() -> int { comptime spin() }", "it runs for too long");
    }
}
