//! Finds and parses the modules a program is made of.
//!
//! A file is a module. `import a.b` loads `a/b.mou`, found next to the file
//! given on the command line; `import std.name` loads a module of the
//! standard library, which is part of the compiler. The declarations of all
//! modules end up in one `File`, each module keeping its own names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::ast::{File, ModImport, Module};
use crate::lexer::{Error, Span, FILE_SHIFT};
use crate::parser;

/// Standard modules written in the language itself.
const STD: &[(&str, &str)] = &[
    ("std.cmp", include_str!("../std/cmp.mou")),
    ("std.hash", include_str!("../std/hash.mou")),
    ("std.list", include_str!("../std/list.mou")),
    ("std.map", include_str!("../std/map.mou")),
    ("std.math", include_str!("../std/math.mou")),
    ("std.text", include_str!("../std/text.mou")),
];

/// Modules the compiler provides directly.
const BUILTIN: &[&str] = &["std.io", "std.raw"];

pub struct Source {
    pub path: String,
    pub text: String,
}

/// The text of every module, to show where an error is.
pub struct Sources(pub Vec<Source>);

impl Sources {
    /// The file a position is in, and its line there.
    pub fn locate(&self, span: Span) -> (&Source, u32) {
        let file = (span.line >> FILE_SHIFT) as usize;
        (&self.0[file.min(self.0.len() - 1)], span.line & ((1 << FILE_SHIFT) - 1))
    }

    pub fn paths(&self) -> Vec<String> {
        self.0.iter().map(|s| s.path.clone()).collect()
    }
}

struct Loader {
    root: PathBuf,
    sources: Vec<Source>,
    /// Module name to its number. The program's own file is module 0.
    ids: HashMap<String, usize>,
    modules: Vec<Option<Module>>,
    merged: File,
    errors: Vec<Error>,
}

/// Loads a program: the file at `path`, whose text is `text`, and
/// everything it imports.
pub fn load(path: &str, text: &str) -> (Sources, Result<File, Vec<Error>>) {
    let root = Path::new(path).parent().map(Path::to_path_buf).unwrap_or_default();
    let mut ld = Loader {
        root,
        sources: Vec::new(),
        ids: HashMap::new(),
        modules: Vec::new(),
        merged: File { profile: None, imports: Vec::new(), decls: Vec::new(), modules: Vec::new() },
        errors: Vec::new(),
    };
    ld.module(String::new(), path.to_string(), text.to_string());
    let sources = Sources(ld.sources);
    if !ld.errors.is_empty() {
        return (sources, Err(ld.errors));
    }
    ld.merged.modules = ld.modules.into_iter().flatten().collect();
    (sources, Ok(ld.merged))
}

/// A loaded program as text, in the form the loader written in Mou
/// (`compiler/load.mou`) prints it.
pub fn dump(path: &str, text: &str) -> String {
    let file = match load(path, text).1 {
        Ok(f) => f,
        Err(errors) => return errors.iter().map(|e| format!("error {}:{} {}\n", e.span.line, e.span.col, e.msg)).collect(),
    };
    let paths = load(path, text).0.paths();
    let mut out = String::new();
    let opt = |o: &Option<String>| o.clone().unwrap_or_else(|| "_".to_string());
    for (k, m) in file.modules.iter().enumerate() {
        let name = if m.name.is_empty() { "-" } else { &m.name };
        // The number of a module is the one of its file.
        let source = (file.decls.get(m.decls.start).map_or(0, |d| d.span.line) >> FILE_SHIFT) as usize;
        let shown = if m.decls.is_empty() { paths.get(k) } else { paths.get(source) };
        out.push_str(&format!("module {k} {name} {} {}..{} {}\n", shown.map_or("?", |p| p.as_str()), m.decls.start, m.decls.end, opt(&m.profile)));
        for i in &m.imports {
            let target = i.module.map_or("_".to_string(), |n| n.to_string());
            out.push_str(&format!("  import {} {} {target} {} {}\n", i.alias, i.path, opt(&i.origin), crate::ast::decls_dump(&i.decls)));
        }
    }
    out.push_str(&crate::ast::dump(&Ok(file)));
    out
}

