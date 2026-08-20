//! Exact behavioral witnesses for the explicitly adopted Smoosh extensions.

use nsh::streams::Streams;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_CASE: AtomicU64 = AtomicU64::new(0);

fn run(script: &str, interactive: bool) -> (Vec<u8>, Vec<u8>, i32) {
    let directory = std::env::temp_dir().join(format!(
        "nsh-smoosh-extension-{}-{}",
        std::process::id(),
        NEXT_CASE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("create isolated case directory");
    let script_path: PathBuf = directory.join("case.sh");
    std::fs::write(&script_path, script).expect("write case script");

    let (stdout_read, stdout_write) = nsh_platform::pipe().expect("create stdout pipe");
    let (stderr_read, stderr_write) = nsh_platform::pipe().expect("create stderr pipe");
    let startup = nsh::Startup::script(script_path.as_os_str().as_encoded_bytes().to_vec());

    let status = nsh_platform::run_in_child(move || {
        std::env::set_current_dir(directory).expect("enter isolated case directory");
        let supplied = Streams::from_fds(std::io::stdin(), &stdout_write, &stderr_write)
            .expect("duplicate test streams");
        let mut builder = nsh::Shell::builder()
            .arg0(bstr::BStr::new(b"smoosh"))
            .inherit_env()
            .streams(supplied)
            .host(nsh::ProcessHost);
        if interactive {
            builder = builder
                .shell_option(nsh::ShellOption::Interactive, true)
                .shell_option(nsh::ShellOption::Monitor, true);
        }
        let mut shell = builder.build().expect("build process shell");
        let status = shell.run_to_completion(startup);
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child");

    let stdout = nsh_platform::read_to_end(&stdout_read).expect("read stdout");
    let stderr = nsh_platform::read_to_end(&stderr_read).expect("read stderr");
    (stdout, stderr, status)
}

// [spec:nsh:req:compat.smoosh.nonlexical-control/test]
#[test]
fn nonlexical_break_crosses_function() {
    let script = "set -o nonlexicalctrl\n\
                  brk() { break 5; echo post; }\n\
                  i=0; while [ $i -lt 5 ]; do echo $i; brk; : $((i+=1)); done";
    let (stdout, _stderr, status) = run(script, false);

    assert_eq!(stdout, b"0\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.nonlexical-control/test]
#[test]
fn nonlexical_continue_crosses_function() {
    let script = "set -o nonlexicalctrl\n\
                  cnt() { continue; echo post; }\n\
                  i=0; while [ $i -lt 5 ]; do echo $i; : $((i+=1)); cnt; echo after; done";
    let (stdout, _stderr, status) = run(script, false);

    assert_eq!(stdout, b"0\n1\n2\n3\n4\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.history-builtin/test]
#[test]
fn history_clear_and_nolog() {
    let script = "history | grep history >/dev/null || exit 1\n\
                  echo hi >/dev/null\n\
                  history | grep echo >/dev/null || exit 2\n\
                  history -c\n\
                  history >hist\n\
                  grep echo >/dev/null hist && exit 3\n\
                  set -o nolog\n\
                  history -c\n\
                  echo hello >/dev/null\n\
                  history >hist2\n\
                  grep echo >/dev/null hist2 && exit 4\n\
                  echo ok\n";
    let (stdout, _stderr, status) = run(script, true);

    assert_eq!(stdout, b"ok\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.source-builtin/test]
#[test]
fn missing_source_is_fatal() {
    let (stdout, stderr, status) = run("source nonesuch\n. nonesuch\n", false);

    assert!(stdout.is_empty());
    assert_eq!(stderr, b"source: nonesuch: not found\n");
    assert_eq!(status, 1);
}

// [spec:nsh:req:compat.smoosh.source-builtin/test]
#[test]
fn source_assignments_persist() {
    let script = "echo 'x=5' >to_source\n\
                  source ./to_source\n\
                  echo ${x?:unset}\n\
                  rm to_source\n\
                  [ \"$x\" -eq 5 ]\n";
    let (stdout, stderr, status) = run(script, false);

    assert_eq!(stdout, b"5\n");
    assert!(stderr.is_empty());
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.hash-all/test]
#[test]
fn hashall_scans_function_body() {
    let script = "set -h\n\
                  hash -r\n\
                  f() {\n\
                      ls\n\
                      touch hi\n\
                      rm hi\n\
                  }\n\
                  hash\n\
                  hash | grep ls || exit 1\n\
                  hash | grep touch || exit 2\n\
                  hash | grep rm || exit 3\n";
    let (stdout, stderr, status) = run(script, false);

    assert!(stdout.windows(2).any(|bytes| bytes == b"ls"));
    assert!(stdout.windows(5).any(|bytes| bytes == b"touch"));
    assert!(stdout.windows(2).any(|bytes| bytes == b"rm"));
    assert!(stderr.is_empty());
    assert_eq!(status, 0);
}
