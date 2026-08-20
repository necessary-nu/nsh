//! The invariants `expand.rs`'s owned glob buffer rests on.
//!
//! Companion to `expansion_buffer.rs`, and there for the same reason:
//! `docs/idiomatization.md` §5 asks for targeted tests over `expand.rs`
//! "notwithstanding [dec:nsh:differential-is-the-oracle] -- the decision
//! rejected a *complete* per-function suite, not a targeted one".
//!
//! `expmeta` builds one candidate path across a recursion that is one frame
//! per component, and each frame owns the prefix below `expdir_len` while
//! writing above it. What holds that together is not visible in a corpus
//! case that globs a short path in a shallow tree: every write to the
//! buffer is a *truncate to this frame's `expdir_len`, then append*, and a
//! truncate to the wrong length carries a sibling's or a child's bytes
//! into the next candidate. So the fixtures here are deliberately deep,
//! wide and long-named — long enough that the buffer has reallocated
//! several times before the leaves.
//!
//! The C reached the same place through a cursor and a reservation: an
//! append at `p` opened with `len = p - stacknxt`, so writing at a cursor
//! below the end discarded what was above it, and
//! `growstackto(expdir_len + name_len + 1)` was the only bound on writes
//! that carried none of their own. Both are gone — the truncate says the
//! first out loud, and appending needs no bound — but the cases that would
//! have caught an error in either are the same cases, so they stay.
//!
//! Expected output was checked against the reference C, not against the
//! port.

use std::path::PathBuf;

use nsh::streams::Streams;

fn read_all(fd: &nsh_platform::Descriptor) -> Vec<u8> {
    nsh_platform::read_to_end(fd).expect("read pipe")
}

/// Run `script` with the shell's stdout on a pipe and return what it wrote.
/// Forks, because the child becomes a shell and ends there.
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

