//! The `test` / `[` builtin.
//! Rules: `docs/spec/port/src/bltin/test.md`.
//!
//! The original parser kept its cursor and last operator in two mutable
//! globals and walked a synthetic `char **`. Here one parser borrows the
//! builtin's words. Operators and operands are typed values, and filesystem
//! questions cross the safe `nsh-platform` boundary.

use bstr::BStr;
use nsh_platform::{AccessMode, FileKind, FileMetadata, ShellBytesExt as _};
use std::cmp::Ordering;

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::evaluation::Flow;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Token {
    End,
    FileReadable,
    FileWritable,
    FileExecutable,
    FileExists,
    FileRegular,
    FileDirectory,
    FileCharDevice,
    FileBlockDevice,
    FileFifo,
    FileSocket,
    FileSymlink,
    FileNonempty,
    FileTerminal,
    FileSetUid,
    FileSetGid,
    FileSticky,
    FileNewer,
    FileOlder,
    FileSame,
    FileOwnedByUser,
    FileOwnedByGroup,
    StringEmpty,
    StringNonempty,
    VariableSet,
    ReferenceSet,
    StringEqual,
    StringNotEqual,
    StringLess,
    StringGreater,
    IntegerEqual,
    IntegerNotEqual,
    IntegerGreaterEqual,
    IntegerGreater,
    IntegerLessEqual,
    IntegerLess,
    Not,
    And,
    Or,
    LeftParen,
    RightParen,
    Operand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OperatorKind {
    Unary,
    Binary,
    BooleanUnary,
    BooleanBinary,
    Parenthesis,
}

#[derive(Clone, Copy)]
struct Operator {
    text: &'static [u8],
    token: Token,
    kind: OperatorKind,
}

const fn op(text: &'static [u8], token: Token, kind: OperatorKind) -> Operator {
    Operator { text, token, kind }
}

static OPERATORS: [Operator; 39] = [
    op(b"-r", Token::FileReadable, OperatorKind::Unary),
    op(b"-w", Token::FileWritable, OperatorKind::Unary),
    op(b"-x", Token::FileExecutable, OperatorKind::Unary),
    op(b"-e", Token::FileExists, OperatorKind::Unary),
    op(b"-f", Token::FileRegular, OperatorKind::Unary),
    op(b"-d", Token::FileDirectory, OperatorKind::Unary),
    op(b"-c", Token::FileCharDevice, OperatorKind::Unary),
    op(b"-b", Token::FileBlockDevice, OperatorKind::Unary),
    op(b"-p", Token::FileFifo, OperatorKind::Unary),
    op(b"-u", Token::FileSetUid, OperatorKind::Unary),
    op(b"-g", Token::FileSetGid, OperatorKind::Unary),
    op(b"-k", Token::FileSticky, OperatorKind::Unary),
    op(b"-s", Token::FileNonempty, OperatorKind::Unary),
    op(b"-t", Token::FileTerminal, OperatorKind::Unary),
    op(b"-z", Token::StringEmpty, OperatorKind::Unary),
    op(b"-n", Token::StringNonempty, OperatorKind::Unary),
    op(b"-h", Token::FileSymlink, OperatorKind::Unary),
    op(b"-O", Token::FileOwnedByUser, OperatorKind::Unary),
    op(b"-G", Token::FileOwnedByGroup, OperatorKind::Unary),
    op(b"-L", Token::FileSymlink, OperatorKind::Unary),
    op(b"-S", Token::FileSocket, OperatorKind::Unary),
    op(b"=", Token::StringEqual, OperatorKind::Binary),
    op(b"!=", Token::StringNotEqual, OperatorKind::Binary),
    op(b"<", Token::StringLess, OperatorKind::Binary),
    op(b">", Token::StringGreater, OperatorKind::Binary),
    op(b"-eq", Token::IntegerEqual, OperatorKind::Binary),
    op(b"-ne", Token::IntegerNotEqual, OperatorKind::Binary),
    op(b"-ge", Token::IntegerGreaterEqual, OperatorKind::Binary),
    op(b"-gt", Token::IntegerGreater, OperatorKind::Binary),
    op(b"-le", Token::IntegerLessEqual, OperatorKind::Binary),
    op(b"-lt", Token::IntegerLess, OperatorKind::Binary),
    op(b"-nt", Token::FileNewer, OperatorKind::Binary),
    op(b"-ot", Token::FileOlder, OperatorKind::Binary),
    op(b"-ef", Token::FileSame, OperatorKind::Binary),
    op(b"!", Token::Not, OperatorKind::BooleanUnary),
    op(b"-a", Token::And, OperatorKind::BooleanBinary),
    op(b"-o", Token::Or, OperatorKind::BooleanBinary),
    op(b"(", Token::LeftParen, OperatorKind::Parenthesis),
    op(b")", Token::RightParen, OperatorKind::Parenthesis),
];

/// The unary operators Bash adds, which POSIX's `test` does not have.
///
/// They are a separate table rather than rows in [`OPERATORS`] because
/// a POSIX shell must still call `-v` an operand: `test -v` there is the
/// one-argument form asking whether the string `-v` is non-empty, and
/// admitting the operator would change that answer.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
static BASH_OPERATORS: [Operator; 2] = [
    op(b"-v", Token::VariableSet, OperatorKind::Unary),
    op(b"-R", Token::ReferenceSet, OperatorKind::Unary),
];

fn operator(word: &BStr, bash: bool) -> Option<&'static Operator> {
    if bash
        && let Some(found) = BASH_OPERATORS
            .iter()
            .find(|candidate| candidate.text == word)
    {
        return Some(found);
    }
    OPERATORS.iter().find(|candidate| candidate.text == word)
}

