//! Differential fuzzing: `mou fuzz <count> [seed]`.
//!
//! Writes random programs, runs each one in the interpreter and as a
//! native program, and reports the ones where the two disagree: in what
//! they print or in how they end. A program the compiler rejects is
//! reported too, because the generator only writes valid programs.

use std::process::Command;

use crate::{cgen, check, comptime, effects, interp, load, mir, own};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len())]
    }
}

const KINDS: &[&str] = &["int", "i8", "u8", "i32", "u32", "i16", "u64"];

fn kind_name(k: &str) -> &str {
    if k == "int" { "i64" } else { k }
}

struct Gen {
    rng: Rng,
    out: String,
    /// Variables in scope: name, kind (`bool` and `f64` too), and whether
    /// it can be assigned.
    vars: Vec<(String, &'static str, bool)>,
    /// Arrays of `int` in scope, and whether they can change.
    arrays: Vec<(String, bool)>,
    /// How many functions exist: `f0` to this one, not included.
    fns: usize,
    names: usize,
    indent: usize,
}

impl Gen {
    fn line(&mut self, text: &str) {
        self.out.push_str(&"    ".repeat(self.indent));
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn fresh(&mut self, prefix: &str) -> String {
        self.names += 1;
        format!("{prefix}{}", self.names)
    }

    fn literal(&mut self, kind: &str) -> String {
        let (lo, hi) = mir::bounds(kind_name(kind));
        let n: i128 = match self.rng.below(16) {
            0 => lo,
            1 => hi,
            2 => 0,
            3 | 4 => 1,
            5 | 6 => (self.rng.next() % 1000) as i128,
            7 | 8 => -((self.rng.next() % 200) as i128),
            _ => 1 + (self.rng.next() % 9) as i128,
        };
        let n = n.clamp(lo, hi);
        // Arithmetic between untyped numbers is made as `int`: one that
        // does not fit there says its type.
        if n > i64::MAX as i128 {
            format!("({n} as {kind})")
        } else if n < 0 {
            format!("({n})")
        } else {
            n.to_string()
        }
    }

    /// An integer expression of a kind.
    fn int(&mut self, kind: &'static str, depth: usize) -> String {
        let leaf = depth == 0 || self.rng.chance(25);
        if leaf {
            let fits: Vec<String> = self
                .vars
                .iter()
                .filter(|(_, k, _)| *k == kind)
                .map(|(n, _, _)| n.clone())
                .collect();
            if !fits.is_empty() && self.rng.chance(70) {
                return fits[self.rng.below(fits.len())].clone();
            }
            return self.literal(kind);
        }
        let d = depth - 1;
        match self.rng.below(12) {
            0..=4 => {
                // Mostly the ones that cannot stop the program, so that it goes on.
                let ops: &[&str] = if self.rng.chance(6) { &["+", "-", "*", "/", "%"] } else { &["+%", "-%", "*%", "+|", "-|", "*|", "&", "|", "^"] };
                let op = self.rng.pick(ops);
                format!("({} {op} {})", self.int(kind, d), self.int(kind, d))
            }
            5 => {
                let op = self.rng.pick(&["<<", ">>"]);
                let by = if self.rng.chance(5) { 8 + self.rng.below(60) } else { self.rng.below(8) };
                format!("({} {op} {by})", self.int(kind, d))
            }
            6 => {
                // A cast stops the program when the value does not fit:
                // mostly from a kind that always does.
                let from = KINDS[self.rng.below(KINDS.len())];
                let from = if check::widens(kind_name(from), kind_name(kind)) || self.rng.chance(15) { from } else { kind };
                format!("({} as {kind})", self.int(from, d))
            }
            7 if !kind.starts_with('u') && self.rng.chance(30) => format!("(-{})", self.int(kind, d)),
            7 => format!("(0 -% {})", self.int(kind, d)),
            8 if kind == "int" && self.fns > 0 => {
                let f = self.rng.below(self.fns);
                let xs = self.array_arg();
                format!("f{f}({}, {}, {xs})", self.int("int", d), self.int("u8", d))
            }
            9 if kind == "int" && !self.arrays.is_empty() => {
                let (xs, _) = self.arrays[self.rng.below(self.arrays.len())].clone();
                match self.rng.below(3) {
                    0 => format!("{xs}[({} as usize) % {xs}.len]", self.int("u8", d)),
                    1 if self.rng.chance(15) => format!("{xs}[{}]", self.rng.below(4)),
                    _ => format!("({xs}.len as int)"),
                }
            }
            10 => format!("if {} {{ {} }} else {{ {} }}", self.boolean(d), self.int(kind, d), self.int(kind, d)),
            _ => self.literal(kind),
        }
    }

    fn array_arg(&mut self) -> String {
        if !self.arrays.is_empty() && self.rng.chance(60) {
            return self.arrays[self.rng.below(self.arrays.len())].0.clone();
        }
        let n = self.rng.below(4);
        let items: Vec<String> = (0..n).map(|_| self.literal("int")).collect();
        format!("[{}]", items.join(", "))
    }

    fn boolean(&mut self, depth: usize) -> String {
        let bools: Vec<String> = self.vars.iter().filter(|(_, k, _)| *k == "bool").map(|(n, _, _)| n.clone()).collect();
        if depth == 0 || self.rng.chance(20) {
            if !bools.is_empty() && self.rng.chance(60) {
                return bools[self.rng.below(bools.len())].clone();
            }
            return self.rng.pick(&["true", "false"]).to_string();
        }
        let d = depth - 1;
        match self.rng.below(6) {
            0..=2 => {
                let kind = KINDS[self.rng.below(KINDS.len())];
                let op = self.rng.pick(&["<", "<=", ">", ">=", "==", "!="]);
                format!("({} {op} {})", self.int(kind, d), self.int(kind, d))
            }
            3 => format!("({} && {})", self.boolean(d), self.boolean(d)),
            4 => format!("({} || {})", self.boolean(d), self.boolean(d)),
            _ => format!("!{}", self.boolean(d)),
        }
    }

    fn float(&mut self, depth: usize) -> String {
        let floats: Vec<String> = self.vars.iter().filter(|(_, k, _)| *k == "f64").map(|(n, _, _)| n.clone()).collect();
        if depth == 0 || self.rng.chance(30) {
            if !floats.is_empty() && self.rng.chance(60) {
                return floats[self.rng.below(floats.len())].clone();
            }
            return self.rng.pick(&["0.5", "1.0", "2.25", "100.0", "0.001", "3.0"]).to_string();
        }
        let d = depth - 1;
        match self.rng.below(4) {
            0..=2 => format!("({} {} {})", self.float(d), self.rng.pick(&["+", "-", "*", "/"]), self.float(d)),
            _ => format!("({} as f64)", self.int("i32", d)),
        }
    }

    fn block(&mut self, depth: usize, count: usize) {
        let (vars, arrays) = (self.vars.len(), self.arrays.len());
        self.indent += 1;
        for _ in 0..count {
            self.stmt(depth);
        }
        self.indent -= 1;
        self.vars.truncate(vars);
        self.arrays.truncate(arrays);
    }

    fn stmt(&mut self, depth: usize) {
        match self.rng.below(15) {
            0..=2 => {
                let kind = KINDS[self.rng.below(KINDS.len())];
                let name = self.fresh("v");
                // Sometimes the value is of a kind that converts to this one.
                let narrow: Vec<&'static str> = KINDS.iter().copied().filter(|k| check::widens(kind_name(k), kind_name(kind))).collect();
                let from = if self.rng.chance(25) { narrow[self.rng.below(narrow.len())] } else { kind };
                let value = self.int(from, 3);
                let mutable = self.rng.chance(60);
                self.line(&format!("{} {name}: {kind} = {value}", if mutable { "var" } else { "let" }));
                self.vars.push((name, kind, mutable));
            }
            3 => {
                let name = self.fresh("b");
                let value = self.boolean(3);
                self.line(&format!("var {name} = {value}"));
                self.vars.push((name, "bool", true));
            }
            4 => {
                let name = self.fresh("x");
                let value = self.float(3);
                self.line(&format!("var {name}: f64 = {value}"));
                self.vars.push((name, "f64", true));
            }
            5..=6 => {
                let targets: Vec<(String, &'static str)> = self.vars.iter().filter(|v| v.2).map(|v| (v.0.clone(), v.1)).collect();
                if targets.is_empty() {
                    return;
                }
                let (name, kind) = targets[self.rng.below(targets.len())].clone();
                match kind {
                    "bool" => {
                        let value = self.boolean(3);
                        self.line(&format!("{name} = {value}"));
                    }
                    "f64" => {
                        let value = self.float(3);
                        let op = self.rng.pick(&["=", "+=", "*="]);
                        self.line(&format!("{name} {op} {value}"));
                    }
                    _ => {
                        let value = self.int(kind, 3);
                        let op = self.rng.pick(&["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^="]);
                        self.line(&format!("{name} {op} {value}"));
                    }
                }
            }
            7 => {
                let name = self.fresh("xs");
                let items = self.array_arg();
                let items = if items.starts_with('[') { items } else { format!("[{}, 1]", self.int("int", 2)) };
                self.line(&format!("var {name}: [int] = {items}"));
                self.arrays.push((name, true));
            }
            8 => {
                let targets: Vec<String> = self.arrays.iter().filter(|a| a.1).map(|a| a.0.clone()).collect();
                if targets.is_empty() {
                    return;
                }
                let xs = targets[self.rng.below(targets.len())].clone();
                let value = self.int("int", 3);
                match self.rng.below(4) {
                    0..=1 => self.line(&format!("{xs}.push({value})")),
                    2 => {
                        let i = self.int("u8", 2);
                        self.line(&format!("if {xs}.len > 0 {{ {xs}[({i} as usize) % {xs}.len] = {value} }}"));
                    }
                    _ => {
                        let i = self.rng.below(3);
                        self.line(&format!("if {xs}.len > {i} {{ {xs}[{i}] = {xs}[{i}] +% {value} }}"));
                    }
                }
            }
            9..=10 if depth > 0 => {
                let c = self.boolean(3);
                self.line(&format!("if {c} {{"));
                let n = 1 + self.rng.below(3);
                self.block(depth - 1, n);
                if self.rng.chance(50) {
                    self.line("} else {");
                    let n = 1 + self.rng.below(3);
                    self.block(depth - 1, n);
                }
                self.line("}");
            }
            11 if depth > 0 => {
                let i = self.fresh("i");
                let n = 1 + self.rng.below(5);
                self.line(&format!("for {i} in 0..{n} {{"));
                self.vars.push((i, "int", false));
                let count = 1 + self.rng.below(3);
                self.block(depth - 1, count);
                self.vars.pop();
                self.line("}");
            }
            12 if depth > 0 => {
                let k = self.fresh("k");
                let n = 1 + self.rng.below(4);
                self.line(&format!("var {k} = 0"));
                self.line(&format!("while {k} < {n} {{"));
                let count = 1 + self.rng.below(2);
                self.block(depth - 1, count);
                self.line(&format!("    {k} += 1"));
                self.line("}");
            }
            13 if !self.arrays.is_empty() => {
                let (xs, _) = self.arrays[self.rng.below(self.arrays.len())].clone();
                let targets: Vec<String> = self.vars.iter().filter(|v| v.2 && v.1 == "int").map(|v| v.0.clone()).collect();
                if let Some(t) = targets.first() {
                    let item = self.fresh("e");
                    self.line(&format!("for {item} in {xs} {{ {t} = {t} +% {item} }}"));
                }
            }
            _ => {
                let name = self.fresh("p");
                let (a, b, c) = (self.int("int", 2), self.int("u8", 2), self.boolean(2));
                self.line(&format!("var {name} = S({a}, {b}, {c})"));
                let value = self.int("int", 2);
                self.line(&format!("{name}.a = {name}.a +% {value} +% {name}.b"));
                self.line(&format!("if {name}.c {{ {name}.b = {name}.b +% 1 }}"));
                let a = self.fresh("v");
                self.line(&format!("let {a}: int = {name}.a +% {name}.b"));
                self.vars.push((a, "int", false));
            }
        }
    }

    fn function(&mut self) {
        let f = self.fns;
        self.vars = vec![("a".into(), "int", false), ("b".into(), "u8", false)];
        self.arrays = vec![("xs".into(), false)];
        self.line(&format!("fn f{f}(a: int, b: u8, xs: [int]) -> int {{"));
        let n = 3 + self.rng.below(8);
        self.indent += 1;
        for _ in 0..n {
            self.stmt(2);
        }
        let result = self.int("int", 3);
        self.line(&result);
        self.indent -= 1;
        self.line("}");
        self.line("");
        self.fns += 1;
    }

    fn program(seed: u64) -> String {
        let mut g = Gen { rng: Rng(seed | 1), out: String::new(), vars: Vec::new(), arrays: Vec::new(), fns: 0, names: 0, indent: 0 };
        for _ in 0..8 {
            g.rng.next();
        }
        g.line("import std.io");
        g.line("");
        g.line("struct S { a: int, b: u8, c: bool }");
        g.line("");
        let fns = 2 + g.rng.below(5);
        for _ in 0..fns {
            g.function();
        }
        g.vars.clear();
        g.arrays.clear();
        g.line("fn main() ! io.Error {");
        g.indent += 1;
        let calls = 3 + g.rng.below(5);
        for _ in 0..calls {
            let f = g.rng.below(g.fns);
            let (a, b, xs) = (g.literal("int"), g.literal("u8"), g.array_arg());
            g.line(&format!("io.print(\"{{f{f}({a}, {b}, {xs})}}\")?"));
        }
        g.indent -= 1;
        g.line("}");
        g.out
    }
}

/// How a run ended: what it printed and its exit status.
type Outcome = (String, i32);

fn interpret(p: &mir::Program) -> Option<Outcome> {
    let mut out = Vec::new();
    interp::FUEL.set(20_000_000);
    let r = interp::run(p, p.main?, &mut out);
    interp::FUEL.set(u64::MAX);
    let status = match r {
        Ok(mir::Value::Err(_)) => 1,
        Ok(_) => 0,
        // Too long for a test: not a result to compare.
        Err(e) if e.msg.contains("runs for too long") => return None,
        Err(_) => 101,
    };
    Some((String::from_utf8_lossy(&out).into_owned(), status))
}

pub fn fuzz(count: u64, seed: u64, sanitize: bool) -> bool {
    let dir = std::env::temp_dir().join(format!("mou-fuzz-{}", std::process::id()));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("mou: cannot create {}: {e}", dir.display());
        return false;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let (mut same, mut skipped, mut bad) = (0, 0, 0);
    let (mut panics, mut lines) = (0, 0);
    for n in 0..count {
        let s = seed.wrapping_add(n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let src = Gen::program(s);
        let keep = |why: &str, detail: &str| {
            let path = dir.join(format!("case-{}.mou", seed.wrapping_add(n)));
            let _ = std::fs::write(&path, &src);
            println!("seed {}: {why}\n  {}\n  program kept in {}", seed.wrapping_add(n), detail.replace('\n', "\n  "), path.display());
        };
        let compiled = load::load("fuzz.mou", &src).1.and_then(|file| {
            let (_, info) = check::analyze(&file)?;
            let mut p = mir::lower(&file, &info)?;
            let mut errors = own::check(&p);
            if errors.is_empty() {
                errors = effects::check(&p);
            }
            if errors.is_empty() {
                errors = comptime::eval(&mut p);
            }
            if errors.is_empty() { Ok(p) } else { Err(errors) }
        });
        let p = match compiled {
            Ok(p) => p,
            Err(errors) => {
                bad += 1;
                let e = &errors[0];
                keep("the compiler rejects a program that should be valid", &format!("line {}: {}", e.span.local_line(), e.msg));
                continue;
            }
        };
        let Some(expected) = interpret(&p) else {
            skipped += 1;
            continue;
        };
        let c = match cgen::emit(&p, &["fuzz.mou".to_string()]) {
            Ok(c) => c,
            Err(e) => {
                bad += 1;
                keep("the C backend cannot write the program", &e);
                continue;
            }
        };
        let (c_path, exe) = (dir.join("fuzz.c"), dir.join("fuzz"));
        if std::fs::write(&c_path, c).is_err() {
            eprintln!("mou: cannot write {}", c_path.display());
            return false;
        }
        let mut build = Command::new(&cc);
        build.arg("-O1").arg("-o").arg(&exe).arg(&c_path).arg("-lm");
        if sanitize {
            build.args(["-g", "-fsanitize=address,undefined", "-fno-sanitize-recover=undefined"]);
        }
        match build.output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                bad += 1;
                keep("the C compiler rejects the program", &String::from_utf8_lossy(&o.stderr).lines().take(4).collect::<Vec<_>>().join("\n"));
                continue;
            }
            Err(e) => {
                eprintln!("mou: cannot run the C compiler `{cc}`: {e}");
                return false;
            }
        }
        let run = match Command::new(&exe).env("ASAN_OPTIONS", "detect_stack_use_after_return=0").output() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("mou: cannot run {}: {e}", exe.display());
                return false;
            }
        };
        let got = (String::from_utf8_lossy(&run.stdout).into_owned(), run.status.code().unwrap_or(-1));
        let stderr = String::from_utf8_lossy(&run.stderr);
        if stderr.contains("Sanitizer") || stderr.contains("runtime error") {
            bad += 1;
            keep("the native program has undefined behavior", &stderr.lines().take(3).collect::<Vec<_>>().join("\n"));
        } else if got != expected {
            bad += 1;
            let detail = format!("interpreter: status {} and {:?}\nnative:      status {} and {:?}", expected.1, expected.0, got.1, got.0);
            keep("the interpreter and the native program disagree", &detail);
        } else {
            same += 1;
            panics += (expected.1 == 101) as u64;
            lines += expected.0.lines().count();
        }
    }
    println!("{count} programs: {same} agree, {bad} problems, {skipped} too long to compare");
    println!("of the ones that agree, {panics} end in a panic; they print {lines} results in all");
    if bad == 0 {
        let _ = std::fs::remove_dir_all(&dir);
    }
    bad == 0
}
