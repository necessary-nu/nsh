use bstr::{BStr, BString};
use nsh::Shell;

fn bytes(values: &[&[u8]]) -> Vec<BString> {
    values.iter().map(|value| BString::from(*value)).collect()
}

#[test]
// [spec:nsh:sem:idiom.typed-expansion/test]
fn structural_fields() {
    let mut shell = Shell::builder().build().unwrap();
    shell.run(b"set -- '' 'a b' c; value='left right'").unwrap();

    assert_eq!(
        shell.expand_word(BStr::new(b"$value")).unwrap(),
        bytes(&[b"left", b"right"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"\"$value\"")).unwrap(),
        bytes(&[b"left right"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"\"$@\"")).unwrap(),
        bytes(&[b"", b"a b", b"c"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"pre\"$@\"post")).unwrap(),
        bytes(&[b"pre", b"a b", b"cpost"]),
    );
}

#[test]
fn parameter_operand_quotes() {
    let mut shell = Shell::builder().build().unwrap();
    shell.run(b"value=abc; pattern='*b'").unwrap();

    assert_eq!(
        shell
            .expand_word(BStr::new(b"\"${value#\"$pattern\"}\""))
            .unwrap(),
        bytes(&[b"abc"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"${missing:-a\\ b}")).unwrap(),
        bytes(&[b"a b"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"${assigned:=a\\*b}")).unwrap(),
        bytes(&[b"a*b"]),
    );
}

#[test]
fn assignment_tilde_prefix() {
    let mut shell = Shell::builder().build().unwrap();
    shell.run(b"HOME=/home/example").unwrap();

    assert_eq!(
        shell.expand_word(BStr::new(b"~:tail")).unwrap(),
        bytes(&[b"~:tail"]),
    );
    shell.run(b"path=~:/bin; ordinary=x=~").unwrap();
    assert_eq!(
        shell.expand_word(BStr::new(b"$path")).unwrap(),
        bytes(&[b"/home/example:/bin"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"$ordinary")).unwrap(),
        bytes(&[b"x=~"]),
    );

    shell.run(b"path=~:${missing-~:~}").unwrap();
    assert_eq!(
        shell.expand_word(BStr::new(b"$path")).unwrap(),
        bytes(&[b"/home/example:/home/example:/home/example"]),
    );
}

#[test]
fn nested_arithmetic_closes_correctly() {
    let mut shell = Shell::builder().build().unwrap();

    assert_eq!(
        shell
            .expand_word(BStr::new(b"$((1 + $((2 + 3)) + 4))"))
            .unwrap(),
        bytes(&[b"10"]),
    );
}

#[test]
fn empty_field_boundaries_survive() {
    let mut shell = Shell::builder().build().unwrap();
    shell.run(b"set -- one '' two; IFS=x").unwrap();

    assert_eq!(
        shell.expand_word(BStr::new(b"$@")).unwrap(),
        bytes(&[b"one", b"", b"two"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"\"$@\"")).unwrap(),
        bytes(&[b"one", b"", b"two"]),
    );

    shell.run(b"set -- '' '' ''; IFS=x").unwrap();
    assert_eq!(
        shell.expand_word(BStr::new(b"$*")).unwrap(),
        bytes(&[b"", b""]),
    );
    shell.run(b"unset IFS").unwrap();
    assert!(shell.expand_word(BStr::new(b"$*")).unwrap().is_empty());
    shell.run(b"set -- '1 2' '3  4'; IFS=").unwrap();
    assert_eq!(
        shell.expand_word(BStr::new(b"$*")).unwrap(),
        bytes(&[b"1 2", b"3  4"]),
    );

    shell.run(b"unset IFS; value='   abc   def   '").unwrap();
    assert_eq!(
        shell.expand_word(BStr::new(b"\"\"$value\"\"")).unwrap(),
        bytes(&[b"", b"abc", b"def", b""]),
    );
}

#[test]
fn assignment_operands_reexpand_scalar() {
    let mut shell = Shell::builder().build().unwrap();
    shell.run(b"set -- '1 2' '3 4'").unwrap();

    assert_eq!(
        shell.expand_word(BStr::new(b"${unset=x\"$@\"x}")).unwrap(),
        bytes(&[b"x1", b"2", b"3", b"4x"]),
    );
    assert_eq!(
        shell.expand_word(BStr::new(b"$unset")).unwrap(),
        bytes(&[b"x1", b"2", b"3", b"4x"]),
    );
}
