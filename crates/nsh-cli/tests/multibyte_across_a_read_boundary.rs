//! A multi-byte character split across the shell's input buffer,
//! measured against the pinned Bash 5.3.
//!
//! `read` settles a multi-byte character out of the bytes the input
//! frame is already holding, which costs one thread-locale selection
//! rather than one per byte. It can only do that when the whole
//! character is there, and the case that has to keep working is the one
//! where it is not: the shell then goes back to the byte-at-a-time
//! decoder and asks the source for the rest.
//!
//! Reaching that case on purpose is the whole design here, and three
//! ways of trying do not reach it. Writing a character's bytes to the
//! shell's stdin pipe in two pieces does not: the shell is blocked in
//! `read(2)`, both writes land before the kernel returns, and the
//! character arrives whole. A `printf` handshake first does not help,
//! because it fixes where the shell is before the first write and the
//! race is between the first write and the second. A pseudo-terminal
//! does not either: a canonical terminal hands over whole lines, so
//! there is no half-character to leave in the buffer, and turning
//! canonical mode off makes the shell read a byte at a time, which is
//! the case with no buffer rather than the case with half a character
//! in it.
//!
//! What does reach it is a regular file, which is the one source this
//! shell buffers, because it is the one it can seek back on when the
//! record ends. A refill takes a fixed number of bytes and stops
//! wherever that lands, so a run of three-byte characters straddles it
//! by construction -- and which of the three positions it lands in is
//! decided by where the run starts, which is what the padding below is
//! for.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// U+20AC, three bytes, and the character the run is made of.
const WIDE: &[u8] = "€".as_bytes();

/// How many bytes of that run stand in front of the mark.
///
/// It has to be more than one refill, or the file arrives whole and
/// nothing straddles anything. The buffer has been 8 KiB since dash;
/// this is eight times that, so the run still spans a refill on a shell
/// whose buffer has doubled three times.
const RUN: usize = 64 * 1024;

/// What follows the run. A row that lost its place in the file reads
/// something else here and says so, rather than agreeing with a
/// reference that also lost its place.
const MARK: &[u8] = b"MARK";

/// A UTF-8 charmap, by whichever of its names this host answers to.
///
/// `LOCPATH` stops glibc reading the system archive, so a host that
/// keeps `C.UTF-8` only there has it under the generated third name.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
fn utf8_name() -> &'static str {
    ["C.UTF-8", "C.utf8", "en_US.UTF-8"]
        .into_iter()
        .find(|name| nsh_platform::Locale::new(name.as_bytes(), &[]).is_ok())
        .expect(
            "no UTF-8 charmap: tried C.UTF-8, C.utf8 and en_US.UTF-8, and \
             without one every character below is one byte wide\n\
             build one and name it to the run:\n\
             \x20   export LOCPATH=$(tests/build-locales.sh)",
        )
}

/// `pad` filler bytes, then whole wide characters up to [`RUN`], then
/// the mark. Returns the file and how many characters the run holds.
fn write_fixture(pad: usize) -> (PathBuf, usize) {
    let mut bytes = vec![b'.'; pad];
    let mut characters = 0;
    while bytes.len() < pad + RUN {
        bytes.extend_from_slice(WIDE);
        characters += 1;
    }
    bytes.extend_from_slice(MARK);
    bytes.push(b'\n');

    let path = std::env::temp_dir().join(format!("nsh-read-boundary-{}-{pad}", std::process::id()));
    std::fs::write(&path, &bytes).expect("write the fixture the shells read");
    (path, characters)
}

/// One script put to one shell with the fixture as its standard input.
///
/// The environment is cleared to the same names on both sides, so what
/// is compared is two shells rather than two environments. `LOCPATH` is
/// carried through when the run has one, because the charmap named
/// above may be the generated one.
fn answer(shell: &Path, dialect: &[&str], locale: &str, input: &Path, script: &str) -> Vec<u8> {
    let stdin = std::fs::File::open(input).expect("open the fixture");
    let mut command = Command::new(shell);
    command
        .args(dialect)
        .arg("-c")
        .arg(script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", locale)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(generated) = std::env::var_os("LOCPATH") {
        command.env("LOCPATH", generated);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    let mut answer = output.stdout;
    answer.extend_from_slice(format!("status={}\n", output.status.code().unwrap_or(-1)).as_bytes());
    answer
}

/// Every position a refill can stop at inside a three-byte character,
/// taken one at a time.
///
/// A refill of `B` bytes over a run starting at `pad` stops
/// `(B - pad) % 3` bytes into whichever character it lands in, so the
/// three rows below cover the three positions whatever `B` is: one where
/// the character is whole in the buffer, one where a single byte of it
/// is there and the buffer is otherwise spent, and one -- the row this
/// file exists for -- where two of its three bytes are there and the
/// shortcut has to decline and go back to the source.
#[test]
fn a_character_the_refill_split_is_still_one() {
    let reference = pinned_bash::path();
    let shell = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let locale = utf8_name();

    let mut expected = b"b=".to_vec();
    expected.extend_from_slice(MARK);
    expected.extend_from_slice(b"\nstatus=0\n");

    for pad in 0..WIDE.len() {
        let (fixture, characters) = write_fixture(pad);
        let script = format!(
            "read -rn{} a\nread -rn{} b\nprintf 'b=%s\\n' \"$b\"\n",
            pad + characters,
            MARK.len()
        );
        let theirs = answer(&reference, &[], locale, &fixture, &script);
        let ours = answer(shell, &["-o", "bash"], locale, &fixture, &script);
        std::fs::remove_file(&fixture).expect("remove the fixture");

        assert_eq!(
            theirs, expected,
            "with {pad} bytes of padding the reference did not reach the mark, \
             so this row compared two shells that both lost their place"
        );
        assert_eq!(ours, theirs, "with {pad} bytes of padding");
    }
}
