use super::*;

/* The scanner's two complaints, as values. The corpus sees the bytes
 * on stderr; only this sees which of the two produced them, and that
 * the scan stopped rather than carried on with a half-applied set of
 * options. */

#[test]
fn an_unknown_letter_returns_its_complaint() {
    let _g = crate::test_support::lock();
    let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let shell = &mut owned;
    let args = [BStr::new("set"), BStr::new("-Q")];

    let error = options(shell, &args, 1).expect_err("-Q is not an option");

    assert_eq!(error.message().to_vec(), b"Illegal option -Q".to_vec());
    assert_eq!(error.status().code(), 2);
}

#[test]
fn an_unknown_name_returns_its_complaint() {
    let _g = crate::test_support::lock();
    let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let shell = &mut owned;
    let args = [BStr::new("set"), BStr::new("-o"), BStr::new("nosuchopt")];

    let error = options(shell, &args, 1).expect_err("-o nosuchopt is not an option");

    assert_eq!(
        error.message().to_vec(),
        b"Illegal option -o nosuchopt".to_vec()
    );
}

/// `Options` is `nextopt` with its state made local, so what it has to
/// agree with is the C's walk, edge for edge. These are the edges:
/// which words the scan consumes is what decides where the operands
/// start, and every builtin reads its operands from there.
fn scan<'a>(args: &'a [&'a BStr], optstring: &[u8]) -> (Vec<u8>, Vec<&'a BStr>) {
    let mut option_scan = Options::new(args);
    let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let shell = &mut owned_sh;
    let mut seen = Vec::new();
    /* `Ok(Some(c))` would end the scan silently on an error and make
     * a failure look like a short option list, so the error is taken
     * loudly: every option string these cases use accepts every
     * option they hand it. */
    while let Some(c) = option_scan
        .next(&mut shell.diagnostics(), optstring)
        .expect("the scan's cases never pass an option the string rejects")
    {
        seen.push(c);
    }
    (seen, option_scan.operands().to_vec())
}