// [spec:dash:sem:test.faccessat-confused-about-superuser-fn]
// The disabled configure-time workaround has no runtime representation.

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

// [spec:dash:sem:test.getn-fn]
fn parse_operand_integer(shell: &mut Shell, word: &BStr) -> Result<i64, Error> {
    let bytes: &[u8] = word.as_ref();
    let mut start = 0;
    while bytes.get(start).is_some_and(|byte| is_c_space(*byte)) {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && is_c_space(bytes[end - 1]) {
        end -= 1;
    }

    let (negative, digits) = match bytes.get(start) {
        Some(b'-') => (true, start + 1),
        Some(b'+') => (false, start + 1),
        _ => (false, start),
    };
    if digits == end || !bytes[digits..end].iter().all(u8::is_ascii_digit) {
        let mut message = b"Illegal number: ".to_vec();
        message.extend_from_slice(bytes);
        return Err(shell.diagnostics().shell_error(&message));
    }

    let limit = if negative {
        (i64::MAX as u64) + 1
    } else {
        i64::MAX as u64
    };
    let mut magnitude = 0u64;
    for &digit in &bytes[digits..end] {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add((digit - b'0') as u64)
            .min(limit);
    }
    Ok(if negative {
        if magnitude == limit {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    })
}

struct TestParser<'a> {
    words: &'a [&'a BStr],
    pos: usize,
    last_operator: Option<&'static Operator>,
    /// Whether the Bash-only operator table is in scope for this scan.
    bash: bool,
}

