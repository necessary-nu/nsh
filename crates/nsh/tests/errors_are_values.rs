//! What the two shared catch frames swallow, and what they let past.
//!
//! `docs/errors-are-values.md` §6 names `redir::redirectsafe` and
//! `parser::expandstr` as the two genuinely dangerous conversions in this
//! node, and gives the reason: they share
//! `expand::restore_handler_expandarg`, whose nine lines decide on the
//! exception path whether to re-raise or to swallow-and-`ifsfree`. Get
//! that decision wrong in either direction and the failure is not a crash
//! but a *silently swallowed error* — the shell carries on with a
//! half-built redirection or a half-expanded prompt and produces a
//! plausible wrong answer. That is the failure mode §6 calls the dangerous
//! one, and it is the one this node has already shipped once: `96cadd4`
//! made `redir::sh_open_fail` return a value, two bare-statement call
//! sites stopped being stops, and `set -C; echo a > f; echo b > f`
//! reported twice and then wrote through a descriptor nobody had opened.
//!
//! So these are written **before** the two frames convert, and they must
//! read the same afterwards. Each one was measured against
//! `tests/.build/ref/src/dash` before it was written here; they encode
//! dash's behaviour, not the port's.
//!
//! Three things are pinned, and they are the three arms of that decision:
//!
//! * **Swallow.** A diagnostic beneath either frame is reported, the frame
//!   returns, and the shell runs the next command. `docs/api-design.md`
//!   §3.3 is the contract — a diagnostic dash reports and carries on past
//!   never reaches a return value at all.
//! * **Do not swallow.** The same diagnostic under a *special* built-in
//!   aborts the script, because `evalcommand` re-raises everything except
//!   an `EXERROR` from a non-special built-in.
//! * **`ifsfree` ran.** The swallowing arm frees the IFS region list, and
//!   a stale region mis-splits the *next* word rather than the failing
//!   one. Field counts after a swallowed failure are how that is visible
//!   from outside.
//!
//! What is deliberately **not** here is the re-raise arm's own trigger.
//! The only thing that can still arrive at these frames as a jump once the
//! diagnostics are values is `EXINT`, which `error.rs:254-256` makes
//! reachable only in an interactive root shell — `tests/harness/ptydiff`
//! is its oracle, and a `debug_assert` on the arm is its instrument. §5's
//! table says the same thing in the row for "the `EXINT` unwind reaching a
//! handler".

use nsh::streams::Streams;

