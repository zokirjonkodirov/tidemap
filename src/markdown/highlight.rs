//! Light syntax colouring for fenced code on the rendered page.
//!
//! One small tokenizer drives every language: comments, strings, numbers, keywords, calls and
//! types, with a few special cases (JSON keys, diffs, markup, YAML-like keys). Unknown languages
//! stay plain. This is colour for reading, not a parser.

use egui::{Color32, FontId, TextFormat, text::LayoutJob};

use crate::theme::Palette;

struct Lang {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    block_comment: Option<(&'static str, &'static str)>,
    quotes: &'static str,
    case_insensitive: bool,
    types: bool,
    calls: bool,
    special: Special,
}

#[derive(Clone, Copy, PartialEq)]
enum Special {
    None,
    Json,
    Diff,
    Markup,
    Keys,
    Css,
    Markdown,
}

const RUST: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];
const JS: &[&str] = &[
    "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else", "enum",
    "export", "extends", "false", "finally", "for", "from", "function", "if", "implements", "import", "in", "instanceof", "interface", "let",
    "new", "null", "of", "private", "protected", "public", "readonly", "return", "static", "super", "switch", "this", "throw", "true", "try",
    "type", "typeof", "undefined", "var", "void", "while", "with", "yield",
];
const PYTHON: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except", "False", "finally", "for",
    "from", "global", "if", "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "self", "True", "try",
    "while", "with", "yield",
];
const GO: &[&str] = &[
    "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough", "for", "func", "go", "goto", "if", "import",
    "interface", "map", "package", "range", "return", "select", "struct", "switch", "type", "var", "true", "false", "nil",
];
const C_LIKE: &[&str] = &[
    "abstract", "auto", "bool", "break", "case", "catch", "char", "class", "const", "continue", "default", "delete", "do", "double", "else",
    "enum", "extends", "extern", "false", "final", "finally", "float", "for", "foreach", "if", "implements", "import", "in", "inline", "int",
    "interface", "internal", "long", "namespace", "new", "null", "nullptr", "operator", "override", "package", "private", "protected", "public",
    "readonly", "return", "short", "signed", "sizeof", "static", "string", "struct", "switch", "template", "this", "throw", "throws", "true",
    "try", "typedef", "typename", "union", "unsigned", "using", "var", "virtual", "void", "volatile", "while",
];
const SHELL: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "function", "in", "return", "export", "local", "echo",
    "cd", "exit", "set", "unset", "source", "alias", "true", "false",
];
const SQL: &[&str] = &[
    "select", "from", "where", "insert", "into", "values", "update", "set", "delete", "create", "table", "drop", "alter", "join", "left",
    "right", "inner", "outer", "on", "group", "by", "order", "having", "limit", "as", "and", "or", "not", "null", "primary", "key", "foreign",
    "references", "index", "distinct", "union", "all", "case", "when", "then", "else", "end", "in", "is", "like", "between", "exists",
];
const CONST_WORDS: &[&str] = &["true", "false", "null", "yes", "no", "on", "off"];

