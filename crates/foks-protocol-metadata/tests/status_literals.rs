//! Rejects numeric FOKS status-code literals in workspace sources. Code that
//! compares or matches a status code uses the generated `foks_rpc::STATUS_*`
//! constants, whose values `pinned.rs` checks against the pinned upstream
//! source; a literal has no such check.
//!
//! The scan covers `crates/*/src` and `apps/desktop/src-tauri/src`, outside
//! `generated/` and `tests/` directories, `tests.rs` files and `#[cfg(test)]`
//! items. After
//! comments and string literals are removed, it reports a 3-5 digit integer
//! literal in these positions:
//!
//! - a struct field or pattern `code: N` (`RemoteStatus { code: N, .. }`,
//!   `code: N | M`);
//! - a comparison `code == N`, `*code != N`, `error.code() == N` or
//!   `N == code`;
//! - the pattern of `matches!(code, N | M)`;
//! - the arm patterns of `match code { N => .. }`.
//!
//! An operand counts as a status code when the last identifier of its place
//! expression is `code` or `status`, or ends in `_code` or `_status`.

use std::path::{Path, PathBuf};

/// Sources whose status-named values are not FOKS status codes, and why.
/// Each entry must still produce at least one report, so a stale exemption
/// fails the test.
const EXEMPTIONS: &[(&str, &str)] = &[(
    "crates/foks-server/src/operations/management.rs",
    "`response` matches HTTP status codes to reason phrases",
)];

