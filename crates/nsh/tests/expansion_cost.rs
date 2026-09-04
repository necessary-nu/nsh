//! What an expansion is allowed to spend, counted rather than timed.
//!
//! Every assertion here reads `nsh_platform::locale_work()`, which counts
//! thread-locale selections and questions put to the C library about a
//! character. Nothing here reads a clock: a duration measured on a shared
//! machine is a verdict about whatever else is running, and the same
//! artifact in this repository read 1.16 s on a quiet machine and 25.27 s
//! an hour later under load.

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
