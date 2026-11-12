mod ast;
mod cgen;
mod check;
mod comptime;
mod effects;
mod fmt;
mod fuzz;
mod interp;
mod lexer;
mod load;
mod mir;
mod own;
mod parser;
mod repl;

use std::process::ExitCode;

const USAGE: &str = "usage: mou <command> <file.mou> [-o <output>] [-l <library>]...
       mou repl
       mou fuzz [<count>] [<seed>] [--sanitize]

commands:
  run      type-check the file and run its `main`; more arguments go to the program
  test     run the functions of the file whose name starts with `test_`
  repl     an interactive session: no file
  fuzz     write random programs and compare the interpreter with the native program
  fmt      give the file the standard layout; with --check, only say whether it has it
  build    compile the file to a native program, through C
  c        print the C source `build` compiles
  check    type-check the file
  effects  print the effects of each function of the file: alloc, panic, io
  mir      print the control-flow graph the interpreter runs
  parse    check that the file parses and list its declarations
  ast      print the syntax tree
  tokens   print the token stream
  lex      print the tokens in the form the lexer written in Mou prints them
  tree     print the syntax tree in the form the parser written in Mou prints it
  modules  print the program with all it imports, as the loader written in Mou does
  decls    print what the declarations say, as the checker written in Mou does
  types    print what the checker finds out about each expression, in the same form
  lowered  print the program lowered to MIR, as the lowering written in Mou does
  rawc     print the C written straight from the MIR, as the compiler written in Mou does";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|c| c == "repl") {
        repl::repl();
        return ExitCode::SUCCESS;
    }
    if args.get(1).is_some_and(|c| c == "typetest") {
        print!("{}", check::types_selftest());
        return ExitCode::SUCCESS;
    }
    if args.get(1).is_some_and(|c| c == "fuzz") {
        // `mou fuzz <count> [seed] [--sanitize]`
        let sanitize = args.iter().any(|a| a == "--sanitize");
        let numbers: Vec<u64> = args[2..].iter().filter_map(|a| a.parse().ok()).collect();
        let (count, seed) = (numbers.first().copied().unwrap_or(100), numbers.get(1).copied().unwrap_or(1));
        return if fuzz::fuzz(count, seed, sanitize) { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    let (Some(cmd), Some(path)) = (args.get(1), args.get(2)) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mou: cannot read {path}: {e}");
            return ExitCode::from(2);
        }
    };
    match cmd.as_str() {
        "check" => return check_file(path, &src),
        "run" | "mir" => {
            interp::ARGS.with(|a| *a.borrow_mut() = args[3..].to_vec());
            return run_file(path, &src, cmd == "mir");
        }
        "test" => return test_file(path, &src),
        "effects" => return effects_file(path, &src),
        "fmt" => {
            let check = args.get(3).is_some_and(|a| a == "--check");
            return match fmt::format(&src) {
                Ok(out) if out == src => ExitCode::SUCCESS,
                Ok(_) if check => {
                    eprintln!("{path} does not have the standard layout: run `mou fmt {path}`");
                    ExitCode::FAILURE
                }
                Ok(out) => match std::fs::write(path, out) {
                    Ok(()) => {
                        println!("formatted {path}");
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("mou: cannot write {path}: {e}");
                        ExitCode::FAILURE
                    }
                },
                Err(e) => {
                    eprintln!("mou: {path}: {e}");
                    ExitCode::FAILURE
                }
            };
        }
        "build" | "c" => {
            // `-o output`, and `-l name` for each C library to link.
            let mut output = None;
            let mut libs = Vec::new();
            let mut rest = args[3..].iter();
            while let Some(a) = rest.next() {
                match (a.as_str(), rest.next()) {
                    ("-o", Some(o)) => output = Some(o.clone()),
                    ("-l", Some(l)) => libs.push(format!("-l{l}")),
                    _ => {
                        eprintln!("{USAGE}");
                        return ExitCode::from(2);
                    }
                }
            }
            return build_file(path, &src, cmd == "c", output, &libs);
        }
        _ => {}
    }
    let result = match cmd.as_str() {
        "tokens" => lexer::lex(&src).map(|toks| {
            for t in toks {
                println!("{:>4}:{:<3} {:?}", t.span.line, t.span.col, t.tok);
            }
        }),
        "lex" => {
            print!("{}", lexer::dump(&src));
            Ok(())
        }
        "modules" => {
            print!("{}", load::dump(path, &src));
            Ok(())
        }
        "rawc" => {
            print!("{}", rawc_text(path, &src));
            Ok(())
        }
        "lowered" => {
            print!("{}", lowered_text(path, &src));
            Ok(())
        }
        "types" => {
            print!("{}", types_text(path, &src));
            Ok(())
        }
        "decls" => {
            print!("{}", decls_text(path, &src));
            Ok(())
        }
        "tree" => {
            print!("{}", ast::dump(&parser::parse_file(&src)));
            Ok(())
        }
        "ast" => parser::parse_file(&src).map(|file| println!("{file:#?}")),
        "parse" => parser::parse_file(&src).map(|file| {
            if let Some(p) = &file.profile {
                println!("profile {p}");
            }
            for i in &file.imports {
                println!("import  {}", i.path);
            }
            for d in &file.decls {
                let (kind, name) = d.kind.describe();
                println!("{kind:<7} {name}");
            }
            println!("ok: {} declarations", file.decls.len());
        }),
        _ => {
            eprintln!("mou: unknown command `{cmd}`\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            report(path, &src, &e);
            ExitCode::FAILURE
        }
    }
}

