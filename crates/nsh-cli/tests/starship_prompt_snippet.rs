//! The starship integration this shell ships, checked against a real
//! starship rather than against a transcript of one.
//!
//! `share/nsh/starship.nsh` is ours, which is the cost of not evaluating
//! starship's own Bash init: their `prompt` arguments are now something
//! this repository has to track. So the check that matters is not "a
//! prompt appeared" -- that would keep passing after starship renamed a
//! flag and began ignoring ours -- but "every flag the snippet passes is
//! still one starship accepts".
//!
//! Both tests require starship on the host. That is a property of the
//! machine rather than of the shell, so they are `#[ignore]`d: the runner
//! reports them as not run, which is the static skip
//! `[spec:nsh:req:oracle.cannot-measure-is-a-failure]` permits. Run them
//! with `cargo test -p nsh-cli --test starship_prompt_snippet -- --ignored`.

use std::process::Command;

const SNIPPET: &str = "share/nsh/starship.nsh";

/// The flags `share/nsh/starship.nsh` passes to `starship prompt`.
///
/// Read out of the snippet rather than written down twice, so a flag
/// added there without a thought about compatibility still reaches this
/// check.
fn flags_the_snippet_passes() -> Vec<String> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let text = std::fs::read_to_string(format!("{root}/{SNIPPET}"))
        .unwrap_or_else(|error| panic!("cannot read {SNIPPET}: {error}"));
    let mut flags: Vec<String> = text
        .split_whitespace()
        .filter(|word| word.starts_with("--"))
        .map(|word| {
            /* The snippet writes `--flag="$VALUE"` and closes a command
             * substitution on the same word, so a flag arrives with its
             * value and sometimes a `)` behind it. */
            let word = word.split('=').next().unwrap_or(word);
            word.trim_end_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .to_owned()
        })
        .collect();
    flags.sort();
    flags.dedup();
    assert!(
        !flags.is_empty(),
        "{SNIPPET} passes no flags at all -- either it stopped calling starship \
         or this reader stopped finding what it passes"
    );
    flags
}

// [spec:nsh:req:interactive.prompt-state/test]
#[test]
#[ignore = "needs starship installed; a host property, not a shell one"]
fn every_flag_the_snippet_passes_still_exists() {
    let help = Command::new("starship")
        .args(["prompt", "--help"])
        .output()
        .expect("starship is not on PATH; run this test only where it is");
    let help = String::from_utf8_lossy(&help.stdout);

    for flag in flags_the_snippet_passes() {
        assert!(
            help.contains(&flag),
            "{SNIPPET} passes {flag}, which `starship prompt --help` no longer \
             lists. The snippet is ours to keep current: fix it there rather \
             than loosening this check."
        );
    }
}

// [spec:nsh:req:interactive.prompt-state/test]
#[test]
#[ignore = "needs starship installed; a host property, not a shell one"]
fn the_failing_status_reaches_the_rendered_prompt() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    /* starship's default preset spans several lines, so the rendered
     * prompt carries newlines of its own. Fold them to a byte that cannot
     * appear in a terminal sequence, or the marker below is the only thing
     * on its line and the reader finds nothing after it. */
    let script = format!(
        ". {root}/{SNIPPET}\n\
         false\n\
         printf 'PROMPT<%s>\\n' \"$(printf '%s' \"$PS1\" | tr '\\n' '~')\"\n\
         exit\n"
    );

    /* Asking starship twice, once for each status, and requiring the two
     * answers to differ. A single render proves only that something was
     * produced; the status is the one thing this shell had to carry into
     * the hook, so it is the thing worth asserting. */
    let after_failure = render(&script);
    let after_success = render(&script.replace("false\n", "true\n"));

    assert!(
        !after_failure.is_empty() && !after_success.is_empty(),
        "no prompt was rendered at all"
    );
    assert_ne!(
        after_failure, after_success,
        "the prompt is the same after a failing command as after a \
         succeeding one, so `$?` is not reaching starship"
    );
    assert!(
        !after_failure.contains("\\["),
        "the prompt carries Bash's non-printing markers, so the snippet asked \
         starship to treat this shell as Bash: {after_failure:?}"
    );
}

fn render(script: &str) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_nsh"))
        .arg("-i")
        /* An empty config, so a user's own starship settings cannot
         * disable the module this test reads and make both renders
         * identical for a reason that is not the shell's. */
        .env("STARSHIP_CONFIG", "/dev/null")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("stdin was piped")
                .write_all(script.as_bytes())?;
            child.wait_with_output()
        })
        .expect("the shell did not run");
    let text = String::from_utf8_lossy(&output.stdout);
    /* An interactive shell writes its prompt to standard output, so the
     * marker is somewhere in a line rather than at the start of one. */
    text.lines()
        .find_map(|line| line.split_once("PROMPT<"))
        .map(|(_, rest)| rest.trim_end_matches('>').to_owned())
        .unwrap_or_default()
}
