//! The invariants `expand.rs`'s owned glob buffer rests on.
//!
//! Companion to `expansion_buffer.rs`, and there for the same reason:
//! `docs/idiomatization.md` §5 asks for targeted tests over `expand.rs`
//! "notwithstanding [dec:nsh:differential-is-the-oracle] -- the decision
//! rejected a *complete* per-function suite, not a targeted one".
//!
//! `expmeta` builds one candidate path across a recursion that is one frame
//! per component, and each frame owns the prefix below `expdir_len` while
//! writing above it. Two things hold that together and neither is visible
//! in a corpus case that globs a short path in a shallow tree: the append
//! happens *at the cursor*, which is what cuts the buffer back to the
//! parent's prefix when a child returns, and the reservation is exactly
//! `expdir_len + name_len + 1`, which the C could afford to get wrong
//! because a region block is never smaller than 504 bytes. So the fixtures
//! here are deliberately deep, wide and long-named.
//!
//! Expected output was checked against the reference C, not against the
//! port.

use std::io::Read;
use std::os::unix::io::FromRawFd;
use std::path::PathBuf;

use nsh::streams::{self, Streams};

unsafe fn pipe() -> (i32, i32) {
    let mut fds = [0i32; 2];
    assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
    (fds[0], fds[1])
}

fn read_all(fd: i32) -> Vec<u8> {
    let mut v = Vec::new();
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    f.read_to_end(&mut v).expect("read pipe");
    v
}

/// Run `script` with the shell's stdout on a pipe and return what it wrote.
/// Forks, because `main_fn` ends in `exitshell`, which `_exit`s.
fn out_of(script: &str) -> Vec<u8> {
    let (r, w) = unsafe { pipe() };
    let argv: Vec<Vec<u8>> = vec![b"sh".to_vec(), b"-c".to_vec(), script.as_bytes().to_vec()];
    unsafe {
        let pid = libc::fork();
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if info
                    .payload()
                    .downcast_ref::<nsh::error::Longjmp>()
                    .is_some()
                {
                    return;
                }
                default_hook(info);
            }));
            let lent = streams::install(Streams {
                stdin: 0,
                stdout: w,
                stderr: 2,
            })
            .expect("install");
            core::mem::forget(lent);
            nsh::shellmain::main_fn(argv.len() as libc::c_int, argv, streams::streams());
        }
        libc::close(w);
        let mut status = 0i32;
        assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
    }
    read_all(r)
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

/// The append happens at the cursor, not at the end of the buffer.
///
/// `stnputs(s, n, p)` derives its length from `p` (`makestrspace` opens
/// with `len = p - stacknxt`), so a frame that a recursive `expmeta`
/// returned into cuts the buffer back to its own `expdir_len` simply by
/// appending at `cp + expdir_len`. Appending at the end instead — the
/// obvious reading — would carry the child's deeper prefix into the
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
    let out = out_of(&format!("echo {dir}/d*/e*/leaf"));

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
    let out = out_of(&format!("echo {dir}/g*/h*/f*"));

    let want = names
        .iter()
        .map(|n| format!("{dir}/g{pad}/h{pad}/{n}"))
        .collect::<Vec<_>>()
        .join(" ")
        + "\n";
    assert_eq!(String::from_utf8_lossy(&out), want);

    let _ = std::fs::remove_dir_all(&base);
}

/// The reservation is `expdir_len + name_len + 1` and it is exact.
///
/// `expmeta_rmescapes` writes the remaining pattern at the cursor through a
/// raw pointer with no bound of its own; the only thing keeping it inside
/// the buffer is that number, and `name_len == strlen(name)` at every
/// entry. In the C an error there is absorbed by a 504-byte minimum block,
/// so the case that would expose it is a *long literal tail* after the last
/// metacharacter — which is what the trailing components below are.
///
/// The escaped `*` in the middle component is the other half: it makes
/// `expmeta_rmescapes` take its `rmescapes` path over a name whose encoded
/// form is longer than its decoded one — and so lands one byte *under* the
/// bound rather than exactly on it, which is why the unescaped tail is
/// here too.
#[test]
fn a_long_literal_tail_stays_inside_the_reservation() {
    let base = fixture("reservation");
    let pad = "x".repeat(200);
    mkdirs(&base, &format!("m{pad}/n*n/o{pad}"));
    touch(&base, &format!("m{pad}/n*n/o{pad}/p{pad}"));
    mkdirs(&base, &format!("m{pad}/s{pad}"));
    touch(&base, &format!("m{pad}/s{pad}/t{pad}"));
    let dir = base.display();

    // Nothing escaped, so `expmeta_rmescapes` writes `name_len` bytes and
    // a NUL: the whole reservation, exactly.
    let out = out_of(&format!("echo {dir}/m*/s{pad}/t{pad}"));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m{pad}/s{pad}/t{pad}\n")
    );

    let out = out_of(&format!("echo {dir}/m*/n\\*n/o{pad}/p{pad}"));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m{pad}/n*n/o{pad}/p{pad}\n")
    );

    // Same tail, reached through a metacharacter in the last component, so
    // the loop's `stnputs` rather than `expmeta_rmescapes` writes it.
    let out = out_of(&format!("echo {dir}/m*/n\\*n/o*/p*"));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m{pad}/n*n/o{pad}/p{pad}\n")
    );

    // A pattern that matches nothing is echoed verbatim, with the escape
    // removed by `rmescapes` rather than by the glob.
    let out = out_of(&format!("echo {dir}/m*/n\\*n/o{pad}/q{pad}"));
    assert_eq!(
        String::from_utf8_lossy(&out),
        format!("{dir}/m*/n*n/o{pad}/q{pad}\n")
    );

    let _ = std::fs::remove_dir_all(&base);
}