#[test]
fn workspace_sources_compare_status_codes_through_constants() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut directories = vec![root.join("apps/desktop/src-tauri/src")];
    for entry in std::fs::read_dir(root.join("crates")).unwrap() {
        let source = entry.unwrap().path().join("src");
        if source.is_dir() {
            directories.push(source);
        }
    }
    let mut files = Vec::new();
    for directory in &directories {
        collect_sources(directory, &mut files);
    }
    assert!(files.len() > 100, "the scan found too few sources");

    let mut violations = Vec::new();
    let mut exempted = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&file).unwrap();
        let lines = status_literal_lines(&source);
        if lines.is_empty() {
            continue;
        }
        if EXEMPTIONS.iter().any(|(path, _)| *path == relative) {
            exempted.push(relative);
        } else {
            for line in lines {
                violations.push(format!("{relative}:{line}"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "status-code literals; use the foks_rpc::STATUS_* constants, adding \
         a [status_aliases] entry to crates/foks-server/protocol/policy-v1.toml \
         for a code without one:\n{}",
        violations.join("\n")
    );
    for (path, reason) in EXEMPTIONS {
        assert!(
            exempted.iter().any(|exempt| exempt == path),
            "{path} is exempt ({reason}) but has no status-named literal"
        );
    }
}

#[test]
fn scan_reports_each_status_literal_form() {
    for (source, lines) in [
        (
            "fn f(e: E) { if let E::RemoteStatus { code: 1013, .. } = e {} }",
            &[1][..],
        ),
        (
            "fn f(e: E) { matches!(e, E::RemoteStatus {\n code: 7 | 7004, .. }); }",
            &[2],
        ),
        ("fn f(code: u64) -> bool { code == 8012 }", &[1]),
        ("fn f(code: &u64) -> bool { *code != 8016 }", &[1]),
        ("fn f(e: E) -> bool { e.code() == 1_013u64 }", &[1]),
        ("fn f(e: E) -> bool { 1013 == e.status }", &[1]),
        (
            "fn f(status_code: u64) -> bool { status_code == 1013 }",
            &[1],
        ),
        (
            "fn f(code: &u64) -> bool {\n matches!(*code, 1013 | 1030 | 12002) }",
            &[2],
        ),
        (
            "fn f(code: u64) {\n match code {\n 1020 => {}\n 1058 | 1062 => g(),\n _ => {} } }",
            &[3, 4],
        ),
        (
            "fn f(e: E) { match e.status() { 0 => {}, x if x > 1 => {},\n 2001 => {} } }",
            &[2],
        ),
        (
            "#[cfg(test)]\nfn t() {}\nfn f(code: u64) -> bool { code == 1013 }",
            &[3],
        ),
        (
            "fn f(c: char) -> bool { c == '\\'' || br\"\\\" == b\"\\\"\" }\n\
             fn g(code: u64) -> bool { code == 1013 }",
            &[2],
        ),
    ] {
        assert_eq!(status_literal_lines(source), lines, "{source}");
    }
}

#[test]
fn scan_ignores_constants_tests_comments_and_unrelated_literals() {
    for source in [
        "fn f(code: u64) -> bool { code == foks_rpc::STATUS_KV_STALE_CACHE_ERROR }",
        "fn f(code: u64) -> bool { code == 0 || code == 12 }",
        "fn f(count: u64) -> bool { count == 1013 }",
        "fn f(code: u64) { match code { 0 => {} STATUS_RATE_LIMIT_ERROR => {} _ => {} } }",
        "fn f(code: u64) { match code { _ => { let x = 1013; } } }",
        "fn f(code: u64) { match code { c => match c { _ => 1013 } } }",
        "fn f(code: u64) -> u64 { match code { 1 => 1013, _ => 2001 } }",
        "fn f(code: u64) -> bool { // code == 1013\n true }",
        "fn f(code: u64) -> &'static str { /* code == 1013 */ \"code == 1013\" }",
        "fn f(code: u64) -> &'static str { r#\"code: 1013\"# }",
        "fn f(c: char) -> bool { c == '\"' || code == STATUS_X }",
        "fn f(e: E) -> i64 { E::Rpc { code: -32600 }.code }",
        "#[cfg(test)]\nmod tests { fn t(code: u64) -> bool { code == 1013 } }",
        "#[cfg(all(test, unix))]\nfn t(code: u64) -> bool { code == 1013 }",
    ] {
        assert!(
            status_literal_lines(source).is_empty(),
            "false report: {source}"
        );
    }
}

fn collect_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.is_dir() {
            if name != "generated" && name != "tests" {
                collect_sources(&path, files);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            files.push(path);
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Identifier(String),
    /// The digits of an integer literal, without separators or suffix.
    Integer(String),
    Punctuation(&'static str),
}

/// The lines of `source` that hold a status-code literal.
fn status_literal_lines(source: &str) -> Vec<usize> {
    let tokens = remove_test_items(tokenize(source));
    let mut lines = Vec::new();
    let mut report = |index: usize| {
        if let Some((token, line)) = tokens.get(index) {
            if is_status_literal(token) {
                lines.push(*line);
            }
        }
    };
    for index in 0..tokens.len() {
        let token = &tokens[index].0;
        match token {
            // `code: N` and `code: N | M` as a struct field or pattern.
            Token::Identifier(name)
                if name == "code"
                    && punctuation_at(&tokens, index + 1) == Some(":")
                    && index.checked_sub(1).is_some_and(|before| {
                        matches!(punctuation_at(&tokens, before), Some("{" | ","))
                    }) =>
            {
                let mut position = index + 2;
                while position < tokens.len() {
                    report(position);
                    if punctuation_at(&tokens, position + 1) != Some("|") {
                        break;
                    }
                    position += 2;
                }
            }
            Token::Punctuation("==" | "!=") => {
                if index > 0 && status_operand_ends_at(&tokens, index - 1) {
                    report(index + 1);
                }
                if index > 0 && status_operand_starts_at(&tokens, index + 1) {
                    report(index - 1);
                }
            }
            Token::Identifier(name)
                if name == "matches"
                    && punctuation_at(&tokens, index + 1) == Some("!")
                    && punctuation_at(&tokens, index + 2) == Some("(") =>
            {
                let Some(comma) = top_level(&tokens, index + 3, &[","]) else {
                    continue;
                };
                if status_operand_ends_at(&tokens, comma - 1) {
                    let end = top_level(&tokens, comma + 1, &[")", "if"]).unwrap_or(tokens.len());
                    (comma + 1..end).for_each(&mut report);
                }
            }
            Token::Identifier(name) if name == "match" => {
                let Some(open) = top_level(&tokens, index + 1, &["{"]) else {
                    continue;
                };
                if open > index + 1 && status_operand_ends_at(&tokens, open - 1) {
                    arm_patterns(&tokens, open + 1, &mut report);
                }
            }
            _ => {}
        }
    }
    lines.sort_unstable();
    lines.dedup();
    lines
}

fn is_status_literal(token: &Token) -> bool {
    match token {
        Token::Integer(digits) => (3..=5).contains(&digits.trim_start_matches('0').len()),
        _ => false,
    }
}

fn punctuation_at(tokens: &[(Token, usize)], index: usize) -> Option<&'static str> {
    match tokens.get(index) {
        Some((Token::Punctuation(punctuation), _)) => Some(punctuation),
        _ => None,
    }
}

fn is_status_name(name: &str) -> bool {
    name == "code" || name == "status" || name.ends_with("_code") || name.ends_with("_status")
}

/// Whether the place expression that ends at `index` names a status code:
/// `code`, `error.code`, or `error.code()`.
fn status_operand_ends_at(tokens: &[(Token, usize)], index: usize) -> bool {
    let index = if punctuation_at(tokens, index) == Some(")")
        && index > 0
        && punctuation_at(tokens, index - 1) == Some("(")
    {
        index.saturating_sub(2)
    } else {
        index
    };
    matches!(tokens.get(index), Some((Token::Identifier(name), _)) if is_status_name(name))
}

/// Whether the place expression that starts at `index` names a status code:
/// `code`, `*code`, `&error.status` or `error.code()`.
fn status_operand_starts_at(tokens: &[(Token, usize)], mut index: usize) -> bool {
    while matches!(punctuation_at(tokens, index), Some("*" | "&")) {
        index += 1;
    }
    let mut last = None;
    while let Some((Token::Identifier(name), _)) = tokens.get(index) {
        last = Some(name);
        index += 1;
        if punctuation_at(tokens, index) == Some("(")
            && punctuation_at(tokens, index + 1) == Some(")")
        {
            index += 2;
        }
        if punctuation_at(tokens, index) != Some(".") {
            break;
        }
        index += 1;
    }
    last.is_some_and(|name| is_status_name(name))
}

/// The position of the first of `targets` at bracket depth zero from `start`,
/// before the enclosing bracket closes.
fn top_level(tokens: &[(Token, usize)], start: usize, targets: &[&str]) -> Option<usize> {
    let mut depth = 0usize;
    for (index, (token, _)) in tokens.iter().enumerate().skip(start) {
        let text = match token {
            Token::Punctuation(punctuation) => *punctuation,
            Token::Identifier(name) if name == "if" => "if",
            _ => continue,
        };
        if depth == 0 && targets.contains(&text) {
            return Some(index);
        }
        match text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = depth.checked_sub(1)?,
            _ => {}
        }
    }
    None
}

/// Reports the literals in each arm pattern of a match body that starts at
/// `start`, skipping guards and arm bodies.
fn arm_patterns(tokens: &[(Token, usize)], start: usize, report: &mut impl FnMut(usize)) {
    let mut index = start;
    while index < tokens.len() && punctuation_at(tokens, index) != Some("}") {
        let Some(arrow) = top_level(tokens, index, &["=>"]) else {
            return;
        };
        let pattern_end = top_level(tokens, index, &["if"])
            .filter(|guard| *guard < arrow)
            .unwrap_or(arrow);
        (index..pattern_end).for_each(&mut *report);
        index = arrow + 1;
        if punctuation_at(tokens, index) == Some("{") {
            let Some(close) = top_level(tokens, index + 1, &["}"]) else {
                return;
            };
            index = close + 1;
            if punctuation_at(tokens, index) == Some(",") {
                index += 1;
            }
        } else {
            let Some(end) = top_level(tokens, index, &[",", "}"]) else {
                return;
            };
            index = if punctuation_at(tokens, end) == Some(",") {
                end + 1
            } else {
                end
            };
        }
    }
}

/// Drops each item that follows `#[cfg(test)]` or `#[cfg(all(test, ..))]`,
/// through the end of its braced body or its terminating semicolon.
fn remove_test_items(tokens: Vec<(Token, usize)>) -> Vec<(Token, usize)> {
    let mut kept = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        if is_test_attribute(&tokens, index) {
            let Some(close) = top_level(&tokens, index + 2, &["]"]) else {
                break;
            };
            // An attribute on a field or variant has no item end before the
            // enclosing bracket closes; the scan then resumes after it.
            index = match top_level(&tokens, close + 1, &["{", ";"]) {
                Some(end) if punctuation_at(&tokens, end) == Some("{") => {
                    top_level(&tokens, end + 1, &["}"]).map_or(close + 1, |body| body + 1)
                }
                Some(end) => end + 1,
                None => close + 1,
            };
            continue;
        }
        kept.push(tokens[index].clone());
        index += 1;
    }
    kept
}

fn is_test_attribute(tokens: &[(Token, usize)], index: usize) -> bool {
    let identifier = |offset: usize, expected: &str| {
        tokens.get(index + offset).map(|(token, _)| token)
            == Some(&Token::Identifier(expected.to_owned()))
    };
    punctuation_at(tokens, index) == Some("#")
        && punctuation_at(tokens, index + 1) == Some("[")
        && identifier(2, "cfg")
        && punctuation_at(tokens, index + 3) == Some("(")
        && (identifier(4, "test")
            || (identifier(4, "all")
                && punctuation_at(tokens, index + 5) == Some("(")
                && identifier(6, "test")))
}

const PUNCTUATION: &[&str] = &[
    "=>", "==", "!=", "::", "(", ")", "[", "]", "{", "}", ",", ";", ":", "|", "*", "&", ".", "!",
    "#", "=", "-", "<", ">",
];

/// Splits Rust source into identifiers, integer literals and the punctuation
/// the scan uses, each with its line, dropping comments, string and character
/// literals, and other characters.
fn tokenize(source: &str) -> Vec<(Token, usize)> {
    let characters: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut line = 1;
    let mut index = 0;
    let at = |index: usize| characters.get(index).copied().unwrap_or('\0');
    while index < characters.len() {
        let character = characters[index];
        if character == '\n' {
            line += 1;
            index += 1;
        } else if character == '/' && at(index + 1) == '/' {
            while index < characters.len() && characters[index] != '\n' {
                index += 1;
            }
        } else if character == '/' && at(index + 1) == '*' {
            let mut depth = 0;
            loop {
                if index >= characters.len() {
                    break;
                }
                if characters[index] == '/' && at(index + 1) == '*' {
                    depth += 1;
                    index += 2;
                } else if characters[index] == '*' && at(index + 1) == '/' {
                    depth -= 1;
                    index += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    line += usize::from(characters[index] == '\n');
                    index += 1;
                }
            }
        } else if character == '"' {
            index += 1;
            while index < characters.len() && characters[index] != '"' {
                if characters[index] == '\\' {
                    index += 1;
                }
                line += usize::from(at(index) == '\n');
                index += 1;
            }
            index += 1;
        } else if character == '\'' {
            if at(index + 1) == '\\' {
                index += 3;
                while index < characters.len() && characters[index] != '\'' {
                    index += 1;
                }
                index += 1;
            } else if at(index + 2) == '\'' {
                index += 3;
            } else {
                // A lifetime or label; its name follows as an identifier.
                index += 1;
            }
        } else if character.is_alphabetic() || character == '_' {
            let start = index;
            while at(index).is_alphanumeric() || at(index) == '_' {
                index += 1;
            }
            let name: String = characters[start..index].iter().collect();
            if matches!(name.as_str(), "r" | "br" | "cr") && matches!(at(index), '"' | '#') {
                let mut hashes = 0;
                while at(index + hashes) == '#' {
                    hashes += 1;
                }
                if at(index + hashes) == '"' {
                    // A raw string literal, which has no escapes.
                    index += hashes + 1;
                    let closing: Vec<char> = std::iter::once('"')
                        .chain(std::iter::repeat_n('#', hashes))
                        .collect();
                    while index < characters.len() && !characters[index..].starts_with(&closing) {
                        line += usize::from(characters[index] == '\n');
                        index += 1;
                    }
                    index += closing.len();
                } else {
                    // A raw identifier such as `r#type`; its name follows.
                    index += 1;
                }
                continue;
            }
            tokens.push((Token::Identifier(name), line));
        } else if character.is_ascii_digit() {
            let start = index;
            while at(index).is_ascii_alphanumeric() || at(index) == '_' {
                index += 1;
            }
            let literal: String = characters[start..index].iter().collect();
            if at(index) == '.' && at(index + 1).is_ascii_digit() {
                // A floating-point literal.
                index += 1;
                while at(index).is_ascii_alphanumeric() || at(index) == '_' {
                    index += 1;
                }
                continue;
            }
            let digits: String = literal
                .chars()
                .take_while(|character| character.is_ascii_digit() || *character == '_')
                .filter(char::is_ascii_digit)
                .collect();
            let radix_prefix = literal.len() > 1 && matches!(&literal[..2], "0x" | "0o" | "0b");
            if !radix_prefix {
                tokens.push((Token::Integer(digits), line));
            }
        } else if let Some(punctuation) = PUNCTUATION.iter().find(|punctuation| {
            characters[index..]
                .iter()
                .take(punctuation.len())
                .copied()
                .eq(punctuation.chars())
        }) {
            tokens.push((Token::Punctuation(punctuation), line));
            index += punctuation.len();
        } else {
            index += 1;
        }
    }
    tokens
}
