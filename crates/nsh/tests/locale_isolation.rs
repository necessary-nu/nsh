//! Black-box verification for each Shell's owned locale.

use bstr::{BStr, BString};
use nsh::{Shell, Streams};

const ISO: &str = "en_US.ISO-8859-1";

// A test that cannot reach its fixture says so rather than passing.
//
// Every assertion below separates a single-byte charmap from UTF-8, so
// without that locale there is nothing here to measure and the `return` this
// call replaced was a pass that meant nothing. It cannot be recovered from
// inside the process: glibc resolves every locale name under its own locale
// directory, an absolute one included, so a generated locale is reachable
// only through `LOCPATH` in the environment, and a test that exported it for
// itself would be mutating the very process environment the last test here
// asserts is unchanged. The fixture is a precondition of the run instead,
// built and named by `tests/build-locales.sh`.
// [dec:nsh:no-ambient-state]
fn require_single_byte_fixture() {
    if let Err(error) = nsh_platform::Locale::new(ISO.as_bytes(), &[]) {
        panic!(
            "{ISO} is required by this test and could not be opened: {error}\n\
             build it and name it to the run:\n\
             \x20   export LOCPATH=$(tests/build-locales.sh)"
        );
    }
}

// [spec:nsh:sem:shell-locale.selection/test]
#[test]
fn locale_precedence_is_posix() {
    require_single_byte_fixture();

    let mut category = Shell::builder()
        .env([("LC_CTYPE", ISO), ("LANG", "C")])
        .build()
        .unwrap();
    assert!(
        category
            .set_var(BStr::new(&[0xe9]), BStr::new(b"yes"))
            .is_ok()
    );

    let mut all = Shell::builder()
        .env([("LC_ALL", "C"), ("LC_CTYPE", ISO), ("LANG", ISO)])
        .build()
        .unwrap();
    assert!(all.set_var(BStr::new(&[0xe9]), BStr::new(b"no")).is_err());

    let mut empty_all = Shell::builder()
        .env([("LC_ALL", ""), ("LC_CTYPE", ISO), ("LANG", "C")])
        .build()
        .unwrap();
    assert!(
        empty_all
            .set_var(BStr::new(&[0xe9]), BStr::new(b"yes"))
            .is_ok()
    );

    let mut language = Shell::builder()
        .env([("LC_ALL", ""), ("LC_CTYPE", ""), ("LANG", ISO)])
        .build()
        .unwrap();
    assert!(
        language
            .set_var(BStr::new(&[0xe9]), BStr::new(b"yes"))
            .is_ok()
    );

    let mut irrelevant_invalid = Shell::builder()
        .env([
            ("LC_ALL", "C"),
            ("LC_CTYPE", "nsh-invalid-locale"),
            ("LANG", ISO),
        ])
        .build()
        .unwrap();
    assert!(
        irrelevant_invalid
            .set_var(BStr::new(&[0xe9]), BStr::new(b"no"))
            .is_err()
    );
}

// [spec:nsh:sem:shell-locale.invalid-selection/test]
#[test]
fn invalid_locale_retains_effective_state() {
    require_single_byte_fixture();

    let mut initial = Shell::builder()
        .env([("LC_ALL", "nsh-invalid-locale")])
        .build()
        .unwrap();
    assert!(
        initial
            .set_var(BStr::new(&[0xe9]), BStr::new(b"no"))
            .is_err()
    );

    let mut runtime = Shell::builder().env([("LC_ALL", ISO)]).build().unwrap();
    runtime
        .set_var(BStr::new(b"LC_ALL"), BStr::new(b"nsh-invalid-locale"))
        .unwrap();
    assert_eq!(
        runtime.var(BStr::new(b"LC_ALL")),
        Some(BStr::new(b"nsh-invalid-locale"))
    );
    assert!(
        runtime
            .set_var(BStr::new(&[0xe9]), BStr::new(b"yes"))
            .is_ok()
    );
}

// [spec:nsh:req:shell-locale.operation-binding/test]
#[test]
fn raw_names_follow_shell_locale() {
    require_single_byte_fixture();

    let mut shell = Shell::builder()
        .env([("LC_ALL", ISO)])
        .streams(Streams::capture().unwrap())
        .build()
        .unwrap();
    let script = BString::from(vec![
        b'x', 0xe9, b'=', b'6', b'\n', b':', b' ', b'$', b'(', b'(', b'x', 0xe9, b'+', b'=', b'1',
        b')', b')', b'\n', b'[', b' ', b'"', b'$', b'x', 0xe9, b'"', b' ', b'=', b' ', b'7', b' ',
        b']',
    ]);
    assert!(shell.run(script).unwrap().success());
    assert_eq!(shell.var(BStr::new(&[b'x', 0xe9])), Some(BStr::new(b"7")));
}

// [spec:nsh:req:shell-locale.instance-isolation/test]
#[test]
fn shell_locales_are_isolated() {
    require_single_byte_fixture();

    let mut before = nsh_platform::process_environment();
    before.sort();
    let mut c_shell = Shell::builder().env([("LC_ALL", "C")]).build().unwrap();
    let mut iso_shell = Shell::builder().env([("LC_ALL", ISO)]).build().unwrap();

    for _ in 0..8 {
        assert!(
            c_shell
                .set_var(BStr::new(&[0xe9]), BStr::new(b"no"))
                .is_err()
        );
        assert!(
            iso_shell
                .set_var(BStr::new(&[0xe9]), BStr::new(b"yes"))
                .is_ok()
        );
        iso_shell.unset_var(BStr::new(&[0xe9])).unwrap();
    }

    let c_worker = std::thread::spawn(move || {
        for _ in 0..128 {
            assert!(
                c_shell
                    .set_var(BStr::new(&[0xe9]), BStr::new(b"no"))
                    .is_err()
            );
        }
    });
    let iso_worker = std::thread::spawn(move || {
        for _ in 0..128 {
            iso_shell
                .set_var(BStr::new(&[0xe9]), BStr::new(b"yes"))
                .unwrap();
            iso_shell.unset_var(BStr::new(&[0xe9])).unwrap();
        }
    });
    c_worker.join().unwrap();
    iso_worker.join().unwrap();

    let mut after = nsh_platform::process_environment();
    after.sort();
    assert_eq!(after, before);
}
