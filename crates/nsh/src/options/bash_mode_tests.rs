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
    let mut sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let enable = [BStr::new(b"-o"), BStr::new(b"bash")];
    options(&mut sh, &enable, 0).unwrap();
    assert_eq!(sh.options.dialect(), Dialect::Bash);

    let disable = [BStr::new(b"+o"), BStr::new(b"bash")];
    options(&mut sh, &disable, 0).unwrap();
    assert_eq!(sh.options.dialect(), Dialect::Posix);
}
