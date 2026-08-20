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
}
