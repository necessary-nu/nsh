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