fn lang(name: &str) -> Option<Lang> {
    let base = |keywords, line_comments, block_comment, quotes, types, calls| Lang {
        keywords,
        line_comments,
        block_comment,
        quotes,
        case_insensitive: false,
        types,
        calls,
        special: Special::None,
    };
    let c_block = Some(("/*", "*/"));
    Some(match name.to_ascii_lowercase().as_str() {
        "rust" | "rs" => base(RUST, &["//"], c_block, "\"", true, true),
        "js" | "javascript" | "jsx" | "mjs" | "cjs" | "ts" | "typescript" | "tsx" => base(JS, &["//"], c_block, "\"'`", true, true),
        "py" | "python" | "python3" => base(PYTHON, &["#"], None, "\"'", true, true),
        "go" | "golang" => base(GO, &["//"], c_block, "\"'`", true, true),
        "c" | "h" | "cpp" | "c++" | "cc" | "hpp" | "java" | "cs" | "csharp" | "kotlin" | "kt" | "swift" | "dart" | "php" | "scala" => {
            base(C_LIKE, &["//"], c_block, "\"'", true, true)
        }
        "sh" | "bash" | "shell" | "zsh" | "console" | "powershell" | "ps1" | "bat" | "cmd" => {
            base(SHELL, &["#"], None, "\"'", false, false)
        }
        "sql" | "mysql" | "postgres" | "postgresql" | "sqlite" => Lang { case_insensitive: true, ..base(SQL, &["--"], c_block, "'\"", false, false) },
        "json" | "jsonc" | "json5" => Lang { special: Special::Json, ..base(CONST_WORDS, &["//"], c_block, "\"", false, false) },
        "yaml" | "yml" | "toml" | "ini" | "conf" | "env" | "dotenv" => {
            Lang { special: Special::Keys, ..base(CONST_WORDS, &["#", ";"], None, "\"'", false, false) }
        }
        "css" | "scss" | "less" => Lang { special: Special::Css, ..base(&[], &["//"], c_block, "\"'", false, false) },
        "html" | "xml" | "svg" | "vue" | "xhtml" => Lang { special: Special::Markup, ..base(&[], &[], Some(("<!--", "-->")), "\"'", false, false) },
        "markdown" | "md" | "mdx" => Lang { special: Special::Markdown, ..base(&[], &[], None, "", false, false) },
        "diff" | "patch" => Lang { special: Special::Diff, ..base(&[], &[], None, "", false, false) },
        _ => return None,
    })
}

struct Colors {
    ink: Color32,
    keyword: Color32,
    string: Color32,
    number: Color32,
    comment: Color32,
    call: Color32,
    ty: Color32,
    punct: Color32,
    key: Color32,
    add: Color32,
    del: Color32,
}

fn colors(p: Palette) -> Colors {
    let dark = p.ink.r() > 200;
    let c = |l: (u8, u8, u8), d: (u8, u8, u8)| {
        let (r, g, b) = if dark { d } else { l };
        Color32::from_rgb(r, g, b)
    };
    Colors {
        ink: p.ink,
        keyword: c((0x8a, 0x3f, 0xa8), (0xc6, 0x9c, 0xe8)),
        string: c((0x2f, 0x7a, 0x4c), (0x8e, 0xc9, 0x9f)),
        number: c((0xb0, 0x62, 0x1a), (0xe3, 0xa8, 0x6b)),
        comment: p.muted,
        call: c((0x2a, 0x5f, 0xa8), (0x84, 0xb4, 0xee)),
        ty: c((0x0f, 0x78, 0x85), (0x7a, 0xcf, 0xd8)),
        punct: p.secondary,
        key: c((0x2a, 0x5f, 0xa8), (0x84, 0xb4, 0xee)),
        add: c((0x1f, 0x7a, 0x3a), (0x8e, 0xd0, 0x9c)),
        del: c((0xb0, 0x35, 0x35), (0xee, 0x9a, 0x9a)),
    }
}

struct Out<'a> {
    job: LayoutJob,
    font: &'a FontId,
    line: f32,
}

impl Out<'_> {
    fn put(&mut self, text: &str, color: Color32, italics: bool) {
        if text.is_empty() {
            return;
        }
        self.job.append(
            text,
            0.0,
            TextFormat { font_id: self.font.clone(), color, italics, line_height: Some(self.line), ..Default::default() },
        );
    }
}

/// A layout job for `text` coloured as `language` (plain when unknown). Never wraps.
pub fn code_job(language: &str, text: &str, p: Palette, font: FontId, line: f32) -> LayoutJob {
    let c = colors(p);
    let mut out = Out { job: LayoutJob::default(), font: &font, line };
    out.job.wrap.max_width = f32::INFINITY;
    let Some(lang) = lang(language) else {
        out.put(text, c.ink, false);
        return out.job;
    };
    if lang.special == Special::Diff {
        for piece in text.split_inclusive('\n') {
            let color = match piece.as_bytes().first() {
                Some(b'+') => c.add,
                Some(b'-') => c.del,
                Some(b'@') => c.keyword,
                _ => c.ink,
            };
            out.put(piece, color, false);
        }
        return out.job;
    }
    if lang.special == Special::Markdown {
        for piece in text.split_inclusive('\n') {
            markdown_line(&mut out, piece, &c);
        }
        return out.job;
    }
    tokenize(&mut out, &lang, text, &c);
    out.job
}