fn words<'a>(raw: &'a [&'a [u8]]) -> Vec<&'a BStr> {
    raw.iter().map(|w| BStr::new(*w)).collect()
}

#[test]
fn non_option_word_stops_scan() {
    let args = words(&[b"jobs", b"%1", b"-l"]);
    let (seen, operands) = scan(&args, b"lp");
    assert!(seen.is_empty());
    assert_eq!(operands, words(&[b"%1", b"-l"]));
}

#[test]
fn options_cluster_within_one_word() {
    let args = words(&[b"jobs", b"-lp", b"%1"]);
    let (seen, operands) = scan(&args, b"lp");
    assert_eq!(seen, b"lp");
    assert_eq!(operands, words(&[b"%1"]));
}

#[test]
fn option_arg_from_same_word() {
    let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let shell = &mut owned_sh;
    let args = words(&[b"read", b"-pPROMPT", b"var"]);
    let mut option_scan = Options::new(&args);
    assert_eq!(
        option_scan.next(&mut shell.diagnostics(), b"p:r").unwrap(),
        Some(b'p')
    );
    assert_eq!(option_scan.arg(), BStr::new(b"PROMPT"));
    assert_eq!(
        option_scan.next(&mut shell.diagnostics(), b"p:r").unwrap(),
        None
    );
    assert_eq!(option_scan.operands(), words(&[b"var"]));
}

#[test]
fn option_arg_from_next_word() {
    let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let shell = &mut owned_sh;
    let args = words(&[b"read", b"-p", b"PROMPT", b"var"]);
    let mut option_scan = Options::new(&args);
    assert_eq!(
        option_scan.next(&mut shell.diagnostics(), b"p:r").unwrap(),
        Some(b'p')
    );
    assert_eq!(option_scan.arg(), BStr::new(b"PROMPT"));
    assert_eq!(
        option_scan.next(&mut shell.diagnostics(), b"p:r").unwrap(),
        None
    );
    assert_eq!(option_scan.operands(), words(&[b"var"]));
}

/// A `:` in the option string belongs to the option in front of it, so
/// the search for a letter has to step over one. `r` is reachable only
/// if it does.
#[test]
fn search_skips_arg_marker() {
    let args = words(&[b"read", b"-r", b"var"]);
    let (seen, operands) = scan(&args, b"p:r");
    assert_eq!(seen, b"r");
    assert_eq!(operands, words(&[b"var"]));
}

#[test]
fn double_dash_ends_scan_consumed() {
    let args = words(&[b"unalias", b"--", b"-a"]);
    let (seen, operands) = scan(&args, b"a");
    assert!(seen.is_empty());
    assert_eq!(operands, words(&[b"-a"]));
}

/// A lone `-` ends the scan like `--` does, but the C returns before
/// `argptr++`, so it stays an operand. `cd -` is the case that cares.
#[test]
fn lone_dash_ends_scan_unconsumed() {
    let args = words(&[b"cd", b"-"]);
    let (seen, operands) = scan(&args, b"LP");
    assert!(seen.is_empty());
    assert_eq!(operands, words(&[b"-"]));
}

#[test]
fn options_spread_over_words() {
    let args = words(&[b"jobs", b"-l", b"-p", b"%1", b"%2"]);
    let (seen, operands) = scan(&args, b"lp");
    assert_eq!(seen, b"lp");
    assert_eq!(operands, words(&[b"%1", b"%2"]));
}

#[test]
fn scan_to_end_leaves_no_operands() {
    let args = words(&[b"jobs", b"-l"]);
    let (seen, operands) = scan(&args, b"lp");
    assert_eq!(seen, b"l");
    assert!(operands.is_empty());
}

/// The empty option string is what a builtin that takes no options
/// passes: it accepts nothing and exists to eat a `--`.
/// What the `set` scan reports is where it stopped, which decides the
/// positional parameters.
fn scan_options(raw: &[&[u8]]) -> usize {
    let _guard = crate::test_support::lock();
    let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let shell = &mut owned;
    let args = words(raw);
    options(shell, &args, 0)
        .expect("these cases scan cleanly")
        .next
}

#[test]
fn scan_stops_at_the_first_operand() {
    let next = scan_options(&[b"-x", b"file", b"-y"]);
    assert_eq!(next, 1);
}

#[test]
fn scan_consumes_a_double_dash() {
    let next = scan_options(&[b"--", b"a"]);
    assert_eq!(next, 1);
}

/// A lone `-` ends the options and is consumed -- unlike the builtin
/// scan, where it stays an operand.
#[test]
fn scan_consumes_a_lone_dash() {
    let next = scan_options(&[b"-", b"a"]);
    assert_eq!(next, 1);
}

#[test]
fn minus_o_takes_next_word() {
    let next = scan_options(&[b"-o", b"noglob", b"rest"]);
    assert_eq!(next, 2);
}

#[test]
fn empty_word_is_not_an_option() {
    let next = scan_options(&[b"", b"-x"]);
    assert_eq!(next, 0);
}

#[test]
fn hashall_tracks_minus_and_plus_forms() {
    let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let enable = words(&[b"-h"]);
    options(&mut shell, &enable, 0).unwrap();
    assert!(shell.options.enabled(ShellOption::HashAll));

    let disable = words(&[b"+h"]);
    options(&mut shell, &disable, 0).unwrap();
    assert!(!shell.options.enabled(ShellOption::HashAll));
}

// [spec:nsh:req:compat.smoosh.nonlexical-control/test]
#[test]
fn nonlexical_control_tracks_long_option_forms() {
    let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let enable = words(&[b"-o", b"nonlexicalctrl"]);
    options(&mut shell, &enable, 0).unwrap();
    assert!(shell.options.enabled(ShellOption::NonLexicalControl));

    let disable = words(&[b"+o", b"nonlexicalctrl"]);
    options(&mut shell, &disable, 0).unwrap();
    assert!(!shell.options.enabled(ShellOption::NonLexicalControl));
}

#[test]
fn empty_optstring_eats_double_dash() {
    let args = words(&[b".", b"--", b"file"]);
    let (seen, operands) = scan(&args, b"");
    assert!(seen.is_empty());
    assert_eq!(operands, words(&[b"file"]));
}