impl<'a> TestParser<'a> {
    fn new(words: &'a [&'a BStr], bash: bool) -> Self {
        Self {
            words,
            pos: 0,
            last_operator: None,
            bash,
        }
    }

    fn word(&self, pos: usize) -> Option<&'a BStr> {
        self.words.get(pos).copied()
    }

    // [spec:dash:sem:test.getop-fn]
    fn operator_at(&self, pos: usize) -> Option<&'static Operator> {
        self.word(pos).and_then(|word| operator(word, self.bash))
    }

    // [spec:dash:sem:test.t-lex-fn]
    fn lex(&mut self, pos: usize) -> Token {
        let Some(word) = self.word(pos) else {
            self.last_operator = None;
            return Token::End;
        };
        let Some(candidate) = operator(word, self.bash) else {
            self.last_operator = None;
            return Token::Operand;
        };

        if (candidate.kind == OperatorKind::Unary && self.is_operand(pos))
            || (candidate.token == Token::LeftParen && self.word(pos + 1).is_none())
        {
            self.last_operator = None;
            Token::Operand
        } else {
            self.last_operator = Some(candidate);
            candidate.token
        }
    }

    // [spec:dash:sem:test.isoperand-fn]
    fn is_operand(&self, pos: usize) -> bool {
        if self.word(pos + 1).is_none() {
            return true;
        }
        if self.word(pos + 2).is_none() {
            return false;
        }
        self.operator_at(pos + 1)
            .is_some_and(|op| op.kind == OperatorKind::Binary)
    }

    // [spec:dash:sem:test.oexpr-fn]
    fn or_expr(&mut self, shell: &mut Shell, mut token: Token) -> Result<bool, Error> {
        let mut result = false;
        loop {
            result |= self.and_expr(shell, token)?;
            if self.word(self.pos).is_none() {
                break;
            }
            token = self.lex(self.pos + 1);
            if token != Token::Or {
                break;
            }
            self.pos += 2;
            token = self.lex(self.pos);
        }
        Ok(result)
    }

    // [spec:dash:sem:test.aexpr-fn]
    fn and_expr(&mut self, shell: &mut Shell, mut token: Token) -> Result<bool, Error> {
        let mut result = true;
        loop {
            if !self.not_expr(shell, token)? {
                result = false;
            }
            if self.word(self.pos).is_none() {
                break;
            }
            token = self.lex(self.pos + 1);
            if token != Token::And {
                break;
            }
            self.pos += 2;
            token = self.lex(self.pos);
        }
        Ok(result)
    }

    // [spec:dash:sem:test.nexpr-fn]
    fn not_expr(&mut self, shell: &mut Shell, mut token: Token) -> Result<bool, Error> {
        if token != Token::Not {
            return self.primary(shell, token);
        }
        token = self.lex(self.pos + 1);
        if token != Token::End {
            self.pos += 1;
        }
        Ok(!self.not_expr(shell, token)?)
    }

    // [spec:dash:sem:test.primary-fn]
    fn primary(&mut self, shell: &mut Shell, token: Token) -> Result<bool, Error> {
        if token == Token::End {
            return Ok(false);
        }
        if token == Token::LeftParen {
            self.pos += 1;
            let nested = self.lex(self.pos);
            if nested == Token::RightParen {
                return Ok(false);
            }
            let result = self.or_expr(shell, nested)?;
            self.pos += 1;
            if self.lex(self.pos) != Token::RightParen {
                return Err(syntax(shell, None, b"closing paren expected"));
            }
            return Ok(result);
        }

        if let Some(op) = self
            .last_operator
            .filter(|op| op.kind == OperatorKind::Unary)
        {
            self.pos += 1;
            let Some(operand) = self.word(self.pos) else {
                return Err(syntax(shell, Some(op.text), b"argument expected"));
            };
            return unary(shell, token, operand);
        }

        self.lex(self.pos + 1);
        if self
            .last_operator
            .is_some_and(|op| op.kind == OperatorKind::Binary)
        {
            return self.binary(shell);
        }

        Ok(self.word(self.pos).is_some_and(|word| !word.is_empty()))
    }

    // [spec:dash:sem:test.binop-fn]
    fn binary(&mut self, shell: &mut Shell) -> Result<bool, Error> {
        let left = self
            .word(self.pos)
            .expect("binary operator has a left operand");
        self.pos += 1;
        self.lex(self.pos);
        let op = self.last_operator.expect("binary token names an operator");
        self.pos += 1;
        let Some(right) = self.word(self.pos) else {
            return Err(syntax(shell, Some(op.text), b"argument expected"));
        };

        Ok(match op.token {
            Token::StringNotEqual => left != right,
            Token::StringLess => shell.locale.collate(left, right) == Ordering::Less,
            Token::StringGreater => shell.locale.collate(left, right) == Ordering::Greater,
            Token::IntegerEqual => {
                parse_operand_integer(shell, left)? == parse_operand_integer(shell, right)?
            }
            Token::IntegerNotEqual => {
                parse_operand_integer(shell, left)? != parse_operand_integer(shell, right)?
            }
            Token::IntegerGreaterEqual => {
                parse_operand_integer(shell, left)? >= parse_operand_integer(shell, right)?
            }
            Token::IntegerGreater => {
                parse_operand_integer(shell, left)? > parse_operand_integer(shell, right)?
            }
            Token::IntegerLessEqual => {
                parse_operand_integer(shell, left)? <= parse_operand_integer(shell, right)?
            }
            Token::IntegerLess => {
                parse_operand_integer(shell, left)? < parse_operand_integer(shell, right)?
            }
            Token::FileNewer => newer(left, right),
            Token::FileOlder => older(left, right),
            Token::FileSame => same_file(left, right),
            _ => left == right,
        })
    }
}

