use super::*;

// [spec:nsh:req:idiom.immutable-ast/test]
#[test]
fn parse_result_owns_here_document_bodies() {
    let mut shell = Shell::builder().build().unwrap();
    crate::input::set_input_string(&mut shell, BStr::new(b"cat <<A <<B\none\nA\ntwo\nB\n"));
    let tree = match parse_command(&mut shell, false).unwrap() {
        ParseResult::Tree(Some(tree)) => tree,
        ParseResult::Tree(None) => panic!("expected a command, found a blank parse unit"),
        ParseResult::Eof => panic!("expected a command, found EOF"),
    };
    let Node::Command(command) = tree else {
        panic!("expected a simple command");
    };
    let bodies: Vec<&BStr> = command
        .redirections
        .iter()
        .map(|redirection| match redirection {
            Redirection::HereDocument(document) => document.body.word.as_bstr(),
            _ => panic!("expected a here-document"),
        })
        .collect();

    assert_eq!(bodies, [BStr::new(b"one\n"), BStr::new(b"two\n")]);
}

// [spec:dash:sem:parser.findkwd-fn/test]
#[test]
fn findkwd_preserves_the_sorted_table_contract() {
    let mut previous: Option<&[u8]> = None;

    for &(bytes, kind) in &RESERVED_WORDS {
        if let Some(previous) = previous {
            assert!(previous < bytes, "reserved words must be strictly sorted");
        }
        previous = Some(bytes);

        assert_eq!(reserved_word(BStr::new(bytes)), Some(kind));

        let mut longer = bytes.to_vec();
        longer.push(b'x');
        assert_eq!(reserved_word(BStr::new(&longer)), None);
    }

    for missing in [b"".as_slice(), b"cas", b"integer", b"zebra"] {
        assert_eq!(reserved_word(BStr::new(missing)), None);
    }

    assert_eq!(reserved_word(BStr::new(&[0xff_u8])), None);
}

#[test]
fn top_level_terminators_error() {
    for source in [b"}\n".as_slice(), b"echo before; do echo after\n"] {
        let mut shell = Shell::builder().build().unwrap();
        crate::input::set_input_string(&mut shell, BStr::new(source));
        assert!(parse_command(&mut shell, false).is_err());
    }
}

#[test]
fn compound_eof_names_expected_token() {
    for (source, expected) in [
        (
            b"if true\n".as_slice(),
            b"Syntax error: end of file unexpected (expecting \"then\")".as_slice(),
        ),
        (
            b"if true; then\n".as_slice(),
            b"Syntax error: end of file unexpected (expecting \"fi\")",
        ),
        (
            b"while true\n".as_slice(),
            b"Syntax error: end of file unexpected (expecting \"do\")",
        ),
        (
            b"for value; do\n".as_slice(),
            b"Syntax error: end of file unexpected (expecting \"done\")",
        ),
        (
            b"(\n".as_slice(),
            b"Syntax error: end of file unexpected (expecting \")\")",
        ),
    ] {
        let mut shell = Shell::builder().build().unwrap();
        crate::input::set_input_string(&mut shell, BStr::new(source));
        let error = match parse_command(&mut shell, false) {
            Err(error) => error,
            Ok(_) => panic!("source should be incomplete: {source:?}"),
        };
        assert_eq!(error.message(), BStr::new(expected), "source: {source:?}");
    }
}

#[test]
fn legacy_backquote_trailing_slash_errors() {
    let mut shell = Shell::builder().build().unwrap();
    crate::input::set_input_string(&mut shell, BStr::new(b"echo `echo a\\"));

    let error = match parse_command(&mut shell, false) {
        Err(error) => error,
        Ok(_) => panic!("backquote should be incomplete"),
    };

    assert_eq!(
        error.message(),
        BStr::new(b"Syntax error: EOF in backquote substitution")
    );
}
