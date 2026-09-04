//! What an expansion is allowed to spend, counted rather than timed.
//!
//! Every assertion here reads `nsh_platform::locale_work()`, which counts
//! thread-locale selections and questions put to the C library about a
//! character. Nothing here reads a clock: a duration measured on a shared
//! machine is a verdict about whatever else is running, and the same
//! artifact in this repository read 1.16 s on a quiet machine and 25.27 s
//! an hour later under load.
// [spec:nsh:req:cost.asserted-as-work/test]

use nsh::{Shell, Streams};
use nsh_platform::LocaleWork;

/// A UTF-8 charmap, by whichever of its names this host answers to.
///
/// Every count below separates characters from bytes, so a host that
/// gave back a single-byte charmap here would measure nothing and pass.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
fn utf8_name() -> &'static str {
    ["C.UTF-8", "C.utf8", "en_US.UTF-8"]
        .into_iter()
        .find(|name| nsh_platform::Locale::new(name.as_bytes(), &[]).is_ok())
        .expect(
            "no UTF-8 charmap: tried C.UTF-8, C.utf8 and en_US.UTF-8\n\
             build one and name it to the run:\n\
             \x20   export LOCPATH=$(tests/build-locales.sh)",
        )
}

fn shell_holding(value: Vec<u8>) -> Shell {
    Shell::builder()
        .env([
            (b"LC_ALL".to_vec(), utf8_name().as_bytes().to_vec()),
            (b"S".to_vec(), value),
        ])
        .streams(Streams::capture().unwrap())
        .build()
        .unwrap()
}

/// What one more run of `script` costs a shell that has already run it.
///
/// The first run is not measured because it is the one that fills
/// whatever a shell caches once -- the locale's byte-class table among
/// them -- and a cost rule is about what a command spends every time,
/// not about what the first one pays for the rest.
fn one_more_run(shell: &mut Shell, script: &[u8]) -> LocaleWork {
    shell.run(script).unwrap();
    let before = nsh_platform::locale_work();
    shell.run(script).unwrap();
    nsh_platform::locale_work().since(before)
}

/// `${#S}` asks the locale once per character of `S`, not once per byte.
///
/// Two values, three hundred characters each, one of them sixty bytes
/// longer because sixty of its characters take two bytes. The interior
/// byte of a two-byte character is a position no caller can arrive at --
/// every walk in the tree steps from one boundary to the next -- so
/// asking the C library what begins there is work whose result is
/// discarded, and the two values must cost the same.
///
/// Measured before the walk stepped: 360 against 300 per expansion, and
/// 7,200,000 against 6,000,000 over twenty thousand of them.
// [spec:nsh:req:cost.only-the-work-the-command-needs/test]
#[test]
fn a_length_asks_per_character_not_per_byte() {
    const CHARACTERS: u64 = 300;
    /* U+00CC sixty times, then two hundred and forty ASCII: three
     * hundred characters in three hundred and sixty bytes. */
    let wide: Vec<u8> = std::iter::repeat_n([0xc3_u8, 0x8c], 60)
        .flatten()
        .chain(std::iter::repeat_n(b'a', 240))
        .collect();
    let narrow: Vec<u8> = std::iter::repeat_n(b'a', 300).collect();
    assert_eq!((wide.len(), narrow.len()), (360, 300));

    let wide = one_more_run(&mut shell_holding(wide), b"n=${#S}");
    let narrow = one_more_run(&mut shell_holding(narrow), b"n=${#S}");

    assert_eq!(
        (wide.character_queries, narrow.character_queries),
        (CHARACTERS, CHARACTERS),
        "a 360-byte value of 300 characters must cost what a 300-byte one does"
    );
    /* The whole walk under one selection is the other half of the same
     * expansion's cost, and asserting it here is what stops a fix for
     * the count above being bought with a selection per character. */
    assert_eq!((wide.selections, narrow.selections), (1, 1));
}

/// A value the locale cannot read costs no more than one it can.
///
/// A byte that begins nothing is stepped over one byte at a time, so a
/// string of them is as many questions as it has bytes -- which is the
/// same one-per-character bound, since each such byte is its own
/// character. This is the case where the stepping fill has nothing to
/// skip, and it is here so that "one per character" is measured at both
/// ends of the range rather than only where it saves something.
// [spec:nsh:req:cost.only-the-work-the-command-needs/test]
#[test]
fn bytes_that_begin_nothing_cost_one_question_each() {
    let invalid: Vec<u8> = std::iter::repeat_n(0x8c_u8, 100).collect();
    let work = one_more_run(&mut shell_holding(invalid), b"n=${#S}");
    assert_eq!(work.character_queries, 100);
}

/// A shell holding `x y z`, with `IFS` assigned and the split proved to
/// work before anything is counted.
///
/// The proof matters: two shells that both split into nothing would cost
/// the same too, and the check below would pass on a shell that had
/// stopped splitting.
fn splitting_three_fields(assignment: &[u8]) -> Shell {
    let mut shell = shell_holding(b"x y z".to_vec());
    shell.run(assignment).unwrap();
    shell.run(b"set -- $S; echo $#").unwrap();
    let said = shell.take_captured_stdout().unwrap();
    assert_eq!(said.trim_ascii_end(), b"3", "the split has to have split");
    shell
}

/// Splitting a field does not read `IFS` again.
///
/// Two shells differing only in the length of `IFS`, splitting the same
/// value into the same three fields. What `IFS` names -- where each of
/// its characters ends, and whether the locale calls it a space -- is
/// derived from `IFS` and the locale and can change only when one of
/// them is assigned, so a split that derived it again would cost three
/// thread-locale selections per character of `IFS` on every field it
/// touched.
///
/// The second half is the other side of the same claim, and it is what
/// stops the first half being satisfied by a shell that has simply
/// stopped asking: reading `IFS` still costs what it always did, at the
/// assignment, where it is paid once.
///
/// Measured before the table moved to the assignment, `LC_ALL=C.UTF-8`,
/// load average 17.3/25.0/26.4 on 32 cores: one `set -- $s` over a
/// 360-byte value cost 34 thread-locale selections against 7 after, and
/// a `while test ...` loop of 20,000 iterations cost 760,037 against
/// 40,001.
// [spec:nsh:req:cost.asserted-as-work/test]
#[test]
fn a_split_does_not_rebuild_what_ifs_says() {
    /* Nine characters that do not occur in the value, so both shells
     * split it into the same three fields and the only difference
     * between them is how much of `IFS` there is to read. */
    const SHORT: &[u8] = b"IFS=' \t\n'";
    const LONG: &[u8] = b"IFS=' \t\n:;,.!?+=-'";

    let short = one_more_run(&mut splitting_three_fields(SHORT), b"set -- $S");
    let long = one_more_run(&mut splitting_three_fields(LONG), b"set -- $S");
    assert_eq!(
        short, long,
        "a split must cost the same whatever length IFS is"
    );

    let assigning_short = one_more_run(&mut shell_holding(b"x y z".to_vec()), SHORT);
    let assigning_long = one_more_run(&mut shell_holding(b"x y z".to_vec()), LONG);
    assert!(
        assigning_long.selections > assigning_short.selections,
        "assigning IFS is where its characters are read: {assigning_long:?} against {assigning_short:?}"
    );
}
