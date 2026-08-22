use bstr::{BStr, ByteSlice as _};

use super::{Command, CommandSearch, find_command};

fn set_bash(shell: &mut crate::context::Shell, on: bool) {
    // [spec:nsh:def:idiom.shell-options]
    crate::options::set_option_by_name(shell, BStr::new(b"bash"), on).unwrap();
    crate::options::options_changed(shell).unwrap();
}

fn find(shell: &mut crate::context::Shell, name: &BStr) -> Command {
    let path = crate::variables::path_value(shell);
    let mut entry = Command::Unknown;
    assert!(
        find_command(
            shell,
            name,
            &mut entry,
            CommandSearch::DEFAULT,
            path.as_bstr()
        )
        .unwrap()
        .status()
        .is_some()
    );
    entry
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn bash_only_lookup_is_per_shell() {
    let _guard = crate::test_support::lock();
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
    let _guard = crate::test_support::lock();
    let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let name = BStr::new(b"shopt");
    set_bash(&mut shell, true);

    assert!(matches!(find(&mut shell, name), Command::Builtin(_)));
    assert!(shell.commands.get(name).is_some());

    set_bash(&mut shell, false);
    assert!(shell.commands.get(name).is_none());
    assert!(matches!(find(&mut shell, name), Command::Unknown));
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn lookup_stamp_invalidates_cache() {
    let _guard = crate::test_support::lock();
    let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let name = BStr::new(b"shopt");
    set_bash(&mut shell, true);
    assert!(matches!(find(&mut shell, name), Command::Builtin(_)));

    shell.options.set(crate::options::ShellOption::Bash, false);
    assert!(matches!(find(&mut shell, name), Command::Unknown));
    assert!(shell.commands.get(name).is_none());
}
