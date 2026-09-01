use super::*;
use crate::test_support::lock;

// [spec:nsh:sem:shell-locale.selection/test]
#[test]
fn an_empty_locale_assignment_is_not_an_unset() {
    let _guard = lock();
    let mut shell = Shell::builder().env([("LC_ALL", "C")]).build().unwrap();
    set_bytes(
        &mut shell,
        BStr::new(b"LC_ALL"),
        Some(BStr::new(b"")),
        VariableAttributes::NONE,
    )
    .unwrap();
    assert_eq!(
        lookup_bytes(&mut shell, BStr::new(b"LC_ALL"))
            .as_ref()
            .map(|value| value.as_slice()),
        Some(&b""[..])
    );
    assert!(
        environment(&shell)
            .unwrap()
            .iter()
            .any(|(name, value)| name.to_shell_bytes() == b"LC_ALL" && value.is_empty())
    );

    unset_bytes(&mut shell, BStr::new(b"LC_ALL")).unwrap();
    assert_eq!(lookup_bytes(&mut shell, BStr::new(b"LC_ALL")), None);
    assert!(
        environment(&shell)
            .unwrap()
            .iter()
            .all(|(name, _)| name.to_shell_bytes() != b"LC_ALL")
    );
}

// [spec:dash:sem:var.lookupvar-fn/test]
#[test]
fn lineno_survives_a_shell_move() {
    let _guard = lock();
    let mut shell = Shell::new(crate::streams::Streams::INHERIT);
    initialize_variables(&mut shell);
    shell.variables.line_number = 41;
    assert_eq!(
        lookup_bytes(&mut shell, BStr::new(b"LINENO"))
            .as_ref()
            .map(|value| value.as_slice()),
        Some(b"41".as_slice())
    );
    let mut moved = shell;
    moved.variables.line_number = 42;
    assert_eq!(
        lookup_bytes(&mut moved, BStr::new(b"LINENO"))
            .as_ref()
            .map(|value| value.as_slice()),
        Some(b"42".as_slice())
    );
}

// [spec:dash:sem:var.setvar-fn/test]
// [spec:posix:req:builtin.getopts.env-optind/test]
#[test]
fn set_and_unset_variable() {
    let _guard = lock();
    let mut shell = Shell::new(crate::streams::Streams::INHERIT);
    set_bytes(
        &mut shell,
        BStr::new(b"Tsetvar"),
        Some(BStr::new(b"hello")),
        VariableAttributes::NONE,
    )
    .unwrap();
    assert_eq!(
        lookup_bytes(&mut shell, BStr::new(b"Tsetvar"))
            .as_ref()
            .map(|value| value.as_slice()),
        Some(b"hello".as_slice())
    );
    unset_bytes(&mut shell, BStr::new(b"Tsetvar")).unwrap();
    assert_eq!(lookup_bytes(&mut shell, BStr::new(b"Tsetvar")), None);

    initialize_variables(&mut shell);
    shell.options.positional_parameters.option_index = 7;
    shell.options.positional_parameters.option_offset = Some(3);

    set_integer_bytes(
        &mut shell,
        BStr::new(b"OPTIND"),
        8,
        VariableAttributes::NONE,
        CallbackPolicy::Suppress,
    )
    .unwrap();
    assert_eq!(shell.options.positional_parameters.option_index, 7);
    assert_eq!(shell.options.positional_parameters.option_offset, Some(3));

    set_bytes(
        &mut shell,
        BStr::new(b"OPTIND"),
        Some(BStr::new(b"1")),
        VariableAttributes::NONE,
    )
    .unwrap();
    assert_eq!(shell.options.positional_parameters.option_index, 1);
    assert_eq!(shell.options.positional_parameters.option_offset, None);
    assert_eq!(
        variable_attributes(&shell, BStr::new(b"OPTIND")),
        Some(VariableAttributes::FIXED),
    );
}

// [spec:dash:sem:var.poplocalvars-fn/test]
#[test]
fn a_frame_restores_in_reverse_order() {
    let _guard = lock();
    let mut shell = Shell::new(crate::streams::Streams::INHERIT);
    set_bytes(
        &mut shell,
        BStr::new(b"Tframe"),
        Some(BStr::new(b"one")),
        VariableAttributes::NONE,
    )
    .unwrap();
    let stop = push_local_scope(&mut shell, true);
    make_local_bytes(
        &mut shell,
        BStr::new(b"Tframe=two"),
        VariableAttributes::NONE,
    )
    .unwrap();
    make_local_bytes(
        &mut shell,
        BStr::new(b"Tframe=three"),
        VariableAttributes::NONE,
    )
    .unwrap();
    assert_eq!(
        lookup_bytes(&mut shell, BStr::new(b"Tframe"))
            .as_ref()
            .map(|value| value.as_slice()),
        Some(b"three".as_slice())
    );
    unwind_local_scopes(&mut shell, stop);
    assert_eq!(
        lookup_bytes(&mut shell, BStr::new(b"Tframe"))
            .as_ref()
            .map(|value| value.as_slice()),
        Some(b"one".as_slice())
    );
}

#[test]
fn environment_is_owned_and_sorted() {
    let _guard = lock();
    let mut shell = Shell::new(crate::streams::Streams::INHERIT);
    set_bytes(
        &mut shell,
        BStr::new(b"ZED"),
        Some(BStr::new(b"z")),
        VariableAttributes::EXPORTED,
    )
    .unwrap();
    set_bytes(
        &mut shell,
        BStr::new(b"ALPHA"),
        Some(BStr::new(b"a")),
        VariableAttributes::EXPORTED,
    )
    .unwrap();
    let environment: Vec<(Vec<u8>, Vec<u8>)> = environment(&shell)
        .unwrap()
        .iter()
        .map(|(name, value)| (name.to_shell_bytes(), value.to_shell_bytes()))
        .collect();
    assert_eq!(
        environment,
        [
            (b"ALPHA".to_vec(), b"a".to_vec()),
            (b"ZED".to_vec(), b"z".to_vec()),
        ]
    );
}
