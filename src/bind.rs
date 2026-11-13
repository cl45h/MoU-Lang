//! `mou bind <header> [name]`: writes the `import c` block of a header.
//!
//! The header goes through the preprocessor of the C compiler, and what
//! comes out is read here: the functions it declares, and the names it
//! gives to types. A function whose types Mou can pass is written as a
//! declaration; the others are listed with the reason they are left out.

use std::collections::HashMap;
use std::io::Write;
use std::process::{Command, Stdio};

/// What a type of C is, as far as a declaration of Mou can say it.
#[derive(Clone, Debug, PartialEq)]
enum CType {
    Void,
    /// A number or a `bool`, by its name in Mou.
    Scalar(&'static str),
    /// A pointer: what it points to, and whether that is `const`.
    Pointer(Box<CType>, bool),
    /// `char`, which is a number by value and text behind a pointer.
    Char,
    /// A struct, a union or anything else that is not passed as a number.
    Other(String),
}

fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        // The preprocessor notes where each line comes from.
        if line.trim_start().starts_with('#') {
            continue;
        }
        let cs: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < cs.len() {
            let c = cs[i];
            if c.is_whitespace() {
                i += 1;
            } else if c.is_alphanumeric() || c == '_' {
                let start = i;
                while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_' || cs[i] == '.') {
                    i += 1;
                }
                out.push(cs[start..i].iter().collect());
            } else if c == '"' || c == '\'' {
                let start = i;
                i += 1;
                while i < cs.len() && cs[i] != c {
                    i += if cs[i] == '\\' { 2 } else { 1 };
                }
                i = (i + 1).min(cs.len());
                out.push(cs[start..i].iter().collect());
            } else if cs[i..].starts_with(&['.', '.', '.']) {
                out.push("...".to_string());
                i += 3;
            } else {
                out.push(c.to_string());
                i += 1;
            }
        }
    }
    out
}

