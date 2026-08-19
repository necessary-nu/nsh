#![cfg(windows)]

use nsh::Shell;
use nsh::streams::Streams;

#[test]
fn backslash_remains_shell_quoting_syntax() {
    let mut shell = Shell::builder()
        .streams(Streams::capture().unwrap())
        .build()
        .unwrap();
    let status = shell.run(br"printf '<%s>\n' C:\foo C:\\foo").unwrap();
    assert!(status.success());
    assert_eq!(
        shell.take_captured_stdout().unwrap(),
        br"<C:foo>
<C:\foo>
"
    );
}

#[test]
fn a_non_native_export_does_not_break_builtins() {
    let mut shell = Shell::builder()
        .env([(b"INVALID".as_slice(), [0xff].as_slice())])
        .streams(Streams::capture().unwrap())
        .build()
        .unwrap();
    let status = shell.run(b"echo still-internal").unwrap();
    assert!(status.success());
    assert_eq!(
        shell.take_captured_stdout().unwrap(),
        b"still-internal\n".as_slice()
    );
}
