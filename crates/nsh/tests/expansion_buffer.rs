//! The invariants `expand.rs`'s owned expansion buffer rests on.
//!
//! `docs/idiomatization.md` §5 names `expand.rs` as one of two genuinely
//! dangerous steps and asks for targeted tests here, "notwithstanding
//! [dec:nsh:differential-is-the-oracle] -- the decision rejected a
//! *complete* per-function suite, not a targeted one". These are that.
//!
//! What they pin is specifically the thing the differential corpus cannot
//! aim at: the C's builder is a cursor into a region that copies its whole
//! block when it grows, and the port's is a `BString` whose `reserve`
//! copies only the first `len` bytes. So bytes written *above* the cursor
//! survive in the C and need an argument in the port, and two places in
//! `expand.c` write there and read the byte back. Each case below drives
//! enough appends to make a reallocation likely at the point the argument
//! is about.

use nsh::streams::Streams;

fn read_all(fd: &nsh_platform::Descriptor) -> Vec<u8> {
    nsh_platform::read_to_end(fd).expect("read pipe")
}

/// Run `script` with the shell's stdout on a pipe and return what it
/// wrote. Forks, because the child becomes a shell and ends there.
fn out_of(script: &str) -> Vec<u8> {
    let (r, w) = nsh_platform::pipe().expect("create pipe");
    let command = script.as_bytes().to_vec();
    nsh_platform::run_in_child(move || {
        let supplied =
            Streams::from_fds(std::io::stdin(), &w, std::io::stderr()).expect("duplicate streams");
        let mut shell = nsh::Shell::builder()
            .arg0(bstr::BStr::new(b"sh"))
            .inherit_env()
            .streams(supplied)
            .host(nsh::ProcessHost)
            .build()
            .expect("build process shell");
        let status = shell.run_to_completion(nsh::Startup::command(command));
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child");
    read_all(&r)
}

/// `expari` winds the cursor back over the arithmetic text and then hands
/// `arith` a pointer *at* the bytes it just truncated away. Nothing on
/// that path may grow the buffer, or the text moves out from under the
/// evaluator.
///
/// The word here is long enough that the buffer has reallocated at least
/// once before the arithmetic, and the arithmetic itself is long enough
/// that a reallocation would take it with it.
#[test]
fn arithmetic_text_survives_being_read_from_above_the_cursor() {
    let pad = "p".repeat(400);
    let script = format!(
        "x={pad}; echo \"${{x}}$(( 1 + 2 * 3 + 4 * 5 + 6 * 7 + 8 * 9 ))${{x}}\"; \
         echo \"$(( (1 + 2) * (3 + 4) ))\""
    );
    let out = out_of(&script);
    let want = format!("{pad}141{pad}\n21\n");
    assert_eq!(String::from_utf8_lossy(&out), want);
}

/// `subevalvar` closes with `*loc = '\0'; STADJUST(loc - expdest, expdest)`,
/// which puts the terminator one past the length. The claim is that
/// `argstr` re-supplies the word's own terminator before anything reads
/// the buffer as a string -- so a trim must be usable both at the end of a
/// word and in the middle of one, with more expansion after it.
#[test]
fn a_trim_leaves_the_word_terminated_wherever_it_sits() {
    let pad = "q".repeat(300);
    let script = format!(
        "x={pad}TAIL; y=abc; \
         echo \"${{x%TAIL}}\"; \
         echo \"${{x%TAIL}}-mid-${{y#a}}-end\"; \
         echo \"${{y%%b*}}${{x##*q}}${{y##*b}}\""
    );
    let out = out_of(&script);
    let want = format!("{pad}\n{pad}-mid-bc-end\naTAILc\n");
    assert_eq!(String::from_utf8_lossy(&out), want);
}

/// `expandarg(n, NULL, flag)` does not grab its result; the two callers
/// read it back out of the buffer. `parser::expandstr` is the one a
/// conversion that reads only `expand.c` misses, and all it costs is the
/// `+ ` on an xtrace line -- so pin both readers together.
#[test]
fn the_ungrabbed_result_reaches_ps4_and_a_here_document() {
    let out = out_of("PS4='+X+ '; set -x; echo traced; set +x");
    // xtrace goes to stderr, which the child inherits; what stdout must
    // show is that the traced command still ran exactly once.
    assert_eq!(String::from_utf8_lossy(&out), "traced\n");

    let pad = "h".repeat(300);
    let script = format!(
        "v=V; while IFS= read -r line; do printf '%s\\n' \"$line\"; done <<EOF\n\
         {pad}$v${{v}}$(echo S)$((6*7))\n\
         EOF\n"
    );
    let out = out_of(&script);
    assert_eq!(String::from_utf8_lossy(&out), format!("{pad}VVS42\n"));
}

/// The word's terminator comes from `argstr`'s `*(q - 1) &= end - 1`,
/// which turns the closing NUL/CTLENDVAR/CTLENDARI into a NUL. A `$'\0'`
/// inside a word ends it there, and the byte count is what says so.
#[test]
fn an_embedded_nul_ends_the_word() {
    let out = out_of("x=$'a\\0b'; printf '[%s][%d]\\n' \"$x\" \"${#x}\"");
    assert_eq!(String::from_utf8_lossy(&out), "[a][1]\n");
}