impl Loader {
    /// Parses a module and, first, the modules it imports. Returns its number.
    fn module(&mut self, name: String, path: String, text: String) -> usize {
        let id = self.sources.len();
        self.ids.insert(name.clone(), id);
        self.modules.push(None);
        // Lines of module `id` are numbered from `id << FILE_SHIFT`, so that
        // a position alone says which file it is in.
        let origin = Span { line: ((id as u32) << FILE_SHIFT) + 1, col: 1 };
        let parsed = parser::parse_file_at(&text, origin);
        self.sources.push(Source { path, text });
        let mut file = match parsed {
            Ok(f) => f,
            Err(e) => {
                self.errors.push(e);
                return id;
            }
        };
        let mut imports = Vec::new();
        for imp in &mut file.imports {
            let alias = match (&imp.alias, &imp.origin) {
                (Some(a), _) => a.clone(),
                (None, None) => imp.path.rsplit('.').next().unwrap_or(&imp.path).to_string(),
                // A foreign import without a name brings nothing the checker can see.
                (None, Some(_)) => continue,
            };
            let module = if imp.origin.is_some() || BUILTIN.contains(&imp.path.as_str()) {
                None
            } else {
                self.import(&imp.path, origin)
            };
            imports.push(ModImport { alias, path: imp.path.clone(), module, origin: imp.origin.clone(), decls: std::mem::take(&mut imp.decls) });
        }
        let profile = file.profile.clone();
        if id == 0 {
            self.merged.profile = file.profile;
            self.merged.imports = file.imports;
        }
        let start = self.merged.decls.len();
        self.merged.decls.extend(file.decls);
        self.modules[id] = Some(Module { name, decls: start..self.merged.decls.len(), imports, profile });
        id
    }

    fn import(&mut self, name: &str, at: Span) -> Option<usize> {
        if let Some(id) = self.ids.get(name) {
            return Some(*id);
        }
        if self.sources.len() >= (u32::MAX >> FILE_SHIFT) as usize {
            self.errors.push(Error { msg: "too many modules".into(), span: at });
            return None;
        }
        if let Some((_, text)) = STD.iter().find(|(n, _)| *n == name) {
            let path = format!("<{name}>");
            return Some(self.module(name.to_string(), path, text.to_string()));
        }
        if name.starts_with("std.") {
            self.errors.push(Error { msg: format!("the standard library has no module `{name}`"), span: at });
            return None;
        }
        let mut path = self.root.clone();
        for part in name.split('.') {
            path.push(part);
        }
        path.set_extension("mou");
        let shown = path.display().to_string();
        match std::fs::read_to_string(&path) {
            Ok(text) => Some(self.module(name.to_string(), shown, text)),
            Err(e) => {
                self.errors.push(Error { msg: format!("cannot load the module `{name}` from {shown}: {e}"), span: at });
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, interp, mir};

    #[test]
    fn modules_are_loaded_from_files_and_keep_their_names() {
        let dir = std::env::temp_dir().join(format!("mou-load-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("geo")).unwrap();
        std::fs::write(
            dir.join("geo/shape.mou"),
            "import util\npub struct Point { x: int }\npub fn twice(p: Point) -> int { util.double(p.x) }\nfn hidden() -> int { 0 }\n\
             pub fn bad(xs: [int]) -> int { xs[9] }\n",
        )
        .unwrap();
        std::fs::write(dir.join("util.mou"), "pub fn double(n: int) -> int { n * 2 }\n").unwrap();
        let root = dir.join("main.mou").display().to_string();
        let run = |src: &str| -> Result<mir::Value, (String, String)> {
            let (sources, file) = load(&root, src);
            let fail = |es: Vec<Error>| {
                let e = &es[0];
                (sources.locate(e.span).0.path.clone(), e.msg.clone())
            };
            let file = file.map_err(fail)?;
            let (_, info) = check::analyze(&file).map_err(fail)?;
            let program = mir::lower(&file, &info).map_err(fail)?;
            let mut out = Vec::new();
            interp::run(&program, program.main.unwrap(), &mut out).map_err(|p| (sources.locate(p.span).0.path.clone(), p.msg))
        };
        // Two modules can use the same name for different things.
        let ok = "import geo.shape\nimport geo.shape as g\nstruct Point { y: int }\n\
                  fn main() -> int { shape.twice(g.Point(4)) + Point(1).y }";
        assert_eq!(run(ok), Ok(mir::Value::Int(9)));
        let (file, msg) = run("import geo.shape\nfn main() -> int { shape.hidden() }").unwrap_err();
        assert!(file.ends_with("main.mou") && msg.contains("private to the module `geo.shape`"), "{file}: {msg}");
        let (file, msg) = run("import geo.nothing\nfn main() { }").unwrap_err();
        assert!(file.ends_with("main.mou") && msg.contains("cannot load the module `geo.nothing`"), "{file}: {msg}");
        // A failure is reported in the file it happens in.
        let (file, msg) = run("import geo.shape\nfn main() -> int { shape.bad([1]) }").unwrap_err();
        assert!(file.ends_with("shape.mou") && msg.contains("out of range"), "{file}: {msg}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
