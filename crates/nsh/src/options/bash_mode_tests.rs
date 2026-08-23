use super::*;
use bstr::BStr;

// [spec:nsh:def:idiom.shell-options]
// [spec:nsh:def:compat.bash.mode/test]
#[test]
fn bash_metadata_has_no_letter() {
    let spec = OPTION_SPECS
        .iter()
        .find(|spec| spec.option == ShellOption::Bash)
        .expect("Bash has declarative option metadata");
    assert_eq!(spec.name, b"bash");
    assert_eq!(spec.letter, None);
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn bash_tracks_long_option_forms() {
    let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let enable = [BStr::new(b"-o"), BStr::new(b"bash")];
    options(&mut shell, &enable, 0).unwrap();
    assert_eq!(shell.options.dialect(), Dialect::Bash);

    let disable = [BStr::new(b"+o"), BStr::new(b"bash")];
    options(&mut shell, &disable, 0).unwrap();
    assert_eq!(shell.options.dialect(), Dialect::Posix);
}

/// `posix` is the dialect option inverted, not Bash's partial POSIX mode.
// [spec:nsh:req:compat.bash.posix-option/test]
#[test]
fn the_posix_option_inverts_the_dialect() {
    let _g = crate::test_support::lock();
    let shell = &mut Shell::new(crate::streams::Streams::INHERIT);

    crate::options::set_option_by_name(shell, BStr::new(b"bash"), true).unwrap();
    assert_eq!(shell.options.dialect(), Dialect::Bash);

    crate::options::set_option_by_name(shell, BStr::new(b"posix"), true).unwrap();
    assert_eq!(
        shell.options.dialect(),
        Dialect::Posix,
        "`set -o posix` ends the dialect rather than trimming it"
    );

    crate::options::set_option_by_name(shell, BStr::new(b"posix"), false).unwrap();
    assert_eq!(shell.options.dialect(), Dialect::Bash);
}

/// A listing speaks the dialect it is made in: Bash has no option called
/// `bash`, so a script in Bash mode must never be shown one.
// [spec:nsh:req:compat.bash.posix-option/test]
#[test]
fn a_listing_names_the_dialect_switch_locally() {
    let _g = crate::test_support::lock();
    let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
    let spec = OPTION_SPECS
        .into_iter()
        .find(|spec| spec.name == b"bash")
        .expect("the dialect option is in the table");

    assert_eq!(
        crate::options::presented_option(spec, Dialect::Posix, &shell.options),
        (b"bash".as_slice(), false)
    );

    crate::options::set_option_by_name(shell, BStr::new(b"bash"), true).unwrap();
    assert_eq!(
        crate::options::presented_option(spec, Dialect::Bash, &shell.options),
        (b"posix".as_slice(), false),
        "in Bash mode the switch is `posix`, and it is off"
    );
}