fn check_file(path: &str, src: &str) -> ExitCode {
    let (sources, loaded) = load::load(path, src);
    let sources = &sources;
    let file = match loaded {
        Ok(f) => f,
        Err(errors) => return fail(sources, &errors),
    };
    match check::analyze(&file) {
        Ok((r, info)) => {
            // Initialization and moves are checked on the MIR.
            let unlowered = match mir::lower(&file, &info) {
                Ok(mut program) => {
                    let mut errors = own::check(&program);
                    if errors.is_empty() {
                        errors = effects::check(&program);
                    }
                    if errors.is_empty() {
                        errors = comptime::eval(&mut program);
                    }
                    if !errors.is_empty() {
                        return fail(sources, &errors);
                    }
                    false
                }
                // Only what the interpreter cannot run yet is left unchecked.
                Err(errors) if errors.iter().all(|e| e.msg.starts_with("the interpreter does not support")) => true,
                Err(errors) => return fail(sources, &errors),
            };
            println!("ok: {} functions checked", r.checked_fns);
            if unlowered {
                println!("note: initialization, moves and effects were not checked: the file uses features that are not lowered to MIR yet");
            }
            if !r.skipped_decls.is_empty() || r.skipped_exprs > 0 {
                println!(
                    "note: {} declarations and {} expressions were not checked (they use features that are not implemented yet)",
                    r.skipped_decls.len(),
                    r.skipped_exprs
                );
                for d in &r.skipped_decls {
                    println!("  not checked: {d}");
                }
            }
            ExitCode::SUCCESS
        }
        Err(errors) => fail(sources, &errors),
    }
}

/// Loads a program, runs every check and lowers it to MIR.
fn compile(path: &str, src: &str) -> (load::Sources, Result<mir::Program, Vec<lexer::Error>>) {
    let (sources, loaded) = load::load(path, src);
    let program = loaded.and_then(|file| lower(&file));
    (sources, program)
}

fn lower(file: &ast::File) -> Result<mir::Program, Vec<lexer::Error>> {
    let (_, info) = check::analyze(file)?;
    let mut program = mir::lower(file, &info)?;
    let mut errors = own::check(&program);
    if errors.is_empty() {
        errors = effects::check(&program);
    }
    if errors.is_empty() {
        errors = comptime::eval(&mut program);
    }
    if errors.is_empty() {
        Ok(program)
    } else {
        Err(errors)
    }
}

/// The C of a program written straight from its MIR: nothing is checked
/// after the types and no constant is computed while compiling. It is what
/// the compiler written in Mou gives so far.
fn rawc_text(path: &str, src: &str) -> String {
    let errors = |es: Vec<lexer::Error>| es.iter().map(|e| format!("error {}:{} {}\n", e.span.line, e.span.col, e.msg)).collect();
    let (sources, file) = load::load(path, src);
    let file = match file {
        Ok(file) => file,
        Err(es) => return errors(es),
    };
    let info = match check::analyze(&file) {
        Ok((_, info)) => info,
        Err(es) => return errors(es),
    };
    let program = match mir::lower(&file, &info) {
        Ok(program) => program,
        Err(es) => return errors(es),
    };
    match cgen::emit(&program, &sources.paths()) {
        Ok(c) => c,
        Err(e) => format!("error 0:0 {e}\n"),
    }
}