/// One line of Markdown source: headings, quotes, list markers, `code`, links and tags.
fn markdown_line(out: &mut Out, line: &str, c: &Colors) {
    let body = line.trim_end_matches('\n');
    let newline = &line[body.len()..];
    let trimmed = body.trim_start();
    let indent = body.len() - trimmed.len();
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        out.put(body, c.comment, false);
        out.put(newline, c.ink, false);
        return;
    }
    let hashes = trimmed.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with(' ') {
        out.put(&body[..indent], c.ink, false);
        out.put(&trimmed[..hashes], c.comment, false);
        out.put(&trimmed[hashes..], c.keyword, false);
        out.put(newline, c.ink, false);
        return;
    }
    let mut rest = body;
    if let Some(quote) = trimmed.strip_prefix('>') {
        out.put(&body[..indent + 1], c.comment, false);
        rest = quote;
    } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        out.put(&body[..indent], c.ink, false);
        out.put(&trimmed[..1], c.keyword, false);
        rest = &trimmed[1..];
    }
    let bytes = rest.as_bytes();
    let (mut from, mut i) = (0, 0);
    while i < bytes.len() {
        let close = match bytes[i] {
            b'`' => rest[i + 1..].find('`').map(|n| (i + n + 2, c.string)),
            b'[' => rest[i..].find("](").and_then(|m| rest[i + m..].find(')').map(|n| (i + m + n + 1, c.key))),
            b'<' => rest[i..].find('>').map(|n| (i + n + 1, c.ty)),
            b'*' | b'_' if bytes.get(i + 1) == Some(&bytes[i]) => rest[i + 2..].find(&rest[i..i + 2]).map(|n| (i + n + 4, c.number)),
            _ => None,
        };
        match close {
            Some((end, color)) if end <= rest.len() => {
                out.put(&rest[from..i], c.ink, false);
                out.put(&rest[i..end], color, false);
                i = end;
                from = end;
            }
            _ => i += rest[i..].chars().next().map_or(1, char::len_utf8),
        }
    }
    out.put(&rest[from..], c.ink, false);
    out.put(newline, c.ink, false);
}

fn starts_with_at(text: &str, at: usize, pattern: &str) -> bool {
    !pattern.is_empty() && text[at..].starts_with(pattern)
}

