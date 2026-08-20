use bstr::{BStr, ByteSlice as _};

use super::{Command, find_command};

fn set_bash(sh: &mut crate::context::Shell, on: bool) {
    crate::options::set_option_by_name(sh, BStr::new(b"bash"), on).unwrap();
    crate::options::options_changed(sh).unwrap();
}

fn find(sh: &mut crate::context::Shell, name: &BStr) -> Command {
    let path = crate::var::pathval(sh);
    let mut entry = Command::Unknown;
    let _ = find_command(sh, name, &mut entry, 0, path.as_bstr()).unwrap();
    entry
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn bash_only_lookup_is_per_shell() {
    let _guard = crate::testutil::lock();
    let mut bash = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let posix = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    set_bash(&mut bash, true);

    assert!(super::builtin(&bash, BStr::new(b"shopt")).is_some());
    assert!(super::builtin(&posix, BStr::new(b"shopt")).is_none());
    assert!(super::builtin(&bash, BStr::new(b"printf")).is_some());
    assert!(super::builtin(&posix, BStr::new(b"printf")).is_some());
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
// [spec:nsh:req:idiom.command-dispatch/test]
#[test]
fn notification_invalidates_builtin_cache() {
    let _guard = crate::testutil::lock();
    let mut sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let name = BStr::new(b"shopt");
    set_bash(&mut sh, true);

    assert!(matches!(find(&mut sh, name), Command::Builtin(_)));
    assert!(sh.commands.get(name).is_some());

    set_bash(&mut sh, false);
    assert!(sh.commands.get(name).is_none());
    assert!(matches!(find(&mut sh, name), Command::Unknown));
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn lookup_stamp_invalidates_cache() {
    let _guard = crate::testutil::lock();
    let mut sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let name = BStr::new(b"shopt");
    set_bash(&mut sh, true);
    assert!(matches!(find(&mut sh, name), Command::Builtin(_)));

    sh.options.set_flag(crate::options::bash, 0);
    assert!(matches!(find(&mut sh, name), Command::Unknown));
    assert!(sh.commands.get(name).is_none());
}