/// A fresh directory under the system temporary directory. Its own name
/// carries no glob metacharacter, so the pattern's literal prefix is
/// literal.
fn fixture(name: &str) -> PathBuf {
    let mut d = std::env::temp_dir();
    d.push(format!("nsh-glob-buffer-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create fixture");
    d
}

fn mkdirs(base: &PathBuf, rel: &str) {
    std::fs::create_dir_all(base.join(rel)).expect("create_dir_all");
}

fn touch(base: &PathBuf, rel: &str) {
    std::fs::File::create(base.join(rel)).expect("create file");
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The append starts from this frame's prefix, not from the end of the
/// buffer.
///
/// A frame that a recursive `expmeta` returned into cuts the buffer back to
/// its own `expdir_len` before writing the next candidate. Appending at the
/// end instead — the obvious reading, and what the C's `stnputs(s, n, p)`
/// looks like until you notice `makestrspace` opening with
/// `len = p - stacknxt` — would carry the child's deeper prefix into the
/// parent's next candidate.
///
/// The tree has two matching directories at each of two levels, so the
/// second entry of every level is appended after a completed recursion has
/// left the buffer longer than that level's prefix. The names are long
/// enough that the buffer has reallocated well before the leaves.
#[test]
fn an_append_after_a_recursion_starts_from_the_parents_prefix() {
    let base = fixture("recursion");
    let pad = "z".repeat(120);
    for one in ["a", "b"] {
        for two in ["a", "b"] {
            mkdirs(&base, &format!("d{pad}{one}/e{pad}{two}"));
            touch(&base, &format!("d{pad}{one}/e{pad}{two}/leaf"));
        }
    }
    let dir = base.display();
    let quoted = shell_quote(&base.to_string_lossy());
    let out = out_of(&format!(
        "set -- {quoted}/d*/e*/leaf; printf '%s\\n' \"$*\""
    ));

    let mut want = String::new();
    for one in ["a", "b"] {
        for two in ["a", "b"] {
            if !want.is_empty() {
                want.push(' ');
            }
            want.push_str(&format!("{dir}/d{pad}{one}/e{pad}{two}/leaf"));
        }
    }
    want.push('\n');
    assert_eq!(String::from_utf8_lossy(&out), want);

    let _ = std::fs::remove_dir_all(&base);
}

/// Keeping a candidate re-seeds the buffer with the prefix and nothing
/// else.
///
/// The C spells that `STARTSTACKSTR(enddir); stnputs(name, expdir_len,
/// enddir)`, which reads as rebuilding the prefix and is really only
/// paying for `grabstackstr` having given the block away. Owned it is a
/// truncate — and a truncate to the wrong length, or a clear, shows up as
/// the second and later matches in one directory losing or duplicating
/// their prefix.
///
/// Every file here is a separate `addfnamealt` at the same `expdir_len`,
/// under a prefix long enough to have reallocated.
#[test]
fn a_kept_candidate_leaves_exactly_the_prefix_behind() {
    let base = fixture("reseed");
    let pad = "y".repeat(150);
    mkdirs(&base, &format!("g{pad}/h{pad}"));
    let names = ["fa", "fb", "fc", "fd", "fe", "ff", "fg", "fh"];
    for n in names {
        touch(&base, &format!("g{pad}/h{pad}/{n}"));
    }
    let dir = base.display();
    let quoted = shell_quote(&base.to_string_lossy());
    let out = out_of(&format!("set -- {quoted}/g*/h*/f*; printf '%s\\n' \"$*\""));

    let want = names
        .iter()
        .map(|n| format!("{dir}/g{pad}/h{pad}/{n}"))
        .collect::<Vec<_>>()
        .join(" ")
        + "\n";
    assert_eq!(String::from_utf8_lossy(&out), want);

    let _ = std::fs::remove_dir_all(&base);
}

/// A long literal tail after the last metacharacter, written by
/// `expmeta_rmescapes` rather than by the readdir loop.
///
/// This was written when `expmeta_rmescapes` wrote the remaining pattern at
/// a raw cursor with no bound of its own, and the only thing keeping it
/// inside the buffer was `growstackto(expdir_len + name_len + 1)` — a
/// number the C could afford to get wrong, because a region block is never
/// smaller than 504 bytes. It appends now, so there is no bound left to be
/// exact about; what the case still covers is the branch itself, which is
/// the one place a whole component is unescaped in one go and then handed
/// to `lstat` instead of matched.
///
/// The escaped `*` in the middle component is the other half: it makes
/// `expmeta_rmescapes` take its `rmescapes` path over a name whose encoded
/// form is longer than its decoded one, so the appended length and the
/// pattern's length differ — which is exactly what `addfnamealt` used to
/// have to be told, and no longer does.
#[test]
fn a_literal_tail_after_the_metacharacter() {
    if !nsh_platform::supports_glob_metacharacters_in_filenames() {
        return;
    }
    let base = fixture("reservation");
    let pad = "x".repeat(200);
    mkdirs(&base, &format!("m{pad}/n*n/o{pad}"));
    touch(&base, &format!("m{pad}/n*n/o{pad}/p{pad}"));
    mkdirs(&base, &format!("m{pad}/s{pad}"));
    touch(&base, &format!("m{pad}/s{pad}/t{pad}"));
    let dir = base.display();
    let quoted = shell_quote(&base.to_string_lossy());

    // Nothing escaped, so `expmeta_rmescapes` appends `name_len` bytes and
    // the caller adds the NUL that `lstat` needs.
    let out = out_of(&format!(
        "set -- {quoted}/m*/s{pad}/t{pad}; printf '%s\\n' \"$*\""
    ));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m{pad}/s{pad}/t{pad}\n")
    );

    let out = out_of(&format!(
        "set -- {quoted}/m*/n\\*n/o{pad}/p{pad}; printf '%s\\n' \"$*\""
    ));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m{pad}/n*n/o{pad}/p{pad}\n")
    );

    // Same tail, reached through a metacharacter in the last component, so
    // the loop's `stnputs` rather than `expmeta_rmescapes` writes it.
    let out = out_of(&format!(
        "set -- {quoted}/m*/n\\*n/o*/p*; printf '%s\\n' \"$*\""
    ));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m{pad}/n*n/o{pad}/p{pad}\n")
    );

    // A pattern that matches nothing is echoed verbatim, with the escape
    // removed by `rmescapes` rather than by the glob.
    let out = out_of(&format!(
        "set -- {quoted}/m*/n\\*n/o{pad}/q{pad}; printf '%s\\n' \"$*\""
    ));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m*/n*n/o{pad}/q{pad}\n")
    );

    let _ = std::fs::remove_dir_all(&base);
}

/// A literal tail that `lstat` rejects leaves the buffer as it found it.
///
/// This is the one path with no counterpart in the C. The C unescapes the
/// tail through a raw cursor and never tells the block about it, so when
/// `lstat` fails there is nothing to undo: the bytes were never counted.
/// Appending counts them, so the failure has to rewind, and forgetting to
/// would leave a rejected candidate's tail glued under the next sibling's.
///
/// The fixture alternates: the odd directories have the file and the even
/// ones do not, so every kept candidate is preceded by a rejected one at
/// the same `expdir_len`, and the tail is long enough that the buffer grew
/// to hold it.
#[test]
fn a_rejected_tail_rewinds_the_buffer() {
    let base = fixture("lstat-rewind");
    let pad = "w".repeat(180);
    let dirs = ["k1", "k2", "k3", "k4", "k5", "k6"];
    for (i, d) in dirs.iter().enumerate() {
        mkdirs(&base, d);
        if i % 2 == 0 {
            touch(&base, &format!("{d}/leaf{pad}"));
        }
    }
    let dir = base.display();
    let quoted = shell_quote(&base.to_string_lossy());
    let out = out_of(&format!(
        "set -- {quoted}/k*/leaf{pad}; printf '%s\\n' \"$*\""
    ));

    let want = dirs
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 0)
        .map(|(_, d)| format!("{dir}/{d}/leaf{pad}"))
        .collect::<Vec<_>>()
        .join(" ")
        + "\n";
    assert_eq!(String::from_utf8_lossy(&out), want);

    let _ = std::fs::remove_dir_all(&base);
}
