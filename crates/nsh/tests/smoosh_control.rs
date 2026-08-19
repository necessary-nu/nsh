//! Loop-control boundaries pinned by the Smoosh compatibility profile.

use nsh::{Shell, Streams};

fn run(script: &[u8]) -> (Vec<u8>, i32) {
    let mut shell = Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .build()
        .expect("build shell");
    let status = shell.run(script).expect("run script").code();
    let stdout = shell
        .take_captured_stdout()
        .expect("read captured stdout")
        .to_vec();
    (stdout, status.into())
}

// [spec:nsh:req:compat.smoosh.control-boundaries/test]
#[test]
fn dot_break_stays_in_dot_context() {
    let path = std::env::temp_dir().join(format!("nsh-dot-break-{}", std::process::id()));
    std::fs::write(&path, b"break\n").expect("write dot script");
    let script = format!(
        "for x in a b c; do echo $x; . {}; done",
        path.display()
    );

    let result = run(script.as_bytes());
    std::fs::remove_file(path).expect("remove dot script");

    assert_eq!(result, (b"a\nb\nc\n".to_vec(), 0));
}

// [spec:nsh:req:compat.smoosh.control-boundaries/test]
#[test]
fn subshell_break_stays_in_subshell() {
    let script = b"for x in a b; do (for y in c d; do break 2; done; echo $x); done";

    assert_eq!(run(script), (b"a\nb\n".to_vec(), 0));
}
