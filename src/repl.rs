//! Interactive session: `mou repl`.
//!
//! The session is a program that grows. A declaration is added to it; a
//! statement is added to its `main`. After each input the whole program is
//! checked and run again from the start, and only what it prints beyond
//! what it printed before is shown. That keeps the session exactly as
//! strict as a file, at the price of running earlier statements again.

use std::io::{BufRead, Write};

use crate::ast::{DeclKind, ExprKind, File, Stmt};
use crate::check::{self, Ty};
use crate::{interp, load, mir, own};

const HELP: &str = "\
Type a declaration (fn, struct, enum, trait, impl, import), a statement or
an expression. An expression shows its value. A line that leaves a bracket
open goes on in the next one.

  :help    this text
  :show    the program of the session so far
  :reset   start again
  :quit    leave (also the end of the input)

Each input runs the session again from the start: what earlier statements
print is hidden, but what they read or write happens again.";

#[derive(Default, Clone)]
struct Session {
    imports: Vec<String>,
    decls: Vec<String>,
    stmts: Vec<String>,
    /// How much the statements so far print.
    printed: usize,
}

impl Session {
    /// The program of the session, with more statements at the end of `main`.
    fn program(&self, extra: &[String]) -> String {
        let mut src = String::from("import std.io\n");
        for i in &self.imports {
            src.push_str(i);
            src.push('\n');
        }
        for d in &self.decls {
            src.push_str(d);
            src.push('\n');
        }
        src.push_str("fn main() ! io.Error {\n");
        for s in self.stmts.iter().chain(extra) {
            src.push_str(s);
            src.push('\n');
        }
        src.push_str("}\n");
        src
    }
}

/// What a program does: the first error, or what it printed and, when it
/// stopped with a panic or an error, why.
struct Outcome {
    out: Vec<u8>,
    stopped: Option<String>,
}

fn first(errors: Vec<crate::lexer::Error>) -> String {
    errors.into_iter().next().map_or("error".to_string(), |e| e.msg)
}

/// Checks a program. With `probe`, also says whether the value of the
/// last `let` of `main` is worth showing: it is not `()`.
fn compile(src: &str, probe: bool) -> Result<(mir::Program, bool), String> {
    let (_, file) = load::load("<repl>", src);
    let file = file.map_err(first)?;
    let (_, info) = check::analyze(&file).map_err(first)?;
    let shows = probe && last_let(&file).is_some_and(|e| !matches!(info.types.get(&(0, e)), Some(Ty::Unit | Ty::Never) | None));
    let mut program = mir::lower(&file, &info).map_err(first)?;
    let mut errors = own::check(&program);
    if errors.is_empty() {
        errors = crate::effects::check(&program);
    }
    if errors.is_empty() {
        errors = crate::comptime::eval(&mut program);
    }
    if !errors.is_empty() {
        return Err(first(errors));
    }
    Ok((program, shows))
}

/// The value of the last `let` of the program's `main`.
fn last_let(file: &File) -> Option<usize> {
    let main = file.decls.iter().rev().find_map(|d| match &d.kind {
        DeclKind::Fn(f) if f.name == "main" => Some(f),
        _ => None,
    })?;
    let ExprKind::Block(b) = &main.body.as_ref()?.kind else { return None };
    b.stmts.iter().rev().find_map(|s| match s {
        Stmt::Let { value: Some(v), .. } => Some(check::id(v)),
        _ => None,
    })
}

fn run(program: &mir::Program) -> Outcome {
    let mut out = Vec::new();
    let Some(main) = program.main else { return Outcome { out, stopped: None } };
    let stopped = match interp::run(program, main, &mut out) {
        Ok(mir::Value::Err(e)) => Some(format!("error: {}", interp::show(program, &e, false))),
        Ok(_) => None,
        Err(p) => Some(format!("panic: {}", p.msg)),
    };
    Outcome { out, stopped }
}

