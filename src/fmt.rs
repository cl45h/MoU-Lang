//! Formatter: gives a file the standard layout.
//!
//! It works on the text, not on the syntax tree, so comments stay where
//! they are. It fixes the indentation, the spaces inside a line and the
//! blank lines, and never moves anything to another line: where a line
//! ends is the author's choice, and in this language it has a meaning.

use crate::lexer::{self, Tok};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Space,
    Newline,
    Word,
    Punct,
    Str,
    Char,
    LineComment,
    BlockComment,
}

struct Piece {
    kind: Kind,
    text: String,
}

/// Cuts the source into pieces without losing a character.
fn scan(src: &str) -> Result<Vec<Piece>, String> {
    let cs: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let at = |i: usize, s: &str| s.chars().enumerate().all(|(k, c)| cs.get(i + k) == Some(&c));
    while i < cs.len() {
        let c = cs[i];
        let start = i;
        let kind = if c == '\n' {
            i += 1;
            Kind::Newline
        } else if c.is_whitespace() {
            while i < cs.len() && cs[i] != '\n' && cs[i].is_whitespace() {
                i += 1;
            }
            Kind::Space
        } else if at(i, "//") {
            while i < cs.len() && cs[i] != '\n' {
                i += 1;
            }
            Kind::LineComment
        } else if at(i, "/*") {
            let mut depth = 0;
            loop {
                if at(i, "/*") {
                    depth += 1;
                    i += 2;
                } else if at(i, "*/") {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else if i >= cs.len() {
                    return Err("unterminated block comment".into());
                } else {
                    i += 1;
                }
            }
            Kind::BlockComment
        } else if c.is_alphanumeric() || c == '_' {
            while i < cs.len() && (cs[i].is_alphanumeric() || cs[i] == '_') {
                i += 1;
            }
            Kind::Word
        } else if c == '"' {
            i += 1;
            loop {
                match cs.get(i) {
                    None => return Err("unterminated string literal".into()),
                    Some('"') => {
                        i += 1;
                        break;
                    }
                    Some('\\') => i += 2,
                    Some('{') if cs.get(i + 1) == Some(&'{') => i += 2,
                    Some('}') if cs.get(i + 1) == Some(&'}') => i += 2,
                    // An interpolated expression ends at its closing brace.
                    Some('{') => {
                        i += 1;
                        let mut depth = 1;
                        while depth > 0 {
                            match cs.get(i) {
                                None => return Err("unterminated `{` in string interpolation".into()),
                                Some('{') => depth += 1,
                                Some('}') => depth -= 1,
                                Some(_) => {}
                            }
                            i += 1;
                        }
                    }
                    Some(_) => i += 1,
                }
            }
            Kind::Str
        } else if c == '\'' {
            i += if cs.get(i + 1) == Some(&'\\') { 4 } else { 3 };
            if i > cs.len() {
                return Err("unterminated character literal".into());
            }
            Kind::Char
        } else {
            match lexer::PUNCT.iter().find(|p| at(i, p)) {
                Some(p) => i += p.len(),
                None => return Err(format!("unexpected character `{c}`")),
            }
            Kind::Punct
        };
        out.push(Piece { kind, text: cs[start..i.min(cs.len())].iter().collect() });
    }
    Ok(out)
}

/// Operators that are always between two operands: one space on each side.
const SPACED: &[&str] = &[
    "=", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", "->", "=>", "+", "*",
    "/", "%", "<", ">", "<<", ">>", "+%", "-%", "*%", "+|", "-|", "*|",
];

/// Whether a space goes between two pieces of a line. `had` says whether
/// the author wrote one: it decides where the layout has no rule.
fn space(prev: &Piece, cur: &Piece, had: bool) -> bool {
    let punct = |p: &Piece, s: &[&str]| p.kind == Kind::Punct && s.contains(&p.text.as_str());
    if cur.kind == Kind::LineComment {
        return true;
    }
    if cur.kind == Kind::BlockComment || prev.kind == Kind::BlockComment {
        return had;
    }
    if punct(cur, &[",", ";", ")", "]"]) || punct(prev, &["(", "["]) {
        return false;
    }
    if punct(prev, &[","]) {
        return true;
    }
    if punct(cur, &[":"]) {
        return false;
    }
    if punct(prev, &[":"]) {
        return true;
    }
    if punct(cur, SPACED) || punct(prev, SPACED) {
        return true;
    }
    if punct(prev, &["{"]) {
        return !punct(cur, &["}"]);
    }
    if punct(cur, &["{", "}"]) {
        return true;
    }
    had
}

const INDENT: &str = "    ";

/// The text of a file in the standard layout.
pub fn format(src: &str) -> Result<String, String> {
    let pieces = scan(src)?;
    // Lines of the source, each with its pieces and whether a space came
    // before each one. A string or a comment that spans lines is one piece.
    let mut lines: Vec<Vec<(bool, &Piece)>> = vec![Vec::new()];
    let mut had = false;
    for p in &pieces {
        match p.kind {
            Kind::Newline => {
                lines.push(Vec::new());
                had = false;
            }
            Kind::Space => had = true,
            _ => {
                lines.last_mut().expect("there is always a line").push((had, p));
                had = false;
            }
        }
    }
    let opener = |p: &Piece| p.kind == Kind::Punct && matches!(p.text.as_str(), "{" | "(" | "[");
    let closer = |p: &Piece| p.kind == Kind::Punct && matches!(p.text.as_str(), "}" | ")" | "]");
    let mut out = String::new();
    let mut depth: usize = 0;
    // Whether a blank line may come next: not at the start of the file or
    // of a block, and never two in a row.
    let mut blank_ok = false;
    let mut pending_blank = false;
    for line in &lines {
        let Some((_, first)) = line.first() else {
            pending_blank = blank_ok;
            continue;
        };
        let leading = line.iter().take_while(|(_, p)| closer(p)).count();
        // A line that goes on from the one before is indented once more.
        let goes_on = match first.kind {
            Kind::Punct => lexer::CONTINUES_LINE.contains(&first.text.as_str()),
            // The conditions of a function, under its signature.
            Kind::Word => first.text == "where",
            _ => false,
        };
        let level = depth.saturating_sub(leading) + goes_on as usize;
        if pending_blank && leading == 0 {
            out.push('\n');
        }
        pending_blank = false;
        out.push_str(&INDENT.repeat(level));
        for (k, (had, p)) in line.iter().enumerate() {
            if k > 0 && space(line[k - 1].1, p, *had) {
                out.push(' ');
            }
            out.push_str(&p.text);
        }
        out.push('\n');
        for (_, p) in line {
            if opener(p) {
                depth += 1;
            } else if closer(p) {
                depth = depth.saturating_sub(1);
            }
        }
        blank_ok = !line.last().is_some_and(|(_, p)| opener(p));
    }
    // What changed must be layout only.
    if meaning(src)? != meaning(&out)? {
        return Err("internal error in the formatter: the result would not mean the same".into());
    }
    Ok(out)
}

/// The tokens of a text without what the layout may change: where each
/// one is, and how many blank lines there are.
fn meaning(src: &str) -> Result<Vec<Tok>, String> {
    let mut out: Vec<Tok> = Vec::new();
    for t in lexer::lex(src).map_err(|e| e.msg)? {
        let tok = match t.tok {
            Tok::Newline if matches!(out.last(), None | Some(Tok::Newline)) => continue,
            Tok::Str(parts) => Tok::Str(
                parts
                    .into_iter()
                    .map(|p| match p {
                        lexer::StrPart::Expr(text, _) => lexer::StrPart::Expr(text, lexer::Span::default()),
                        lit => lit,
                    })
                    .collect(),
            ),
            // A file may end without a newline.
            Tok::Eof => {
                if out.last() == Some(&Tok::Newline) {
                    out.pop();
                }
                Tok::Eof
            }
            other => other,
        };
        out.push(tok);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn layout() {
        let src = "\n\nimport std.io\n\n\n\nstruct P{x:int,y :int}\nfn main( ) ! io.Error{\n\n  let p=P( 1,2 )   // a point\n\tlet xs=[1,2 , 3]\n\
                   if p.x<2&&xs.len>0{\n io.print(\"{p.x}  {xs[0]+1}\")?\n\n\n }\n  let s = \"a  ,b\"   +   \"c\"\n  let n = -p.x - 1 + xs[0]*2\n\
                   let t = xs\n.len\n\n}\n\n\n";
        let want = "import std.io\n\nstruct P { x: int, y: int }\nfn main() ! io.Error {\n    let p = P(1, 2) // a point\n    let xs = [1, 2, 3]\n\
                    \x20   if p.x < 2 && xs.len > 0 {\n        io.print(\"{p.x}  {xs[0]+1}\")?\n    }\n    let s = \"a  ,b\" + \"c\"\n    let n = -p.x - 1 + xs[0] * 2\n\
                    \x20   let t = xs\n        .len\n}\n";
        let got = format(src).unwrap();
        assert_eq!(got, want);
        assert_eq!(format(&got).unwrap(), got);
    }

    #[test]
    fn every_file_of_the_project_keeps_its_meaning() {
        for dir in ["compiler", "examples", "std", "tests/native"] {
            let dir = format!("{}/{dir}", env!("CARGO_MANIFEST_DIR"));
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_none_or(|e| e != "mou") {
                    continue;
                }
                let src = std::fs::read_to_string(&path).unwrap();
                let once = format(&src).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                assert_eq!(format(&once).unwrap(), once, "{}: formatting twice changes the file", path.display());
            }
        }
    }
}
