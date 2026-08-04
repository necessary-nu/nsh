//! Entry point. The real work is in `shellmain::main_fn`, which is the
//! literal port of `main()` in `src/main.c`.

fn main() {
    // The port implements C's `longjmp` as an unwind carrying a
    // `error::Longjmp` payload (see `eval::setjmp_catch`). Those unwinds
    // are ordinary control flow — every shell error, interrupt, `exit`
    // and `set -e` raise goes through one — but the default panic hook
    // prints a "thread 'main' panicked" banner each time it is *raised*,
    // whether or not it is caught. Filter those out and leave the hook's
    // normal behaviour intact for genuine bugs.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info
            .payload()
            .downcast_ref::<dash::error::Longjmp>()
            .is_some()
        {
            return;
        }
        default_hook(info);
    }));

    // C's `main(int argc, char **argv)` receives raw NUL-terminated byte
    // strings. An argument need not be valid UTF-8, and dash passes such
    // bytes through untouched — `dash -c $'x=\xff; echo $x'` prints the
    // byte. `std::env::args()` unwraps a UTF-8 conversion and panics on
    // any non-UTF-8 argument, so the port died with status 101 where the
    // C ran normally.
    //
    // These stay `Vec<u8>` rather than `String`: a `String` holding
    // non-UTF-8 bytes violates its own invariant, and building one with
    // `from_utf8_unchecked` would be undefined behaviour even though the
    // only thing done with it here is `as_bytes`.
    let argv: Vec<Vec<u8>> = std::env::args_os()
        .map(std::os::unix::ffi::OsStringExt::into_vec)
        .collect();
    dash::shellmain::main_fn(argv.len() as libc::c_int, argv);
}
