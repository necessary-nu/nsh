//! The drop-in directories, measured from the outside.
//!
//! Two halves, and the second is the security-relevant one. That an
//! interactive shell reads a package's drop-in is the feature; that a
//! build script running under `sh` does *not* is what keeps a recipe's
//! build from depending on which packages happen to be installed on the
//! builder. Both are checked here, from a tree built for each case.
//!
//! Every shell is run with `-c` rather than over a pty. `Interactive` is
//! one shell option and the gate reads that option, so `-i -c` exercises
//! the same branch a terminal session does while leaving standard output
//! free of prompts. The pty shape is checked by the tests that are about
//! the terminal.

use std::fs;
use std::io::Read as _;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const NSH: &str = env!("CARGO_BIN_EXE_nsh");

/// How long a shell that runs one `echo` may take before it is a hang.
///
/// Generous because this machine is shared with other build sessions, and
/// bounded because a new test file here is picked up by every session's
/// `cargo test --workspace`: an unbounded wait would be their hang too.
const PATIENCE: Duration = Duration::from_secs(60);

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

/// A private `$XDG_DATA_DIRS` tree and somewhere to put the output.
///
/// One per test, named by process and sequence, so no two checks share a
/// directory, an inherited descriptor or a working directory.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("nsh-vendor-conf-{}-{sequence}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create the fixture root");
        Self { root }
    }

    /// The `nsh/vendor_conf.d` under one entry of `$XDG_DATA_DIRS`,
    /// created.
    fn vendor(&self, data_directory: &str) -> PathBuf {
        let path = self.root.join(data_directory).join("nsh/vendor_conf.d");
        fs::create_dir_all(&path).expect("create a vendor directory");
        path
    }

    /// `$XDG_DATA_DIRS` naming the given entries in order.
    fn data_dirs(&self, entries: &[&str]) -> String {
        entries
            .iter()
            .map(|entry| self.root.join(entry).to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Run a shell against this tree and report what it wrote.
    ///
    /// Standard output and error go to files rather than pipes: the shell
    /// is interactive, so it may write more than this test reads, and a
    /// filled pipe with nobody draining it is a deadlock rather than a
    /// failure.
    fn run(
        &self,
        data_dirs: &str,
        environment: &[(&str, &Path)],
        arguments: &[&str],
        script: &str,
    ) -> Output {
        let out = self.root.join("stdout");
        let err = self.root.join("stderr");
        let mut command = Command::new(NSH);
        command
            .args(arguments)
            .arg("-c")
            .arg(script)
            /* The child's, not this process's: a check that changed the
             * suite's own working directory would be changing it for every
             * other check running as a thread beside it. */
            .current_dir(&self.root)
            .env("XDG_DATA_DIRS", data_dirs)
            /* Whatever the machine set would otherwise be read after the
             * drop-ins and could supply a marker this test attributes to
             * one. */
            .env_remove("ENV");
        for (name, value) in environment {
            command.env(name, value);
        }
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::from(fs::File::create(&out).expect("create stdout")))
            .stderr(Stdio::from(fs::File::create(&err).expect("create stderr")))
            .spawn()
            .expect("start the shell");

        let deadline = Instant::now() + PATIENCE;
        loop {
            match child.try_wait().expect("ask after the shell") {
                Some(_) => break,
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "the shell did not finish within {PATIENCE:?}; it had \
                         written {:?} to standard output and {:?} to standard \
                         error",
                        read(&out),
                        read(&err)
                    );
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        Output {
            stdout: read(&out),
            stderr: read(&err),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct Output {
    stdout: String,
    stderr: String,
}

/// Read a file the shell wrote, whatever bytes it holds.
///
/// A shell's output is not required to be UTF-8 and a test that panicked
/// on it would report a decoding failure where the interesting result is.
fn read(path: &Path) -> String {
    let mut bytes = Vec::new();
    if let Ok(mut file) = fs::File::open(path) {
        let _ = file.read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn write_drop_in(directory: &Path, name: &str, contents: &str) {
    fs::write(directory.join(name), contents).expect("write a drop-in");
}

/// A tree with two data directories: `10-first` and `20-second` under the
/// earlier one, `30-third` under the later.
///
/// Each appends its own name to `$TRAIL`, so one variable records both
/// which files ran and in what order.
fn trail_fixture() -> (Fixture, String) {
    let fixture = Fixture::new();
    let early = fixture.vendor("early");
    let late = fixture.vendor("late");
    write_drop_in(&early, "20-second.nsh", "TRAIL=\"$TRAIL second\"\n");
    write_drop_in(&early, "10-first.nsh", "TRAIL=\"$TRAIL first\"\n");
    write_drop_in(&late, "30-third.nsh", "TRAIL=\"$TRAIL third\"\n");
    let data_dirs = fixture.data_dirs(&["early", "late"]);
    (fixture, data_dirs)
}

// [spec:nsh:req:interactive.vendor-startup-path/test]
#[test]
fn an_interactive_shell_reads_drop_ins_by_name() {
    let (fixture, data_dirs) = trail_fixture();
    let output = fixture.run(
        &data_dirs,
        &[],
        &["-i", "-o", "bash"],
        "echo \"TRAIL:$TRAIL\"",
    );

    assert!(
        output.stdout.contains("TRAIL: first second third"),
        "the drop-ins did not all run in name order: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// The administrator directory is `/etc/nsh/conf.d` and cannot be written
/// by a test, so the ordering it depends on -- a directory read later
/// overrides one read earlier -- is measured between two vendor
/// directories, which is the same list and the same loop.
// [spec:nsh:req:interactive.vendor-startup-path/test]
#[test]
fn a_directory_read_later_overrides_one_read_earlier() {
    let fixture = Fixture::new();
    let early = fixture.vendor("early");
    let late = fixture.vendor("late");
    write_drop_in(&early, "10-setting.nsh", "SETTING=packaged\n");
    write_drop_in(&late, "10-setting.nsh", "SETTING=local\n");
    let data_dirs = fixture.data_dirs(&["early", "late"]);

    let output = fixture.run(
        &data_dirs,
        &[],
        &["-i", "-o", "bash"],
        "echo \"SETTING:$SETTING\"",
    );

    assert!(
        output.stdout.contains("SETTING:local"),
        "the later directory did not win: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// The half that matters if it is ever wrong. A recipe build script runs
/// under `sh`, and a drop-in reaching it would make the build depend on
/// the builder's installed package set.
// [spec:nsh:req:interactive.vendor-path-is-not-for-scripts+1/test]
#[test]
fn a_non_interactive_shell_reads_no_drop_in() {
    let (fixture, data_dirs) = trail_fixture();
    let output = fixture.run(&data_dirs, &[], &["-o", "bash"], "echo \"TRAIL:$TRAIL\"");

    assert!(
        output.stdout.contains("TRAIL:") && !output.stdout.contains("first"),
        "a non-interactive shell read a drop-in: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// The default dialect reads them, and this is the regression guard for
/// the one bug this feature has already had.
///
/// The rule first said the drop-ins were not read "in POSIX mode". In this
/// shell POSIX mode is not a mode one enters -- it is the default, and the
/// Bash dialect is the departure -- so that clause gated the feature on
/// `-o bash` and made it unreachable by the plain `nsh -i` a terminal
/// spawns, which is the only shell it was built for. Measured before the
/// correction: `nsh -i` read nothing, `nsh -i -o bash` read the drop-ins.
// [spec:nsh:req:interactive.vendor-path-is-not-for-scripts+1/test]
#[test]
fn the_default_dialect_reads_a_drop_in() {
    let (fixture, data_dirs) = trail_fixture();
    let output = fixture.run(&data_dirs, &[], &["-i"], "echo \"TRAIL:$TRAIL\"");

    assert!(
        output.stdout.contains("first"),
        "the plain interactive shell read no drop-in, so the feature is \
         unreachable by the shell it exists for: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// The four kinds of entry that turn a startup mechanism into a bug
/// report, in one directory with a file that must still be read.
// [spec:nsh:req:interactive.vendor-startup-path/test]
#[test]
fn only_a_regular_file_is_read() {
    let fixture = Fixture::new();
    let vendor = fixture.vendor("share");
    write_drop_in(&vendor, "10-regular.nsh", "TRAIL=\"$TRAIL regular\"\n");
    fs::create_dir(vendor.join("20-directory.nsh")).expect("a directory named like a drop-in");
    symlink("/nowhere/at/all", vendor.join("30-dangling.nsh")).expect("a dangling symlink");
    /* The masking idiom: a package's file stays installed and the
     * administrator points its name at a device instead. */
    symlink("/dev/null", vendor.join("40-masked.nsh")).expect("a symlink to a device");
    fs::write(fixture.root.join("linked.nsh"), "TRAIL=\"$TRAIL linked\"\n").expect("a real file");
    symlink(
        fixture.root.join("linked.nsh"),
        vendor.join("50-symlink.nsh"),
    )
    .expect("a symlink to a file");
    let data_dirs = fixture.data_dirs(&["share"]);

    let output = fixture.run(
        &data_dirs,
        &[],
        &["-i", "-o", "bash"],
        "echo \"TRAIL:$TRAIL\"",
    );

    assert!(
        output.stdout.contains("TRAIL: regular linked"),
        "a regular file or a symlink to one was skipped, or something that \
         is neither was read: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// A machine with no packages installed has no vendor directory, and a
/// shell that complained about that on every startup is a shell people
/// turn the mechanism off in.
// [spec:nsh:req:interactive.vendor-startup-path/test]
#[test]
fn a_directory_that_is_not_there_is_silent() {
    let fixture = Fixture::new();
    let data_dirs = fixture.data_dirs(&["never-created"]);

    let output = fixture.run(&data_dirs, &[], &["-i", "-o", "bash"], "echo ran");

    assert!(
        !output.stderr.contains("vendor_conf.d"),
        "an absent directory was reported: stderr {:?}",
        output.stderr
    );
    assert!(
        output.stdout.contains("ran"),
        "the shell did not get as far as its command: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// Unreadable through a symlink loop rather than through a mode, so that
/// the check measures the same thing when the suite runs as root -- where
/// no mode makes a directory unreadable and a permissions fixture would
/// quietly become a test of nothing.
// [spec:nsh:req:interactive.vendor-startup-path/test]
#[test]
fn an_unreadable_directory_is_reported() {
    let fixture = Fixture::new();
    let parent = fixture.root.join("share/nsh");
    fs::create_dir_all(&parent).expect("create the data directory");
    symlink("vendor_conf.d", parent.join("vendor_conf.d")).expect("a symlink to itself");
    let data_dirs = fixture.data_dirs(&["share"]);

    let output = fixture.run(&data_dirs, &[], &["-i", "-o", "bash"], "echo ran");

    assert!(
        output.stderr.contains("vendor_conf.d"),
        "a directory that exists and cannot be read was not reported: \
         stderr {:?}",
        output.stderr
    );
    assert!(
        output.stdout.contains("ran"),
        "reporting the directory ended the shell: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// `$ENV` is the user's own file and is read after both directories, which
/// is the same "local wins" ordering the two directories have between
/// them. The rules do not say so; this is where that decision is written
/// down in a form that fails when it changes.
// [spec:nsh:req:interactive.vendor-startup-path/test]
#[test]
fn the_user_file_is_read_last() {
    let fixture = Fixture::new();
    let vendor = fixture.vendor("share");
    write_drop_in(&vendor, "10-setting.nsh", "SETTING=packaged\n");
    let user_file = fixture.root.join("user.nsh");
    fs::write(&user_file, "echo \"ENV-SAW:$SETTING\"\nSETTING=user\n")
        .expect("write the user file");
    let data_dirs = fixture.data_dirs(&["share"]);

    let output = fixture.run(
        &data_dirs,
        &[("ENV", &user_file)],
        &["-i", "-o", "bash"],
        "echo \"SETTING:$SETTING\"",
    );

    assert!(
        output.stdout.contains("ENV-SAW:packaged"),
        "the user's file did not run after the drop-ins: stdout {:?}, \
         stderr {:?}",
        output.stdout,
        output.stderr
    );
    assert!(
        output.stdout.contains("SETTING:user"),
        "the user's file did not get the last word: stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}

/// A drop-in's name comes out of a directory listing, so expanding it
/// would let a package -- or anything that can write into one of these
/// directories -- run a command by choosing a filename.
// [spec:nsh:req:interactive.vendor-startup-path/test]
#[test]
fn a_drop_in_name_is_used_as_written() {
    let fixture = Fixture::new();
    let vendor = fixture.vendor("share");
    /* Relative, because a filename cannot hold a `/` and the shell runs
     * with the fixture root as its working directory. */
    let witness = fixture.root.join("expanded");
    write_drop_in(
        &vendor,
        "$(touch expanded) a b.nsh",
        "TRAIL=\"$TRAIL odd\"\n",
    );
    write_drop_in(&vendor, "$HOME.nsh", "TRAIL=\"$TRAIL home\"\n");
    let data_dirs = fixture.data_dirs(&["share"]);

    let output = fixture.run(
        &data_dirs,
        &[],
        &["-i", "-o", "bash"],
        "echo \"TRAIL:$TRAIL\"",
    );

    assert!(
        !witness.exists(),
        "a filename was expanded as shell text and ran a command"
    );
    assert!(
        output.stdout.contains("odd") && output.stdout.contains("home"),
        "a file whose name looks like shell text was not read as itself: \
         stdout {:?}, stderr {:?}",
        output.stdout,
        output.stderr
    );
}