/// Whether the text leaves a bracket open, so that the input goes on.
fn open(text: &str) -> bool {
    let mut depth = 0i32;
    let mut cs = text.chars().peekable();
    while let Some(c) = cs.next() {
        match c {
            '"' => {
                // Skip the string; an interpolation may hold brackets.
                while let Some(d) = cs.next() {
                    match d {
                        '\\' => {
                            cs.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '\'' => {
                if cs.next() == Some('\\') {
                    cs.next();
                }
                cs.next();
            }
            '/' if cs.peek() == Some(&'/') => {
                for d in cs.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
            }
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth > 0
}

const DECLARES: &[&str] = &["fn", "struct", "enum", "trait", "impl", "type", "pub"];

/// Takes one input into the session. Returns what to show.
fn feed(s: &mut Session, input: &str) -> Result<String, String> {
    let word: String = input.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if word == "import" {
        if input == "import std.io" || s.imports.iter().any(|i| i == input) {
            return Ok(String::new());
        }
        let mut next = s.clone();
        next.imports.push(input.to_string());
        compile(&next.program(&[]), false)?;
        *s = next;
        return Ok(String::new());
    }
    if DECLARES.contains(&word.as_str()) {
        let mut next = s.clone();
        next.decls.push(input.to_string());
        compile(&next.program(&[]), false)?;
        *s = next;
        return Ok(String::new());
    }
    // An expression with a value is shown; anything else is a statement.
    let shown = [format!("let it_ = {input}"), "io.print(\"{it_}\")?".to_string()];
    let (program, keep) = match compile(&s.program(&shown), true) {
        Ok((program, true)) => (program, format!("let _ = {input}")),
        _ => (compile(&s.program(&[input.to_string()]), false)?.0, input.to_string()),
    };
    let done = run(&program);
    let new = String::from_utf8_lossy(done.out.get(s.printed..).unwrap_or_default()).into_owned();
    if let Some(why) = done.stopped {
        return Err(format!("{new}{why}"));
    }
    // What is kept does not print the value again.
    s.stmts.push(keep);
    s.printed = match compile(&s.program(&[]), false) {
        Ok((program, _)) => run(&program).out.len(),
        Err(_) => done.out.len(),
    };
    Ok(new)
}

pub fn repl() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut s = Session::default();
    println!("mou repl. :help for help, :quit to leave.");
    loop {
        let mut input = String::new();
        loop {
            print!("{}", if input.is_empty() { "mou> " } else { "...    " });
            let _ = std::io::stdout().flush();
            match lines.next() {
                Some(Ok(line)) => {
                    input.push_str(&line);
                    input.push('\n');
                }
                _ => {
                    println!();
                    return;
                }
            }
            if !open(&input) {
                break;
            }
        }
        let input = input.trim();
        match input {
            "" => continue,
            ":quit" | ":q" => return,
            ":help" => println!("{HELP}"),
            ":reset" => s = Session::default(),
            ":show" => print!("{}", s.program(&[])),
            _ if input.starts_with(':') => println!("unknown command `{input}`: see :help"),
            _ => match feed(&mut s, input) {
                Ok(out) => print!("{out}"),
                Err(e) => println!("{e}"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{feed, open, Session};

    #[test]
    fn a_session_grows() {
        let mut s = Session::default();
        let mut say = |input: &str| feed(&mut s, input);
        assert_eq!(say("1 + 2"), Ok("3\n".into()));
        assert_eq!(say("var xs: [int] = [1, 2]"), Ok(String::new()));
        assert_eq!(say("xs.push(3)"), Ok(String::new()));
        assert_eq!(say("xs"), Ok("[1, 2, 3]\n".into()));
        assert_eq!(say("xs.pop()"), Ok("Some(3)\n".into()));
        assert_eq!(say("xs.len"), Ok("2\n".into()));
        assert_eq!(say("fn double(n: int) -> int { n * 2 }"), Ok(String::new()));
        assert_eq!(say("struct P { x: int }"), Ok(String::new()));
        assert_eq!(say("io.print(\"{double(xs[1])} {P(4)}\")?"), Ok("4 P(x: 4)\n".into()));
        assert_eq!(say("import std.text"), Ok(String::new()));
        assert_eq!(say("text.words(\"a b\")"), Ok("[\"a\", \"b\"]\n".into()));
        assert_eq!(say("for x in xs { io.print(\"{x}\")? }"), Ok("1\n2\n".into()));
        // An input that fails leaves the session as it was.
        assert!(say("xs[9]").unwrap_err().contains("panic: index 9 is out of range"));
        assert!(say("nope + 1").unwrap_err().contains("nope"));
        assert!(say("fn bad() -> int { \"a\" }").is_err());
        assert_eq!(say("xs.len as int + double(1)"), Ok("4\n".into()));
    }

    #[test]
    fn open_brackets() {
        assert!(open("fn f() {"));
        assert!(!open("fn f() { }"));
        assert!(!open("let s = \"{ (\""));
        assert!(!open("let c = '{' // ("));
        assert!(open("f(1,\n [2"));
    }
}
