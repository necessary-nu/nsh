//! Structural regression checks for Rust-native core naming.

use std::path::{Path, PathBuf};

fn rust_sources(directory: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("source directory is readable") {
        let path = entry.expect("source entry is readable").path();
        if path.is_dir() {
            rust_sources(&path, paths);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
}

fn quoted_end(bytes: &[u8], mut cursor: usize, quote: u8) -> usize {
    cursor += 1;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' => cursor = (cursor + 2).min(bytes.len()),
            byte if byte == quote => return cursor + 1,
            _ => cursor += 1,
        }
    }
    bytes.len()
}

fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut opening = start;
    if matches!(bytes.get(opening), Some(b'b' | b'c')) {
        opening += 1;
    }
    if bytes.get(opening) != Some(&b'r') {
        return None;
    }
    opening += 1;
    let hashes = bytes[opening..]
        .iter()
        .take_while(|&&byte| byte == b'#')
        .count();
    opening += hashes;
    if bytes.get(opening) != Some(&b'"') {
        return None;
    }

    let mut cursor = opening + 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'"'
            && bytes.get(cursor + 1..cursor + 1 + hashes) == Some(&bytes[opening - hashes..opening])
        {
            return Some(cursor + 1 + hashes);
        }
        cursor += 1;
    }
    Some(bytes.len())
}

fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 1usize;
    let mut cursor = start + 2;
    while cursor + 1 < bytes.len() {
        match (bytes[cursor], bytes[cursor + 1]) {
            (b'/', b'*') => {
                depth += 1;
                cursor += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                cursor += 2;
                if depth == 0 {
                    return cursor;
                }
            }
            _ => cursor += 1,
        }
    }
    bytes.len()
}

fn identifiers(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut identifiers = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let skipped = if bytes.get(cursor..cursor + 2) == Some(b"//") {
            bytes[cursor..]
                .iter()
                .position(|&byte| byte == b'\n')
                .map_or(bytes.len(), |offset| cursor + offset + 1)
        } else if bytes.get(cursor..cursor + 2) == Some(b"/*") {
            block_comment_end(bytes, cursor)
        } else if let Some(end) = raw_string_end(bytes, cursor) {
            end
        } else if bytes[cursor] == b'"' {
            quoted_end(bytes, cursor, b'"')
        } else if matches!(bytes.get(cursor..cursor + 2), Some(b"b\"") | Some(b"c\"")) {
            quoted_end(bytes, cursor + 1, b'"')
        } else if bytes.get(cursor..cursor + 2) == Some(b"b'") {
            quoted_end(bytes, cursor + 1, b'\'')
        } else {
            cursor
        };
        if skipped != cursor {
            cursor = skipped;
            continue;
        }

        if bytes[cursor] == b'_' || bytes[cursor].is_ascii_alphabetic() {
            let start = cursor;
            cursor += 1;
            while cursor < bytes.len()
                && (bytes[cursor] == b'_' || bytes[cursor].is_ascii_alphanumeric())
            {
                cursor += 1;
            }
            identifiers.push(source[start..cursor].to_owned());
        } else {
            cursor += 1;
        }
    }
    identifiers
}

// [spec:nsh:req:idiom.rust-naming/test]
#[test]
fn core_symbols_are_rust_named() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    rust_sources(&source_root, &mut paths);
    paths.sort();

    let forbidden = [
        "INTOFF",
        "INTON",
        "FORCEINTON",
        "USTPUTC",
        "STADJUST",
        "arglist",
        "backcmd",
        "builtincmd",
        "evalbackcmd",
        "evalcommand",
        "evalpipe",
        "evalstring",
        "evalsubshell",
        "evaltree",
        "forkchild",
        "forkparent",
        "forkshell",
        "ifsregion",
        "ifsstate",
        "localvar",
        "makejob",
        "mystring",
        "nextopt",
        "parsefile",
        "redir",
        "restartjob",
        "shellexec",
        "shellparam",
        "showjob",
        "showjobs",
        "siginbox",
        "signames",
        "strpush",
        "synstack",
        "testutil",
        "yylex",
    ];

    for path in paths {
        let source = std::fs::read_to_string(&path).expect("Rust source is UTF-8");
        let tokens = identifiers(&source);
        for name in forbidden {
            assert!(
                !tokens.iter().any(|token| token == name),
                "{} retains C-shaped identifier {name}",
                path.display()
            );
        }
        for pair in tokens.windows(2) {
            if pair[0] == "fn" {
                assert!(
                    !pair[1].ends_with("cmd"),
                    "{} retains *cmd function {}",
                    path.display(),
                    pair[1]
                );
            }
        }
    }

    for old_module in [
        "eval.rs",
        "exec.rs",
        "fd.rs",
        "redir.rs",
        "siginbox.rs",
        "signames.rs",
        "testutil.rs",
        "var.rs",
    ] {
        assert!(
            !source_root.join(old_module).exists(),
            "old module {old_module} exists"
        );
    }
}
