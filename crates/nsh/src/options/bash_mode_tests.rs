use super::*;
use bstr::BStr;

// [spec:nsh:def:compat.bash.mode/test]
#[test]
fn bash_option_appends_without_letter() {
    assert_eq!(nonlexicalctrl, 19);
    assert_eq!(bash, 20);
    assert_eq!(bash, NOPTS - 1);
    assert_eq!(optnames[bash].to_bytes(), b"bash");
    assert_eq!(optletters[bash], 0);
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn bash_tracks_long_option_forms() {
    let mut sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let enable = [BStr::new(b"-o"), BStr::new(b"bash")];
    options(&mut sh, &enable, 0, false).unwrap();
    assert_eq!(sh.options.dialect(), Dialect::Bash);

    let disable = [BStr::new(b"+o"), BStr::new(b"bash")];
    options(&mut sh, &disable, 0, false).unwrap();
    assert_eq!(sh.options.dialect(), Dialect::Posix);
}