fn tokenize(out: &mut Out, lang: &Lang, text: &str, c: &Colors) {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut plain_from = 0;
    macro_rules! flush {
        ($upto:expr) => {
            if plain_from < $upto {
                out.put(&text[plain_from..$upto], c.ink, false);
            }
        };
    }
    let mut line_start = true;
    while i < text.len() {
        let rest = &text[i..];
        let ch = rest.chars().next().unwrap_or(' ');
        if ch == '\n' {
            i += 1;
            line_start = true;
            continue;
        }
        // Comments.
        if let Some(marker) = lang.line_comments.iter().find(|m| starts_with_at(text, i, m)) {
            let _ = marker;
            flush!(i);
            let end = rest.find('\n').map_or(text.len(), |n| i + n);
            out.put(&text[i..end], c.comment, true);
            i = end;
            plain_from = i;
            continue;
        }
        if let Some((open, close)) = lang.block_comment
            && starts_with_at(text, i, open)
        {
            flush!(i);
            let end = text[i + open.len()..].find(close).map_or(text.len(), |n| i + open.len() + n + close.len());
            out.put(&text[i..end], c.comment, true);
            i = end;
            plain_from = i;
            continue;
        }
        // Markup tags.
        if lang.special == Special::Markup && ch == '<' {
            flush!(i);
            let end = rest.find('>').map_or(text.len(), |n| i + n + 1);
            let tag = &text[i..end];
            let name_end = tag[1..].find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/').map_or(tag.len(), |n| n + 1);
            out.put(&tag[..name_end], c.keyword, false);
            // Attributes: strings coloured, the rest plain.
            let mut attrs = &tag[name_end..];
            while let Some(q) = attrs.find(['"', '\'']) {
                out.put(&attrs[..q], c.key, false);
                let quote = attrs.as_bytes()[q] as char;
                let close = attrs[q + 1..].find(quote).map_or(attrs.len(), |n| q + 1 + n + 1);
                out.put(&attrs[q..close], c.string, false);
                attrs = &attrs[close..];
            }
            out.put(attrs, c.punct, false);
            i = end;
            plain_from = i;
            continue;
        }
        // Strings.
        if lang.quotes.contains(ch) {
            flush!(i);
            let mut j = i + 1;
            let mut escaped = false;
            while j < text.len() {
                let cj = text[j..].chars().next().unwrap_or(' ');
                if cj == '\\' && !escaped {
                    escaped = true;
                } else {
                    if cj == ch && !escaped {
                        j += 1;
                        break;
                    }
                    if cj == '\n' && ch != '`' {
                        break;
                    }
                    escaped = false;
                }
                j += cj.len_utf8();
            }
            let literal = &text[i..j.min(text.len())];
            let key = lang.special == Special::Json && text[j.min(text.len())..].trim_start_matches([' ', '\t']).starts_with(':');
            out.put(literal, if key { c.key } else { c.string }, false);
            i = j.min(text.len());
            plain_from = i;
            line_start = false;
            continue;
        }
        // Numbers.
        if ch.is_ascii_digit() && (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_')) {
            flush!(i);
            let len = rest.find(|x: char| !(x.is_ascii_alphanumeric() || x == '_' || x == '.')).unwrap_or(rest.len());
            out.put(&rest[..len], c.number, false);
            i += len;
            plain_from = i;
            line_start = false;
            continue;
        }
        // Words.
        if ch.is_alphabetic() || ch == '_' || ch == '$' || ch == '@' || (ch == '-' && lang.special == Special::Keys && false) {
            let len = rest
                .find(|x: char| !(x.is_alphanumeric() || x == '_' || x == '$' || (x == '-' && matches!(lang.special, Special::Keys | Special::Css))))
                .unwrap_or(rest.len())
                .max(ch.len_utf8());
            let word = &rest[..len];
            let after = text[i + len..].trim_start_matches([' ', '\t']);
            let color = if lang.keywords.iter().any(|k| if lang.case_insensitive { k.eq_ignore_ascii_case(word) } else { *k == word }) {
                Some(c.keyword)
            } else if matches!(lang.special, Special::Keys | Special::Css) && (after.starts_with(':') || (after.starts_with('=') && line_start)) && (line_start || lang.special == Special::Css) {
                Some(c.key)
            } else if lang.calls && after.starts_with('(') {
                Some(c.call)
            } else if lang.types && word.chars().next().is_some_and(char::is_uppercase) && word.len() > 1 {
                Some(c.ty)
            } else if word.starts_with('$') || word.starts_with('@') {
                Some(c.ty)
            } else {
                None
            };
            if let Some(color) = color {
                flush!(i);
                out.put(word, color, false);
                plain_from = i + len;
            }
            i += len;
            line_start = false;
            continue;
        }
        if ch == ' ' || ch == '\t' {
            i += 1;
            continue;
        }
        // Punctuation.
        if matches!(ch, '{' | '}' | '(' | ')' | '[' | ']' | ';' | ',' | ':' | '=' | '+' | '-' | '*' | '/' | '<' | '>' | '!' | '&' | '|' | '.' | '?' | '%' | '^' | '~') {
            flush!(i);
            out.put(&text[i..i + ch.len_utf8()], c.punct, false);
            plain_from = i + ch.len_utf8();
        }
        i += ch.len_utf8();
        line_start = false;
    }
    flush!(text.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_cover_the_whole_text() {
        let p = crate::theme::palette(false);
        for (lang, text) in [
            ("rust", "fn main() { let x = \"hi\"; // note\n  /* b */ 42 }\n"),
            ("json", "{\"a\": [1, true, null]}"),
            ("python", "def f(x):\n    return 'é' # ü\n"),
            ("html", "<a href=\"x\">t</a><!-- c -->"),
            ("yaml", "key: value\n# c\nlist:\n  - a\n"),
            ("diff", "+a\n-b\n c\n"),
            ("css", "a { color: red; }"),
            ("markdown", "# Title\n> quote **bold** `code` [a](b) <i>\n- item\n```rs\n"),
            ("nonsense", "plain"),
        ] {
            let job = code_job(lang, text, p, FontId::monospace(13.0), 20.0);
            assert_eq!(job.text, text, "{lang} lost or changed text");
        }
    }
}