/// The declarations at the top: each one up to its `;`, without what is
/// between braces. A function with a body is not a declaration to keep.
fn declarations(toks: &[String]) -> Vec<Vec<String>> {
    let (mut out, mut cur, mut i) = (Vec::new(), Vec::new(), 0);
    while i < toks.len() {
        match toks[i].as_str() {
            ";" => out.push(std::mem::take(&mut cur)),
            "{" => {
                let mut depth = 0;
                while i < toks.len() {
                    match toks[i].as_str() {
                        "{" => depth += 1,
                        "}" => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    i += 1;
                }
                // After the body of a function nothing follows.
                if cur.last().is_some_and(|t| t == ")") && cur.first().is_none_or(|t| t != "typedef") {
                    cur.clear();
                } else {
                    cur.push("{}".to_string());
                }
            }
            _ => cur.push(toks[i].clone()),
        }
        i += 1;
    }
    out
}

/// Takes out what says nothing about the types: attributes, the name the
/// linker uses, and the words about storage and inlining.
fn plain(decl: &[String]) -> Vec<String> {
    let (mut out, mut i) = (Vec::new(), 0);
    while i < decl.len() {
        let t = decl[i].as_str();
        let group = matches!(t, "__attribute__" | "__attribute" | "__asm__" | "__asm" | "asm" | "__declspec" | "_Alignas" | "__nonnull" | "__wur" | "__THROW");
        if group && decl.get(i + 1).is_some_and(|n| n == "(") {
            let mut depth = 0;
            i += 1;
            while i < decl.len() {
                match decl[i].as_str() {
                    "(" => depth += 1,
                    ")" => depth -= 1,
                    _ => {}
                }
                i += 1;
                if depth == 0 {
                    break;
                }
            }
            continue;
        }
        let skip = matches!(
            t,
            "extern" | "static" | "inline" | "__inline" | "__inline__" | "__extension__" | "_Noreturn" | "register" | "volatile" | "__volatile__" | "restrict" | "__restrict" | "__restrict__" | "__const" | "_Nullable" | "_Nonnull" | "_Null_unspecified" | "__signed__"
        );
        if t == "__const" {
            out.push("const".to_string());
        } else if t == "__signed__" {
            out.push("signed".to_string());
        } else if !skip {
            out.push(t.to_string());
        }
        i += 1;
    }
    out
}

struct Reader {
    typedefs: HashMap<String, CType>,
    /// The size in bytes of `long` on this machine.
    long: usize,
}

impl Reader {
    /// The type some words say, without a name among them.
    fn ctype(&self, words: &[String]) -> CType {
        let stars = words.iter().filter(|w| *w == "*").count();
        let first_star = words.iter().position(|w| w == "*").unwrap_or(words.len());
        let is_const = words[..first_star].iter().any(|w| w == "const");
        let base: Vec<&str> = words[..first_star].iter().map(|w| w.as_str()).filter(|w| *w != "const").collect();
        let mut t = self.base(&base);
        for k in 0..stars {
            t = CType::Pointer(Box::new(t), k == 0 && is_const);
        }
        t
    }

    fn base(&self, words: &[&str]) -> CType {
        let has = |w: &str| words.contains(&w);
        let longs = words.iter().filter(|w| **w == "long").count();
        if words.is_empty() {
            return CType::Other("nothing".into());
        }
        if matches!(words[0], "struct" | "union") {
            return CType::Other(words.join(" "));
        }
        if words[0] == "enum" {
            return CType::Scalar("i32");
        }
        if let [one] = words {
            if let Some(t) = self.typedefs.get(*one) {
                return t.clone();
            }
        }
        let known = |w: &&str| matches!(*w, "void" | "char" | "short" | "int" | "long" | "signed" | "unsigned" | "float" | "double" | "_Bool" | "bool");
        if !words.iter().all(known) {
            return CType::Other(words.join(" "));
        }
        let unsigned = has("unsigned");
        let pick = |s: &'static str, u: &'static str| CType::Scalar(if unsigned { u } else { s });
        if has("void") {
            CType::Void
        } else if has("_Bool") || has("bool") {
            CType::Scalar("bool")
        } else if has("float") {
            CType::Scalar("f32")
        } else if has("double") {
            if longs > 0 { CType::Other("long double".into()) } else { CType::Scalar("f64") }
        } else if has("char") {
            if has("signed") {
                CType::Scalar("i8")
            } else if unsigned {
                CType::Scalar("u8")
            } else {
                CType::Char
            }
        } else if has("short") {
            pick("i16", "u16")
        } else if longs >= 2 || (longs == 1 && self.long == 8) {
            pick("i64", "u64")
        } else {
            pick("i32", "u32")
        }
    }

    /// `typedef` gives a name to a type: the ones that are a number or a
    /// pointer are followed, the rest are remembered as something else.
    fn typedef(&mut self, decl: &[String]) {
        let words = &decl[1..];
        let Some(name) = words.last() else { return };
        // A function or an array type has no simple reading.
        if words.iter().any(|w| w == "(" || w == "[") {
            let what = if words.iter().any(|w| w == "(") { "the function type" } else { "the array type" };
            self.typedefs.insert(name.clone(), CType::Other(format!("{what} {name}")));
            return;
        }
        let mut t = self.ctype(&words[..words.len() - 1]);
        if words.iter().any(|w| w == "{}") {
            let stars = words.iter().filter(|w| *w == "*").count();
            t = CType::Other(name.clone());
            for _ in 0..stars {
                t = CType::Pointer(Box::new(t), false);
            }
        }
        // A name for a struct says more than the struct does.
        if matches!(&t, CType::Other(what) if what.starts_with("struct ") || what.starts_with("union ")) {
            t = CType::Other(name.clone());
        }
        self.typedefs.insert(name.clone(), t);
    }
}

/// How a parameter is written in Mou, or why it cannot be.
fn param_type(t: &CType) -> Result<String, String> {
    match t {
        CType::Scalar(s) => Ok(s.to_string()),
        CType::Char => Ok("i8".into()),
        CType::Pointer(inner, is_const) => match &**inner {
            CType::Char if *is_const => Ok("str".into()),
            CType::Char => Ok("mut [u8]".into()),
            CType::Scalar(s) if *s != "bool" => Ok(if *is_const { format!("[{s}]") } else { format!("mut [{s}]") }),
            _ => Ok("ptr?".into()),
        },
        CType::Void => Err("a parameter of type `void`".into()),
        CType::Other(what) => Err(format!("`{what}` passed by value")),
    }
}

fn result_type(t: &CType) -> Result<String, String> {
    match t {
        CType::Void => Ok(String::new()),
        CType::Scalar(s) => Ok(format!(" -> {s}")),
        CType::Char => Ok(" -> i8".into()),
        CType::Pointer(inner, _) => Ok(if **inner == CType::Char { " -> str?" } else { " -> ptr?" }.into()),
        CType::Other(what) => Err(format!("it gives `{what}` by value")),
    }
}

/// A name of C as a name of Mou.
fn mou_name(name: &str, k: usize) -> String {
    let n = name.trim_start_matches('_');
    if n.is_empty() || !n.chars().next().is_some_and(|c| c.is_alphabetic()) {
        return format!("a{k}");
    }
    if crate::lexer::KEYWORDS.contains(&n) || matches!(n, "self" | "str" | "int" | "bool" | "ptr") {
        return format!("{n}_");
    }
    n.to_string()
}

/// The declaration in Mou of a function of C, or why there is none.
fn function(r: &Reader, decl: &[String]) -> Option<(String, Result<String, String>)> {
    let open = decl.iter().position(|t| t == "(")?;
    let name = decl.get(open.checked_sub(1)?)?.clone();
    if !name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_') || open < 2 {
        return None;
    }
    // The parameters end where their parenthesis closes.
    let mut depth = 0;
    let mut close = open;
    for (i, t) in decl.iter().enumerate().skip(open) {
        match t.as_str() {
            "(" => depth += 1,
            ")" => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            close = i;
            break;
        }
    }
    if decl[..open - 1].iter().any(|t| t == "(" || t == "=" || t == "{}") {
        return None;
    }
    let made = (|| {
        if close + 1 != decl.len() {
            return Err("it gives a function or an array".to_string());
        }
        let ret = result_type(&r.ctype(&decl[..open - 1]))?;
        let inner = &decl[open + 1..close];
        let mut params: Vec<&[String]> = Vec::new();
        let (mut depth, mut start) = (0, 0);
        for (i, t) in inner.iter().enumerate() {
            match t.as_str() {
                "(" | "[" => depth += 1,
                ")" | "]" => depth -= 1,
                "," if depth == 0 => {
                    params.push(&inner[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        if start < inner.len() {
            params.push(&inner[start..]);
        }
        if let [only] = params.as_slice() {
            if only.len() == 1 && only[0] == "void" {
                params.clear();
            }
        }
        let mut out = Vec::new();
        for (k, p) in params.iter().enumerate() {
            if p.iter().any(|t| t == "...") {
                return Err("it takes any number of arguments".to_string());
            }
            if p.iter().any(|t| t == "(") {
                return Err("it takes a function".to_string());
            }
            // `int xs[4]` is a pointer when it is a parameter.
            let array = p.iter().any(|t| t == "[");
            let mut words: Vec<String> = p.iter().take_while(|t| *t != "[").cloned().collect();
            // The last word is the name, when the words before it are a type.
            let mut pname = format!("a{k}");
            let named = words.len() > 1
                && words.last().is_some_and(|w| {
                    let w = w.as_str();
                    w != "*" && w != "const" && !r.typedefs.contains_key(w) && !matches!(w, "void" | "char" | "short" | "int" | "long" | "signed" | "unsigned" | "float" | "double" | "_Bool")
                })
                && !matches!(words[words.len() - 2].as_str(), "struct" | "union" | "enum");
            if named {
                pname = mou_name(&words.pop().unwrap_or_default(), k);
            }
            if array {
                words.push("*".to_string());
            }
            let ty = param_type(&r.ctype(&words))?;
            out.push((pname, ty));
        }
        // Two parameters cannot share a name.
        for k in 0..out.len() {
            if out[..k].iter().any(|(n, _)| *n == out[k].0) {
                out[k].0 = format!("{}{k}", out[k].0);
            }
        }
        let params: Vec<String> = out.iter().map(|(n, t)| if let Some(inner) = t.strip_prefix("mut ") { format!("{n}: mut {inner}") } else { format!("{n}: {t}") }).collect();
        Ok(format!("fn {name}({}){ret}", params.join(", ")))
    })();
    Some((name, made))
}

/// The text of the bindings of a header, from what the preprocessor gives.
fn bindings(header: &str, alias: &str, preprocessed: &str) -> String {
    let toks = tokens(preprocessed);
    let mut r = Reader { typedefs: HashMap::new(), long: 8 };
    let mut fns: Vec<(String, Result<String, String>)> = Vec::new();
    for decl in declarations(&toks) {
        let decl = plain(&decl);
        // The line this module adds to ask for the size of `long`.
        if let [mark, size] = decl.as_slice() {
            if mark == "mou_long_size" {
                r.long = size.parse().unwrap_or(8);
                continue;
            }
        }
        if decl.first().is_some_and(|t| t == "typedef") {
            r.typedef(&decl);
            continue;
        }
        let Some((name, made)) = function(&r, &decl) else { continue };
        // Names that start with `_` belong to the library itself.
        if name.starts_with('_') || fns.iter().any(|(n, _)| *n == name) {
            continue;
        }
        fns.push((name, made));
    }
    fns.sort_by(|a, b| a.0.cmp(&b.0));
    let good: Vec<&String> = fns.iter().filter_map(|(_, m)| m.as_ref().ok()).collect();
    let left: Vec<(&String, &String)> = fns.iter().filter_map(|(n, m)| m.as_ref().err().map(|why| (n, why))).collect();
    let mut out = String::new();
    out.push_str(&format!("// Written by `mou bind {header}`: {} functions, {} left out.\n", good.len(), left.len()));
    out.push_str("// A pointer to numbers is written as an array (`[T]`, or `mut [T]` when C can\n");
    out.push_str("// write through it), text as `str`, and any other pointer as `ptr?`. What a\n");
    out.push_str("// pointer is for is not in the header: check the ones you use.\n");
    out.push_str("@profile(system)\n\n");
    out.push_str(&format!("import c \"{header}\" as {alias} {{\n"));
    for g in &good {
        out.push_str(&format!("    {g}\n"));
    }
    out.push_str("}\n");
    if !left.is_empty() {
        out.push_str("\n// Left out:\n");
        for (n, why) in left {
            out.push_str(&format!("//   {n}: {why}\n"));
        }
    }
    out
}

/// `mou bind <header> [name]`: prints the bindings, or says what failed.
pub fn bind(header: &str, alias: Option<&str>) -> Result<String, String> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let stem = header.rsplit('/').next().unwrap_or(header).split('.').next().unwrap_or("c");
    let stem: String = stem.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
    let alias = alias.map(str::to_string).unwrap_or_else(|| mou_name(&stem, 0));
    let mut child = Command::new(&cc)
        .args(["-E", "-P", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run the C compiler `{cc}`: {e}"))?;
    let source = format!("#include <{header}>\nmou_long_size __SIZEOF_LONG__;\n");
    // The compiler reads all of it before it writes, so this cannot block.
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(source.as_bytes()).map_err(|e| format!("cannot write to `{cc}`: {e}"))?;
    }
    let done = child.wait_with_output().map_err(|e| format!("cannot run `{cc}`: {e}"))?;
    if !done.status.success() {
        let why = String::from_utf8_lossy(&done.stderr);
        return Err(format!("the C compiler cannot read `{header}`: {}", why.lines().next().unwrap_or("no reason given")));
    }
    Ok(bindings(header, &alias, &String::from_utf8_lossy(&done.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With the C compiler of this machine: what a real header gives is
    /// accepted by the checker.
    #[test]
    fn a_real_header_is_bound() {
        let Ok(out) = bind("math.h", None) else {
            eprintln!("no C compiler: `mou bind` was not tested on a real header");
            return;
        };
        assert!(out.contains("import c \"math.h\" as math {"), "{out}");
        assert!(out.contains("    fn sqrt(x: f64) -> f64\n"), "{out}");
        assert!(out.contains("    fn frexp(x: f64, exponent: mut [i32]) -> f64\n"), "{out}");
        let file = crate::load::load("math.mou", &out).1.unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
        crate::check::analyze(&file).unwrap_or_else(|es| panic!("{:?}", es.iter().map(|e| &e.msg).collect::<Vec<_>>()));
    }

    #[test]
    fn declarations_are_read() {
        let src = "mou_long_size 8;
typedef unsigned long size_t;
typedef struct { int quot; int rem; } div_t;
typedef struct _IO_FILE FILE;
extern double sqrt (double __x) __attribute__ ((__nothrow__ , __leaf__));
extern size_t strlen (const char *__s) __attribute__ ((__pure__));
extern char *strerror (int __errnum);
extern int printf (const char *__restrict __format, ...);
extern div_t div (int __numer, int __denom);
extern void *memcpy (void *__restrict __dest, const void *__restrict __src, size_t __n);
extern double frexp (double __x, int *__exponent);
extern int memcmp2 (const unsigned char *a, const unsigned char *b, unsigned n);
extern int fclose (FILE *__stream);
extern void qsort (void *base, size_t n, size_t size, int (*cmp) (const void *, const void *));
static inline int twice (int x) { return x * 2; }
extern void (*signal (int sig, void (*handler) (int))) (int);
extern int __hidden (int);
extern long double sqrtl (long double x);
extern void srand (unsigned int __seed);
extern int rand (void);
extern int pick (int, int type, char fn[8]);
";
        let out = bindings("x.h", "x", src);
        for line in [
            "fn sqrt(x: f64) -> f64",
            "fn strlen(s: str) -> u64",
            "fn strerror(errnum: i32) -> str?",
            "fn memcpy(dest: ptr?, src: ptr?, n: u64) -> ptr?",
            "fn frexp(x: f64, exponent: mut [i32]) -> f64",
            "fn memcmp2(a: [u8], b: [u8], n: u32) -> i32",
            "fn fclose(stream: ptr?) -> i32",
            "fn srand(seed: u32)\n",
            "fn rand() -> i32",
            "fn pick(a0: i32, type_: i32, fn_: mut [u8]) -> i32",
            "//   printf: it takes any number of arguments",
            "//   div: it gives `div_t` by value",
            "//   qsort: it takes a function",
            "//   sqrtl: it gives `long double` by value",
            "import c \"x.h\" as x {",
        ] {
            assert!(out.contains(line), "missing {line:?} in:\n{out}");
        }
        for absent in ["twice", "hidden", "fn signal"] {
            assert!(!out.contains(absent), "{absent} should not be there:\n{out}");
        }
    }
}