// [spec:nsh:def:idiom.logical-descriptors]
fn unary(shell: &mut Shell, token: Token, operand: &BStr) -> Result<bool, Error> {
    Ok(match token {
        Token::StringEmpty => operand.is_empty(),
        Token::StringNonempty => !operand.is_empty(),
        Token::VariableSet => crate::variables::special::is_assigned(shell, operand),
        Token::ReferenceSet => crate::variables::nameref::is_reference(shell, operand),
        Token::FileTerminal => {
            let descriptor = parse_operand_integer(shell, operand)? as i32;
            LogicalDescriptor::new(descriptor)
                .and_then(|descriptor| shell.descriptors.get(descriptor))
                .as_ref()
                .is_some_and(nsh_platform::is_terminal)
        }
        Token::FileReadable => test_file_access(operand, AccessMode::READ_OK),
        Token::FileWritable => test_file_access(operand, AccessMode::WRITE_OK),
        Token::FileExecutable => test_file_access(operand, AccessMode::EXEC_OK),
        _ => file_stat(operand, token),
    })
}

/// Evaluate one `test` unary operator named by its spelling.
///
/// `[[ ]]` asks the same filesystem and string questions, so it borrows
/// the answers rather than restating them. `None` means the spelling is
/// not a unary operator here.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(crate) fn unary_test(
    shell: &mut Shell,
    name: &BStr,
    operand: &BStr,
) -> Option<Result<bool, Error>> {
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let found = operator(name, bash).filter(|found| found.kind == OperatorKind::Unary)?;
    Some(unary(shell, found.token, operand))
}

/// Whether a path exists, for the `-a` and `-e` spellings.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(crate) fn file_exists(path: &BStr) -> bool {
    file_stat(path, Token::FileExists)
}

/// The `-nt`, `-ot` and `-ef` file comparisons by spelling.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(crate) fn file_comparison(name: &BStr, left: &BStr, right: &BStr) -> Option<bool> {
    let name: &[u8] = name.as_ref();
    match name {
        b"-nt" => Some(newer(left, right)),
        b"-ot" => Some(older(left, right)),
        b"-ef" => Some(same_file(left, right)),
        _ => None,
    }
}

// [spec:dash:sem:test.testcmd-fn]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let Some(command) = args.first() else {
        return Ok(Flow::Done((1).into()));
    };
    let mut expression = &args[1..];
    if *command == b"[".as_slice() {
        if expression
            .last()
            .is_none_or(|word| word.first() != Some(&b']'))
        {
            return Err(shell.diagnostics().shell_error(b"missing ]"));
        }
        expression = &expression[..expression.len() - 1];
    }

    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let mut result = 1;
    loop {
        if expression.is_empty() {
            return Ok(Flow::Done((result).into()));
        }

        let forced_operand = expression.len() == 3
            && operator(expression[1], bash).is_some_and(|op| op.kind == OperatorKind::Binary);
        if !forced_operand && matches!(expression.len(), 3 | 4) {
            if expression.first() == Some(&BStr::new(b"("))
                && expression.last() == Some(&BStr::new(b")"))
            {
                expression = &expression[1..expression.len() - 1];
            } else if expression.first() == Some(&BStr::new(b"!")) {
                result = 0;
                expression = &expression[1..];
                continue;
            }
        }

        let mut parser = TestParser::new(expression, bash);
        let first = if forced_operand {
            Token::Operand
        } else {
            parser.lex(0)
        };
        let value = parser.or_expr(shell, first)?;
        if parser.word(parser.pos).is_some() && parser.word(parser.pos + 1).is_some() {
            let unexpected = parser.word(parser.pos).unwrap();
            return Err(syntax(shell, Some(unexpected), b"unexpected operator"));
        }
        return Ok(Flow::Done((result ^ i32::from(value)).into()));
    }
}

