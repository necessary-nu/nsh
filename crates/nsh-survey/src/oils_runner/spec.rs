//! The Oils `.test.sh` file format: a parser and the lexer under it.
//!
//! A different program from the runner that uses it. This reads bytes and
//! answers with a structure -- a file's metadata, its cases, and the
//! assertions and qualifiers each case carries -- and knows nothing about
//! shells, containment, timeouts or reports. Everything that happens once
//! a case has been read is `oils_runner.rs`.
//!
//! Moved here unchanged. `oils_runner.rs` was fifty functions in one file
//! and nineteen of them were this.

use super::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub(super) struct ParsedFile {
    pub(super) metadata: FileMetadata,
    pub(super) cases: Vec<TestCase>,
}

#[derive(Debug, Default)]
pub(super) struct FileMetadata {
    values: BTreeMap<String, String>,
    pub(super) our_shell: Option<String>,
    pub(super) legacy_tmp_dir: bool,
}

#[derive(Debug)]
pub(super) struct TestCase {
    pub(super) description: String,
    pub(super) line: usize,
    pub(super) code: Vec<u8>,
    pub(super) ideal: Assertions,
    pub(super) per_shell: BTreeMap<String, QualifiedAssertions>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Assertions {
    pub(super) stdout: Vec<ExpectedBytes>,
    pub(super) stderr: Vec<ExpectedBytes>,
    pub(super) status: Option<i32>,
}

#[derive(Clone, Debug)]
pub(super) struct ExpectedBytes {
    pub(super) source: String,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct QualifiedAssertions {
    pub(super) qualifier: String,
    pub(super) assertions: Assertions,
}

#[derive(Debug)]
struct CaseBuilder {
    description: String,
    line: usize,
    code: Option<Vec<u8>>,
    ideal: Assertions,
    per_shell: BTreeMap<String, QualifiedAssertions>,
}

pub(super) fn parse_spec(path: &Path) -> Result<ParsedFile> {
    let bytes = fs::read(path)?;
    parse_spec_bytes(&bytes).map_err(|error| format!("{}: {error}", path.display()).into())
}

pub(super) fn parse_spec_bytes(bytes: &[u8]) -> Result<ParsedFile> {
    let mut tokens = Tokenizer::new(bytes)?;
    let mut metadata = FileMetadata::default();
    while let Token::Key(key) = tokens.peek().token.clone() {
        if key.qualifier.is_some() {
            return Err(format!("line {}: qualifier in file metadata", key.line).into());
        }
        let value = String::from_utf8(key.value)?;
        if metadata.values.insert(key.name.clone(), value).is_some() {
            return Err(format!("line {}: duplicate file metadata {}", key.line, key.name).into());
        }
        tokens.advance(LexMode::Outer)?;
    }
    const FILE_FIELDS: &[&str] = &[
        "our_shell",
        "compare_shells",
        "suite",
        "tags",
        "oils_failures_allowed",
        "oils_cpp_failures_allowed",
        "legacy_tmp_dir",
    ];
    if let Some(invalid) = metadata
        .values
        .keys()
        .find(|name| !FILE_FIELDS.contains(&name.as_str()))
    {
        return Err(format!("invalid file metadata {invalid:?}").into());
    }
    metadata.our_shell = metadata.values.get("our_shell").cloned();
    metadata.legacy_tmp_dir = metadata
        .values
        .get("legacy_tmp_dir")
        .is_some_and(|value| !value.is_empty());

    let mut cases = Vec::new();
    while !matches!(tokens.peek().token, Token::Eof) {
        cases.push(parse_case(&mut tokens)?);
    }
    Ok(ParsedFile { metadata, cases })
}

fn parse_case(tokens: &mut Tokenizer<'_>) -> Result<TestCase> {
    let (description, line) = match &tokens.peek().token {
        Token::CaseBegin(description) => (description.clone(), tokens.peek().line),
        token => {
            return Err(format!(
                "line {}: expected case heading, got {token:?}",
                tokens.peek().line
            )
            .into());
        }
    };
    tokens.advance(LexMode::Outer)?;
    let mut builder = CaseBuilder {
        description,
        line,
        code: None,
        ideal: Assertions::default(),
        per_shell: BTreeMap::new(),
    };
    parse_case_metadata(tokens, &mut builder)?;
    if builder.code.is_none() {
        let mut code = Vec::new();
        if !matches!(tokens.peek().token, Token::Plain(_)) {
            return Err(format!("line {}: expected case code", tokens.peek().line).into());
        }
        while let Token::Plain(line) = &tokens.peek().token {
            code.extend_from_slice(line);
            tokens.advance(LexMode::Raw)?;
        }
        builder.code = Some(code);
        parse_case_metadata(tokens, &mut builder)?;
    }
    Ok(TestCase {
        description: builder.description,
        line: builder.line,
        code: builder.code.expect("case code assigned"),
        ideal: builder.ideal,
        per_shell: builder.per_shell,
    })
}

fn parse_case_metadata(tokens: &mut Tokenizer<'_>, builder: &mut CaseBuilder) -> Result<()> {
    loop {
        match tokens.peek().token.clone() {
            Token::Key(key) => {
                apply_case_metadata(builder, key)?;
                tokens.advance(LexMode::Outer)?;
            }
            Token::Multiline(mut key) => {
                if !key.value.is_empty() {
                    return Err(format!(
                        "line {}: multiline {} value must start on the following line",
                        key.line, key.name
                    )
                    .into());
                }
                tokens.advance(LexMode::Raw)?;
                let mut value = Vec::new();
                while let Token::Plain(line) = &tokens.peek().token {
                    value.extend_from_slice(line);
                    tokens.advance(LexMode::Raw)?;
                }
                if matches!(tokens.peek().token, Token::End) {
                    tokens.advance(LexMode::Outer)?;
                }
                key.name.make_ascii_lowercase();
                key.value = value;
                apply_case_metadata(builder, key)?;
            }
            _ => return Ok(()),
        }
    }
}

fn apply_case_metadata(builder: &mut CaseBuilder, key: KeyValue) -> Result<()> {
    if key.name == "code" {
        if key.qualifier.is_some() {
            return Err(format!("line {}: code cannot be shell-qualified", key.line).into());
        }
        if builder.code.replace(key.value).is_some() {
            return Err(format!("line {}: duplicate code", key.line).into());
        }
        return Ok(());
    }
    if let Some(qualifier) = key.qualifier {
        for shell in key.shells {
            let qualified =
                builder
                    .per_shell
                    .entry(shell.clone())
                    .or_insert_with(|| QualifiedAssertions {
                        qualifier: qualifier.clone(),
                        assertions: Assertions::default(),
                    });
            if qualified.qualifier != qualifier {
                return Err(format!(
                    "line {}: inconsistent qualifier for {shell}: {} versus {qualifier}",
                    key.line, qualified.qualifier
                )
                .into());
            }
            set_assertion(
                &mut qualified.assertions,
                &key.name,
                &key.value,
                key.line,
                true,
            )?;
        }
    } else {
        set_assertion(&mut builder.ideal, &key.name, &key.value, key.line, false)?;
    }
    Ok(())
}

fn set_assertion(
    set: &mut Assertions,
    name: &str,
    value: &[u8],
    line: usize,
    reject_duplicate_base: bool,
) -> Result<()> {
    match name {
        "stdout" => set_bytes(
            &mut set.stdout,
            name,
            value.to_vec(),
            line,
            reject_duplicate_base,
        ),
        "stderr" => set_bytes(
            &mut set.stderr,
            name,
            value.to_vec(),
            line,
            reject_duplicate_base,
        ),
        "stdout-json" => {
            let decoded: String = serde_json::from_str(std::str::from_utf8(value)?)?;
            set_bytes(
                &mut set.stdout,
                name,
                decoded.into_bytes(),
                line,
                reject_duplicate_base,
            )
        }
        "stderr-json" => {
            let decoded: String = serde_json::from_str(std::str::from_utf8(value)?)?;
            set_bytes(
                &mut set.stderr,
                name,
                decoded.into_bytes(),
                line,
                reject_duplicate_base,
            )
        }
        "status" => {
            if reject_duplicate_base && set.status.is_some() {
                return Err(format!("line {line}: duplicate status assertion").into());
            }
            set.status = Some(std::str::from_utf8(value)?.trim().parse()?);
            Ok(())
        }
        // A small number of upstream files spell the optional multiline
        // terminator as `## END:`. Oils tokenizes that as inert case metadata.
        "END" => Ok(()),
        _ => Err(format!("line {line}: unsupported case metadata {name:?}").into()),
    }
}

fn set_bytes(
    slot: &mut Vec<ExpectedBytes>,
    source: &str,
    value: Vec<u8>,
    line: usize,
    reject_duplicate_base: bool,
) -> Result<()> {
    if reject_duplicate_base && !slot.is_empty() {
        let base = source.strip_suffix("-json").unwrap_or(source);
        return Err(format!("line {line}: duplicate {base} assertion").into());
    }
    if let Some(existing) = slot.iter_mut().find(|expected| expected.source == source) {
        existing.bytes = value;
    } else {
        slot.push(ExpectedBytes {
            source: source.to_owned(),
            bytes: value,
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum LexMode {
    Outer,
    Raw,
}

#[derive(Clone, Debug)]
struct SpannedToken {
    line: usize,
    token: Token,
}

#[derive(Clone, Debug)]
enum Token {
    CaseBegin(String),
    Key(KeyValue),
    Multiline(KeyValue),
    End,
    Plain(Vec<u8>),
    Eof,
}

#[derive(Clone, Debug)]
struct KeyValue {
    line: usize,
    qualifier: Option<String>,
    shells: Vec<String>,
    name: String,
    value: Vec<u8>,
}

struct Tokenizer<'a> {
    lines: Vec<&'a [u8]>,
    next: usize,
    cursor: SpannedToken,
}

impl<'a> Tokenizer<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let mut tokenizer = Self {
            lines: bytes.split_inclusive(|byte| *byte == b'\n').collect(),
            next: 0,
            cursor: SpannedToken {
                line: 0,
                token: Token::Eof,
            },
        };
        tokenizer.advance(LexMode::Outer)?;
        Ok(tokenizer)
    }

    fn peek(&self) -> &SpannedToken {
        &self.cursor
    }

    fn advance(&mut self, mode: LexMode) -> Result<()> {
        loop {
            if self.next == self.lines.len() {
                self.cursor = SpannedToken {
                    line: self.next + 1,
                    token: Token::Eof,
                };
                return Ok(());
            }
            let line_number = self.next + 1;
            let line = self.lines[self.next];
            self.next += 1;
            if let Some(token) = classify_line(line, line_number, mode)? {
                self.cursor = SpannedToken {
                    line: line_number,
                    token,
                };
                return Ok(());
            }
        }
    }
}

fn classify_line(line: &[u8], line_number: usize, mode: LexMode) -> Result<Option<Token>> {
    if matches!(mode, LexMode::Outer) && trim_ascii(line).is_empty() {
        return Ok(None);
    }
    if let Some(rest) = line.strip_prefix(b"####") {
        return Ok(Some(Token::CaseBegin(String::from_utf8(
            trim_ascii(rest).to_vec(),
        )?)));
    }
    if let Some(key) = parse_key_value(line, line_number)? {
        return Ok(Some(if matches!(key.name.as_str(), "STDOUT" | "STDERR") {
            Token::Multiline(key)
        } else {
            Token::Key(key)
        }));
    }
    if is_end_marker(line) {
        return Ok(Some(Token::End));
    }
    if line.starts_with(b"##") {
        return Err(format!("line {line_number}: invalid ## metadata line").into());
    }
    if trim_start_ascii(line).starts_with(b"#") {
        return Ok(None);
    }
    Ok(Some(Token::Plain(line.to_vec())))
}

fn parse_key_value(line: &[u8], line_number: usize) -> Result<Option<KeyValue>> {
    let Some(after_hashes) = line.strip_prefix(b"##") else {
        return Ok(None);
    };
    if !after_hashes.first().is_some_and(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let content = strip_line_ending(trim_start_ascii(after_hashes));
    let Some(colon) = content.iter().position(|byte| *byte == b':') else {
        return Ok(None);
    };
    let words: Vec<&[u8]> = content[..colon]
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|word| !word.is_empty())
        .collect();
    let (qualifier, shells, name) = match words.as_slice() {
        [name] if valid_key(name) => (None, Vec::new(), *name),
        [qualifier, shells, name]
            if valid_qualifier(qualifier) && valid_shells(shells) && valid_key(name) =>
        {
            (
                Some(String::from_utf8(qualifier.to_vec())?),
                String::from_utf8(shells.to_vec())?
                    .split('/')
                    .map(str::to_owned)
                    .collect(),
                *name,
            )
        }
        _ => return Ok(None),
    };
    let mut value = trim_start_ascii(&content[colon + 1..]).to_vec();
    let name = String::from_utf8(name.to_vec())?;
    if matches!(name.as_str(), "stdout" | "stderr") {
        value.push(b'\n');
    }
    Ok(Some(KeyValue {
        line: line_number,
        qualifier,
        shells,
        name,
        value,
    }))
}

fn valid_key(value: &[u8]) -> bool {
    !value.is_empty()
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn valid_shells(value: &[u8]) -> bool {
    !value.is_empty()
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'/'))
        && !value.starts_with(b"/")
        && !value.ends_with(b"/")
        && !value.windows(2).any(|window| window == b"//")
}

fn valid_qualifier(value: &[u8]) -> bool {
    value == b"OK"
        || value == b"BUG"
        || value == b"N-I"
        || value
            .strip_prefix(b"OK-")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit))
        || value
            .strip_prefix(b"BUG-")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.iter().all(u8::is_ascii_digit))
}

fn is_end_marker(line: &[u8]) -> bool {
    line.strip_prefix(b"##")
        .filter(|rest| rest.first().is_some_and(u8::is_ascii_whitespace))
        .map(trim_start_ascii)
        .is_some_and(|rest| rest.starts_with(b"END"))
}

fn strip_line_ending(mut line: &[u8]) -> &[u8] {
    if let Some(without) = line.strip_suffix(b"\n") {
        line = without;
    }
    if let Some(without) = line.strip_suffix(b"\r") {
        line = without;
    }
    line
}

fn trim_start_ascii(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    &value[start..]
}

fn trim_ascii(value: &[u8]) -> &[u8] {
    let value = trim_start_ascii(value);
    let end = value
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(0, |index| index + 1);
    &value[..end]
}