/// A program lowered to MIR, before any later stage looks at it, or why
/// it could not be lowered.
fn lowered_text(path: &str, src: &str) -> String {
    let errors = |es: Vec<lexer::Error>| es.iter().map(|e| format!("error {}:{} {}\n", e.span.line, e.span.col, e.msg)).collect();
    let file = match load::load(path, src).1 {
        Ok(file) => file,
        Err(es) => return errors(es),
    };
    let info = match check::analyze(&file) {
        Ok((_, info)) => info,
        Err(es) => return errors(es),
    };
    match mir::lower(&file, &info) {
        Ok(program) => mir::dump(&program),
        Err(es) => errors(es),
    }
}

/// What the checker finds out about each expression of a program, or why
/// the program could not be loaded.
fn types_text(path: &str, src: &str) -> String {
    match load::load(path, src).1 {
        Ok(file) => check::info_dump(&file),
        Err(es) => es.iter().map(|e| format!("error {}:{} {}\n", e.span.line, e.span.col, e.msg)).collect(),
    }
}

/// What the declarations of a program say, or why it could not be loaded.
fn decls_text(path: &str, src: &str) -> String {
    match load::load(path, src).1 {
        Ok(file) => check::decls_dump(&file),
        Err(es) => es.iter().map(|e| format!("error {}:{} {}\n", e.span.line, e.span.col, e.msg)).collect(),
    }
}

fn fail(sources: &load::Sources, errors: &[lexer::Error]) -> ExitCode {
    for e in errors {
        report_in(sources, e);
    }
    eprintln!("{} error(s)", errors.len());
    ExitCode::FAILURE
}