// [spec:dash:sem:test.syntax-fn]
fn syntax(shell: &mut Shell, op: Option<&[u8]>, message: &[u8]) -> Error {
    let mut text = Vec::new();
    if let Some(op) = op.filter(|op| !op.is_empty()) {
        text.extend_from_slice(op);
        text.extend_from_slice(b": ");
    }
    text.extend_from_slice(message);
    shell.diagnostics().shell_error(&text)
}

// [spec:dash:sem:test.filstat-fn]
fn file_stat(word: &BStr, token: Token) -> bool {
    let Ok(path) = word.try_to_path_buf() else {
        return false;
    };
    let Ok(metadata) = nsh_platform::path_metadata(&path, token != Token::FileSymlink) else {
        return false;
    };
    match token {
        Token::FileExists => true,
        Token::FileRegular => metadata.kind == FileKind::Regular,
        Token::FileDirectory => metadata.kind == FileKind::Directory,
        Token::FileCharDevice => metadata.kind == FileKind::CharacterDevice,
        Token::FileBlockDevice => metadata.kind == FileKind::BlockDevice,
        Token::FileFifo => metadata.kind == FileKind::Fifo,
        Token::FileSocket => metadata.kind == FileKind::Socket,
        Token::FileSymlink => metadata.kind == FileKind::Symlink,
        Token::FileSetUid => metadata.mode & 0o4000 != 0,
        Token::FileSetGid => metadata.mode & 0o2000 != 0,
        Token::FileSticky => metadata.mode & 0o1000 != 0,
        Token::FileNonempty => metadata.size != 0,
        Token::FileOwnedByUser => metadata.user == nsh_platform::effective_uid().as_raw(),
        Token::FileOwnedByGroup => metadata.group == nsh_platform::effective_gid().as_raw(),
        _ => true,
    }
}

fn modified(metadata: &FileMetadata) -> (i64, i64) {
    (metadata.modified_seconds, metadata.modified_nanoseconds)
}

// [spec:dash:sem:test.newerf-fn]
fn newer(left: &BStr, right: &BStr) -> bool {
    let Ok(left) = left
        .try_to_path_buf()
        .and_then(|path| nsh_platform::path_metadata(&path, true))
    else {
        return false;
    };
    let Ok(right) = right
        .try_to_path_buf()
        .and_then(|path| nsh_platform::path_metadata(&path, true))
    else {
        return true;
    };
    modified(&left) > modified(&right)
}

// [spec:dash:sem:test.olderf-fn]
fn older(left: &BStr, right: &BStr) -> bool {
    let Ok(right) = right
        .try_to_path_buf()
        .and_then(|path| nsh_platform::path_metadata(&path, true))
    else {
        return false;
    };
    let Ok(left) = left
        .try_to_path_buf()
        .and_then(|path| nsh_platform::path_metadata(&path, true))
    else {
        return true;
    };
    modified(&left) < modified(&right)
}

// [spec:dash:sem:test.equalf-fn]
fn same_file(left: &BStr, right: &BStr) -> bool {
    let (Ok(left), Ok(right)) = (left.try_to_path_buf(), right.try_to_path_buf()) else {
        return false;
    };
    nsh_platform::path_is_same_file(&left, &right)
}