/// Run `script` with stdout and stderr merged onto one pipe — which is how
/// `tests/harness/dscase.sh:64-71` runs all 61,498 differential cases, and
/// therefore the only stream shape whose byte order this crate has an
/// oracle for — and return the merged bytes with the shell's exit status.
///
/// Forks, because the child becomes a shell and ends there.
fn run(script: &str) -> (String, i32) {
    let (r, w) = nsh_platform::pipe().expect("create pipe");

    let argv: Vec<Vec<u8>> = vec![b"sh".to_vec(), b"-c".to_vec(), script.as_bytes().to_vec()];

    let status = nsh_platform::run_in_child(move || {
        let supplied = Streams::from_fds(std::io::stdin(), &w, &w).expect("duplicate streams");
        /* `main_fn` returns now — [dec:nsh:host-owns-the-process] made
        ending the process the caller's act — so this fork's child
        has to end itself. Returning would carry it back into the
        test harness after the fork. */
        let status = nsh::shellmain::main_fn(argv, supplied);
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child");
    let bytes = nsh_platform::read_to_end(&r).expect("read pipe");
    (String::from_utf8_lossy(&bytes).into_owned(), status)
}

/// A path in the system temporary directory that no other test uses.
fn scratch(tag: &str) -> String {
    let path = std::env::temp_dir().join(format!("nsh-eav-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_file(&path);
    path.to_string_lossy().into_owned()
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------
// redirectsafe: the swallowing arm
// ---------------------------------------------------------------------

/// A redirection that cannot be opened on a plain command is reported and
/// the script goes on to the next command with status 0 — the status of
/// the `echo` that follows, not the 2 the failure took.
///
/// This is `evalcommand`'s call (`eval.rs:1028`): `redirectsafe` returns
/// non-zero, `bail:` takes the status, and `spclbltin <= 0` means it is
/// not re-raised.
#[test]
fn open_failure_is_swallowed() {
    let (out, status) = run("echo a > /nonexistent-dir/x; echo after");
    assert_eq!(
        out,
        "sh: 1: cannot create /nonexistent-dir/x: Directory nonexistent\nafter\n"
    );
    assert_eq!(status, 0);
}

/// The same failure on a *compound* command goes through the other
/// `redirectsafe` call (`eval.rs:269`, the `NREDIR` arm of `evaltree`),
/// which sets `checkexit = EV_TESTED` and skips the body. The body must
/// not run and the script must continue.
#[test]
fn compound_redirect_skips_body() {
    let (out, status) = run("{ echo body; } > /nonexistent-dir/x; echo after");
    assert_eq!(
        out,
        "sh: 1: cannot create /nonexistent-dir/x: Directory nonexistent\nafter\n"
    );
    assert_eq!(status, 0);
}

/// A function body is not a boundary: the failure is swallowed inside it,
/// the rest of the body runs, and so does the rest of the script.
#[test]
fn function_survives_redirect_failure() {
    let (out, status) = run("f() { echo x > /nonexistent-dir/x; echo in; }; f; echo after");
    assert_eq!(
        out,
        "sh: 1: cannot create /nonexistent-dir/x: Directory nonexistent\nin\nafter\n"
    );
    assert_eq!(status, 0);
}

// ---------------------------------------------------------------------
// redirectsafe: the arm that does not swallow
// ---------------------------------------------------------------------

/// A redirection error on a **special** built-in aborts the script. The
/// adopted Smoosh closure profile keeps that boundary and assigns status 1
/// to the shell error, intentionally taking precedence over dash's status 2.
///
/// It is the direction that matters most here: a conversion that made the
/// swallow unconditional would print the same bytes and then print
/// `after`, and only the status and the missing line say so.
#[test]
// [spec:nsh:req:compat.smoosh.error-contracts/test]
fn special_builtin_redirect_aborts() {
    let (out, status) = run(": > /nonexistent-dir/x; echo after");
    assert_eq!(
        out,
        "sh: 1: cannot create /nonexistent-dir/x: Directory nonexistent\n"
    );
    assert_eq!(status, 1);
}

/// `exec` with a failing redirection is the same shape and is worth its
/// own case, because `exec` reaches `redirectsafe` with `EV_EXIT` set and
/// no command word at all.
#[test]
// [spec:nsh:req:compat.smoosh.error-contracts/test]
fn failing_exec_redirect_aborts() {
    let (out, status) = run("exec 3> /nonexistent-dir/x; echo after");
    assert_eq!(
        out,
        "sh: 1: cannot create /nonexistent-dir/x: Directory nonexistent\n"
    );
    assert_eq!(status, 1);
}

/// The exact shape of the bug this node shipped at `96cadd4`, kept as a
/// case: under `noclobber` two failing creates must report twice and write
/// nothing, and the file must still hold what it held before.
#[test]
fn noclobber_failure_writes_nothing() {
    let f = scratch("noclobber");
    let quoted = shell_quote(&f);
    let script = format!(
        "echo original > {quoted}; set -C; echo a > {quoted}; echo b > {quoted}; \
         set +C; IFS= read -r saved < {quoted}; printf '%s\\n' \"$saved\""
    );
    let (out, status) = run(&script);
    let exists = nsh_platform::Locale::c()
        .unwrap()
        .error_message(&nsh_platform::already_exists_error());
    let _ = std::fs::remove_file(&f);
    assert_eq!(
        out,
        format!(
            "sh: 1: cannot create {f}: {exists}\n\
             sh: 1: cannot create {f}: {exists}\n\
             original\n"
        )
    );
    assert_eq!(status, 0);
}

// ---------------------------------------------------------------------
// expandstr: the swallowing arm
// ---------------------------------------------------------------------

/// `expandstr` swallows every diagnostic it catches and returns the string
/// it was given, **unexpanded**. `PS4` is the reachable driver: the
/// diagnostic goes out, the raw text is used as the trace prefix, and the
/// traced command still runs.
///
/// The unexpanded prefix in the expected bytes is the whole point. A
/// conversion that propagated the error instead would lose the trace line
/// and abort the script; one that swallowed without restoring `result`
/// would print an empty prefix or whatever the failed expansion left.
#[test]
fn unexpandable_prompt_is_used_raw() {
    let (out, status) = run("PS4='${nope?bad}'; set -x; echo hi");
    assert_eq!(out, "sh: 1: nope: bad\n${nope?bad}echo hi\nhi\n");
    assert_eq!(status, 0);
}

/// The same for a diagnostic raised by `expandstr`'s *parse* rather than
/// its expansion — `readtoken1` reaching an unterminated command
/// substitution. Both bridges inside that frame's closure are on this
/// path, so both are pinned.
#[test]
fn unparsable_prompt_is_swallowed() {
    let (out, status) = run("PS4='$(exit 7)+ '; set -x; echo hi");
    assert_eq!(
        out,
        "sh: 1: Syntax error: end of file unexpected (expecting \")\")\n\
         $(exit 7)+ echo hi\nhi\n"
    );
    assert_eq!(status, 0);
}

/// A failing prompt expansion happens once per traced command and must
/// stay swallowed every time, not merely the first.
#[test]
fn failing_prompt_swallowed_each_time() {
    let (out, status) = run("PS4='${nope?bad}'; set -x; echo one; echo two");
    assert_eq!(
        out,
        "sh: 1: nope: bad\n${nope?bad}echo one\none\n\
         sh: 1: nope: bad\n${nope?bad}echo two\ntwo\n"
    );
    assert_eq!(status, 0);
}

// ---------------------------------------------------------------------
// The `ifsfree` half of the swallowing arm
// ---------------------------------------------------------------------

/// `restore_handler_expandarg`'s swallowing arm calls `ifsfree()`, and the
/// reason it must is that a region left recorded by the *failed*
/// expansion would mis-split the *next* word. Field splitting after a
/// swallowed prompt failure is how that is observable from outside the
/// process: three colon-separated fields must still be three.
#[test]
fn prompt_failure_frees_ifs_regions() {
    let (out, status) =
        run("PS4='${nope?bad}'; set -x; IFS=:; v=a:b:c; set -- $v; set +x; echo \"$#/$1/$2/$3\"");
    assert!(
        out.ends_with("3/a/b/c\n"),
        "field splitting after a swallowed prompt failure: {out:?}"
    );
    assert_eq!(status, 0);
}

/// The same for the other frame: after a swallowed redirection failure the
/// next word still splits into the fields `IFS` asks for.
#[test]
fn redirect_failure_frees_ifs_regions() {
    let (out, status) =
        run("IFS=:; v=a:b:c; echo x > /nonexistent-dir/q; set -- $v; echo \"$#/$1/$2/$3\"");
    assert_eq!(
        out,
        "sh: 1: cannot create /nonexistent-dir/q: Directory nonexistent\n3/a/b/c\n"
    );
    assert_eq!(status, 0);
}

// ---------------------------------------------------------------------
// Flow: exit is control flow, and a forked child ends on its own
// ---------------------------------------------------------------------

/// `exit` in a subshell inside an EXIT trap.
///
/// The child's `exit` must reach a **fresh** `exitshell`, so the child
/// runs its *own* EXIT trap. It must not resume the parent's `exitshell`,
/// which is already past its `trap[0].take()` and would skip that trap.
///
/// The C never had the choice: a `longjmp` to `main_handler` lands at
/// `exit:`. Returning an exit as a value does have the choice, and taking
/// the wrong one skips `inner`. The child's explicit `exit 2` remains its
/// status after its own EXIT action; the parent observes 2, then its
/// successful `echo` becomes the parent's implicit-shutdown status.
// [spec:posix:req:builtin.exit.wait-status-from-n/test]
// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn subshell_exit_trap_runs() {
    let (out, status) = run(r#"trap '( trap "echo inner" EXIT; exit 2 ); echo $?' EXIT"#);
    assert_eq!(out, "inner\n2\n");
    assert_eq!(status, 0);
}

/// An explicit outer `exit n` survives a normally completed EXIT action. An
/// `exit` inside that action instead selects a replacement and cannot re-enter
/// the action, which was already taken.
// [spec:posix:req:builtin.exit.wait-status-from-n/test]
// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn explicit_exit_survives_action() {
    let (out, status) = run("trap 'echo T; true' EXIT; exit 5");
    assert_eq!(out, "T\n");
    assert_eq!(status, 5);

    // An `exit` inside the action makes its own status the final one.
    let (out, status) = run("trap 'echo T; exit 9' EXIT; exit 3");
    assert_eq!(out, "T\n");
    assert_eq!(status, 9);
}

/// An `exit` inside a command substitution ends the substitution's child
/// and nothing else — the one forked child that cannot hand its `Flow`
/// back, because it sits under the whole expansion chain.
#[test]
fn substitution_exit_ends_child() {
    let (out, status) = run(r#"x=$(echo a; exit 3; echo b); echo "[$x] $?"; echo after"#);
    assert_eq!(out, "[a] 3\nafter\n");
    assert_eq!(status, 0);
}

/// `set -e` aborts with no error value in flight at all — `false`
/// produces no diagnostic — which is why the abort is `Flow` and not
/// `Err`. `docs/api-design.md` §3.5 makes exactly this argument.
#[test]
fn set_e_abort_carries_nothing() {
    let (out, status) = run("set -e; echo a; false; echo unreachable");
    assert_eq!(out, "a\n");
    assert_eq!(status, 1);
}