fn build_file(path: &str, src: &str, print: bool, output: Option<String>, libs: &[String]) -> ExitCode {
    let (sources, program) = compile(path, src);
    let program = match program {
        Ok(p) => p,
        Err(errors) => return fail(&sources, &errors),
    };
    let c = match cgen::emit(&program, &sources.paths()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("mou: {e}");
            return ExitCode::FAILURE;
        }
    };
    if print {
        print!("{c}");
        return ExitCode::SUCCESS;
    }
    let stem = std::path::Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("out");
    let output = output.unwrap_or_else(|| stem.to_string());
    let c_path = format!("{output}.c");
    if let Err(e) = std::fs::write(&c_path, c) {
        eprintln!("mou: cannot write {c_path}: {e}");
        return ExitCode::FAILURE;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    // `CFLAGS` replaces the default `-O2`: `-O1` builds a large program much faster.
    let flags = std::env::var("CFLAGS").unwrap_or_else(|_| "-O2".to_string());
    match std::process::Command::new(&cc).args(flags.split_whitespace()).args(["-o", &output, &c_path, "-lm"]).args(libs).status() {
        Ok(status) if status.success() => {
            println!("built {output} (C source in {c_path})");
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("mou: the C compiler `{cc}` failed on {c_path}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("mou: cannot run the C compiler `{cc}`: {e} (set CC to choose another)");
            ExitCode::FAILURE
        }
    }
}

fn run_file(path: &str, src: &str, dump: bool) -> ExitCode {
    let (sources, program) = compile(path, src);
    let program = match program {
        Ok(p) => p,
        Err(errors) => return fail(&sources, &errors),
    };
    if dump {
        for f in &program.fns {
            println!("fn {} (params: {}, locals: {})", f.name, f.params, f.locals);
            for (i, b) in f.blocks.iter().enumerate() {
                println!("  bb{i}:");
                for s in &b.stmts {
                    println!("    {:?} = {:?}", s.place, s.rv);
                }
                println!("    {:?}", b.term);
            }
        }
        return ExitCode::SUCCESS;
    }
    let Some(main) = program.main.filter(|m| program.fns[*m].params == 0) else {
        eprintln!("mou: {path} has no `main` function without parameters");
        return ExitCode::FAILURE;
    };
    let result = interp::run(&program, main, &mut std::io::stdout().lock());
    match result {
        Ok(mir::Value::Err(e)) => {
            eprintln!("{path}: `main` returned an error: {}", interp::show(&program, &e, false));
            ExitCode::FAILURE
        }
        // An integer returned by `main` is the exit status of the process.
        Ok(mir::Value::Int(status)) => ExitCode::from((status & 0xFF) as u8),
        Ok(mir::Value::Ok(v)) => match *v {
            mir::Value::Int(status) => ExitCode::from((status & 0xFF) as u8),
            _ => ExitCode::SUCCESS,
        },
        Ok(_) => ExitCode::SUCCESS,
        Err(p) => {
            report_in(&sources, &lexer::Error { msg: format!("panic: {}", p.msg), span: p.span });
            // Innermost frames first; a deep stack is cut in the middle.
            let (head, tail) = (8, 2);
            for (i, name) in p.stack.iter().enumerate() {
                if p.stack.len() <= head + tail || i < head || i >= p.stack.len() - tail {
                    eprintln!("  in {name}");
                } else if i == head {
                    eprintln!("  ... {} more calls ...", p.stack.len() - head - tail);
                }
            }
            ExitCode::from(101)
        }
    }
}

/// Prints what each function of the file can do besides computing.
fn effects_file(path: &str, src: &str) -> ExitCode {
    let (sources, program) = compile(path, src);
    let program = match program {
        Ok(p) => p,
        Err(errors) => return fail(&sources, &errors),
    };
    let fx = effects::infer(&program);
    for (i, f) in program.fns.iter().enumerate() {
        let own = f.span.line >> lexer::FILE_SHIFT == 0;
        if own && f.name != "closure" && !f.name.ends_with(" (place)") && !f.name.starts_with("constant ") {
            println!("{:<28} {}", f.name, effects::names(fx.of[i]));
        }
    }
    ExitCode::SUCCESS
}

/// Runs the tests of a file: its functions without parameters whose name
/// starts with `test_`. A test fails when it panics or returns an error.
fn test_file(path: &str, src: &str) -> ExitCode {
    let (sources, program) = compile(path, src);
    let program = match program {
        Ok(p) => p,
        Err(errors) => return fail(&sources, &errors),
    };
    if program.tests.is_empty() {
        eprintln!("mou: {path} has no tests: functions without parameters whose name starts with `test_`");
        return ExitCode::FAILURE;
    }
    let mut failed = 0;
    for &t in &program.tests {
        let name = &program.fns[t].name;
        // What a test prints is only shown when it fails.
        let mut out = Vec::new();
        let problem = match interp::run(&program, t, &mut out) {
            Ok(mir::Value::Err(e)) => Some(format!("returned an error: {}", interp::show(&program, &e, false))),
            Ok(_) => None,
            Err(p) => {
                let (source, line) = sources.locate(p.span);
                Some(format!("{}:{line}:{}: panic: {}", source.path, p.span.col, p.msg))
            }
        };
        match problem {
            None => println!("ok    {name}"),
            Some(why) => {
                failed += 1;
                println!("FAIL  {name}\n      {why}");
                for line in String::from_utf8_lossy(&out).lines() {
                    println!("      | {line}");
                }
            }
        }
    }
    let total = program.tests.len();
    println!("\n{} passed, {failed} failed", total - failed);
    if failed == 0 { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Reports an error of a loaded program, in the file it belongs to.
fn report_in(sources: &load::Sources, e: &lexer::Error) {
    let (source, line) = sources.locate(e.span);
    report(&source.path, &source.text, &lexer::Error { msg: e.msg.clone(), span: lexer::Span { line, col: e.span.col } });
}

fn report(path: &str, src: &str, e: &lexer::Error) {
    eprintln!("{path}:{}:{}: error: {}", e.span.line, e.span.col, e.msg);
    if let Some(line) = src.lines().nth(e.span.line.saturating_sub(1) as usize) {
        eprintln!("  {line}");
        eprintln!("  {}^", " ".repeat(e.span.col.saturating_sub(1) as usize));
    }
}

#[cfg(test)]
mod selfhost {
    use crate::{cgen, check, comptime, interp, lexer, load, mir, own};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// The compiler written in Mou gives, stage by stage and for every file
    /// of the project, what this compiler gives: tokens, tree, modules,
    /// declarations, types, MIR and C. `tests/decls`, `tests/types` and
    /// `tests/lower` hold programs with mistakes, which both must report in
    /// the same words. At the end it compiles itself, and the compiler that
    /// comes out writes the same C again.
    ///
    /// With a C compiler at hand the stages run as a native program; without
    /// one they run in the interpreter, and the C is not compared.
    #[test]
    fn the_compiler_in_mou_agrees_with_this_one() {
        let root = env!("CARGO_MANIFEST_DIR");
        let main = format!("{root}/compiler/main.mou");
        let src = std::fs::read_to_string(&main).unwrap();
        let (sources, file) = load::load(&main, &src);
        let file = file.unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let (_, info) = check::analyze(&file).unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        let mut program = mir::lower(&file, &info).unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        assert!(own::check(&program).is_empty());
        assert!(comptime::eval(&mut program).is_empty());
        let mut files = Vec::new();
        for dir in ["compiler", "examples", "std", "tests/native", "tests/decls", "tests/types", "tests/lower"] {
            for entry in std::fs::read_dir(format!("{root}/{dir}")).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|e| e == "mou") {
                    files.push(path);
                }
            }
        }
        assert!(files.len() > 50);
        // The functions on types, over the same list of types.
        interp::ARGS.with(|a| *a.borrow_mut() = vec!["typetest".to_string()]);
        let mut out = Vec::new();
        interp::run(&program, program.main.unwrap(), &mut out).unwrap_or_else(|p| panic!("typetest: {}", p.msg));
        assert_eq!(String::from_utf8_lossy(&out), check::types_selftest());
        // The compiler written in Mou as a native program, when there is a
        // C compiler to make it with.
        let dir = std::env::temp_dir().join(format!("mou-selfhost-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let build = |c: &str, name: &str| -> Option<PathBuf> {
            let (source, exe) = (dir.join(format!("{name}.c")), dir.join(name));
            std::fs::write(&source, c).unwrap();
            let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
            let built = Command::new(cc).args(["-O0", "-w", "-o"]).arg(&exe).arg(&source).arg("-lm").output().ok()?;
            assert!(built.status.success(), "{}", String::from_utf8_lossy(&built.stderr));
            Some(exe)
        };
        let (std_dir, runtime) = (format!("{root}/std"), format!("{root}/src/runtime.c"));
        let native = |exe: &Path, stage: &str, file: &str| {
            let run = Command::new(exe).args([stage, file, &std_dir, &runtime]).output().unwrap();
            String::from_utf8_lossy(&run.stdout).into_owned()
        };
        let mouc = build(&cgen::emit(&program, &sources.paths()).unwrap(), "mouc");
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            let shown = path.display().to_string();
            let mut stages = vec![
                ("lex", lexer::dump(&text)),
                ("tree", crate::ast::dump(&crate::parser::parse_file(&text))),
                ("modules", load::dump(&shown, &text)),
                ("decls", crate::decls_text(&shown, &text)),
            ];
            // `main.mou` holds every other file of the compiler: what comes
            // after the declarations is compared once, there.
            if !shown.contains("/compiler/") || shown.ends_with("/main.mou") {
                stages.push(("types", crate::types_text(&shown, &text)));
                stages.push(("lowered", crate::lowered_text(&shown, &text)));
                if mouc.is_some() {
                    stages.push(("c", crate::rawc_text(&shown, &text)));
                }
            }
            for (stage, expected) in stages {
                let got = match &mouc {
                    Some(exe) => native(exe, stage, &shown),
                    None => {
                        interp::ARGS.with(|a| *a.borrow_mut() = vec![stage.to_string(), shown.clone(), std_dir.clone()]);
                        let mut out = Vec::new();
                        interp::run(&program, program.main.unwrap(), &mut out).unwrap_or_else(|p| panic!("{}: {}", path.display(), p.msg));
                        String::from_utf8_lossy(&out).into_owned()
                    }
                };
                assert!(got == expected, "{stage} of {} differs", path.display());
            }
        }
        // The compiler compiles itself, and what comes out does it again.
        if let Some(exe) = &mouc {
            let first = native(exe, "c", &main);
            assert!(first == crate::rawc_text(&main, &src), "the C of the compiler differs");
            if let Some(second) = build(&first, "mouc2") {
                assert!(native(&second, "c", &main) == first, "the compiler compiled by itself does not write itself again");
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