// [spec:dash:sem:test.test-file-access-fn]
// [spec:dash:sem:exec.test-file-access-fn]
// [spec:dash:sem:test.test-access-fn]
// [spec:dash:sem:exec.test-access-fn]
// [spec:dash:sem:test.has-exec-bit-set-fn]
pub fn test_file_access(path: &BStr, access: AccessMode) -> bool {
    path.try_to_path_buf()
        .is_ok_and(|path| nsh_platform::effective_access(&path, access))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(words: &[&[u8]]) -> i32 {
        let args: Vec<&BStr> = words.iter().map(|word| BStr::new(*word)).collect();
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
        let Flow::Done(status) = run(shell, &args).unwrap() else {
            unreachable!("`test` always finishes")
        };
        i32::from(status.code())
    }

    #[test]
    fn no_expression_is_false() {
        assert_eq!(eval(&[b"test"]), 1);
        assert_eq!(eval(&[b"[", b"]"]), 1);
    }

    #[test]
    fn one_word_tests_for_emptiness() {
        assert_eq!(eval(&[b"test", b"x"]), 0);
        assert_eq!(eval(&[b"test", b""]), 1);
        assert_eq!(eval(&[b"test", b"-n"]), 0);
        assert_eq!(eval(&[b"test", b"="]), 0);
    }

    #[test]
    fn unary_string_operators() {
        assert_eq!(eval(&[b"test", b"-n", b"x"]), 0);
        assert_eq!(eval(&[b"test", b"-n", b""]), 1);
        assert_eq!(eval(&[b"test", b"-z", b""]), 0);
        assert_eq!(eval(&[b"test", b"-z", b"x"]), 1);
    }

    #[test]
    fn binary_string_operators() {
        assert_eq!(eval(&[b"test", b"a", b"=", b"a"]), 0);
        assert_eq!(eval(&[b"test", b"a", b"=", b"b"]), 1);
        assert_eq!(eval(&[b"test", b"a", b"!=", b"b"]), 0);
    }

    #[test]
    fn a_middle_operator_wins() {
        assert_eq!(eval(&[b"test", b"!", b"=", b"!"]), 0);
        assert_eq!(eval(&[b"test", b"!", b"=", b"x"]), 1);
    }

    #[test]
    fn integer_comparison() {
        assert_eq!(eval(&[b"test", b"1", b"-eq", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"1", b"-ne", b"1"]), 1);
        assert_eq!(eval(&[b"test", b"1", b"-lt", b"2"]), 0);
        assert_eq!(eval(&[b"test", b"2", b"-gt", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"-1", b"-lt", b"0"]), 0);
    }

    #[test]
    fn the_remaining_integer_operators() {
        assert_eq!(eval(&[b"test", b"1", b"-le", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"2", b"-le", b"1"]), 1);
        assert_eq!(eval(&[b"test", b"1", b"-ge", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"1", b"-ge", b"2"]), 1);
    }

    #[test]
    fn permission_predicates() {
        assert_eq!(eval(&[b"test", b"-r", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-x", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-s", b"/nonexistent-for-nsh-tests"]), 1);
    }

    #[test]
    fn negation_and_connectives() {
        assert_eq!(eval(&[b"test", b"!", b"-n", b""]), 0);
        assert_eq!(eval(&[b"test", b"x", b"-a", b"y"]), 0);
        assert_eq!(eval(&[b"test", b"x", b"-a", b""]), 1);
        assert_eq!(eval(&[b"test", b"", b"-o", b"y"]), 0);
        assert_eq!(eval(&[b"test", b"", b"-o", b""]), 1);
    }

    #[test]
    fn parentheses_group() {
        assert_eq!(eval(&[b"test", b"(", b"x", b")"]), 0);
        assert_eq!(eval(&[b"test", b"(", b"", b")"]), 1);
    }

    #[test]
    fn file_operators_read_the_filesystem() {
        assert_eq!(eval(&[b"test", b"-e", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-d", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-f", b"/"]), 1);
        assert_eq!(eval(&[b"test", b"-e", b"/nonexistent-for-nsh-tests"]), 1);
    }

    #[test]
    fn bracket_requires_its_bracket() {
        assert_eq!(eval(&[b"[", b"x", b"]"]), 0);
        let args = [BStr::new(b"["), BStr::new(b"x")];
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
        let error = run(shell, &args).expect_err("`[ x` is missing its bracket");
        assert_eq!(error.message().to_vec(), b"missing ]".to_vec());
    }
}
