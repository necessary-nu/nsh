//! Literal port of `src/bltin/bltin.h` — the compatibility header for
//! the `bltin` sub-library — plus the module declarations for the builtins
//! that were imported from standalone BSD utilities.
//! Rules: `docs/spec/port/src/bltin/bltin.md`.
//!
//! `bltin.h` is a pile of `#define`s that remap the stdio-ish names those
//! sources use onto the shell's own output layer. C macros are textual, and
//! so are `macro_rules!` macros: the definitions below sit *before* the
//! `pub mod` items, which is what puts them in textual scope inside
//! `printf.rs`, `test.rs` and `times.rs` — exactly as `#include "bltin.h"`
//! does for the C files.
//!
//! We port the `SHELL`-defined, `USE_GLIBC_STDIO`-undefined configuration,
//! which is what a normal `dash` build uses.
//!
//! Cross-module signatures assumed by this shim (see the port report):
//!   * `crate::output::out1fmt!`, `crate::output::outfmt!` — variadic
//!     printf-alikes. Rust cannot *define* a C variadic function, so the
//!     natural rendering of `void out1fmt(const char *, ...)` in the Rust
//!     port is a macro; these aliases forward to it.
//!   * `crate::output::{out1, out2, output, outc, out1c, outstr, flushout}`
//!   * `crate::error::sh_error!`, `crate::error::sh_warnx!`
//!   * `crate::var::bltinlookup`, `crate::eval::commandname`
//!
//! Macro bodies are only type-checked when expanded, so the aliases that no
//! builtin in this crate happens to use cost nothing at build time.

// ---------------------------------------------------------------------
// src/bltin/bltin.h:50-67 — stdio names remapped onto the output layer.
// ---------------------------------------------------------------------

/// `#define stdout out1` (src/bltin/bltin.h:56)
macro_rules! stdout {
    () => {
        crate::output::out1
    };
}

/// `#define stderr out2` (src/bltin/bltin.h:57)
macro_rules! stderr {
    () => {
        crate::output::out2
    };
}

/// `#define printf out1fmt` (src/bltin/bltin.h:58)
macro_rules! printf {
    ($($arg:expr),* $(,)?) => {
        crate::output::out1fmt!($($arg),*)
    };
}

/// `#define putc(c, file) outc(c, file)` (src/bltin/bltin.h:59)
macro_rules! putc {
    ($c:expr, $file:expr) => {
        crate::output::outc($c as libc::c_int, $file)
    };
}

/// `#define putchar(c) out1c(c)` (src/bltin/bltin.h:60)
macro_rules! putchar {
    ($c:expr) => {
        crate::output::out1c($c as libc::c_int)
    };
}

/// `#define FILE struct output` (src/bltin/bltin.h:61)
pub type FILE = crate::output::output;

/// `#define fprintf outfmt` (src/bltin/bltin.h:62)
macro_rules! fprintf {
    ($file:expr, $($arg:expr),* $(,)?) => {
        crate::output::outfmt!($file, $($arg),*)
    };
}

/// `#define fputs outstr` (src/bltin/bltin.h:63)
macro_rules! fputs {
    ($s:expr, $file:expr) => {
        crate::output::outstr($s, $file)
    };
}

/// `#define fflush flushout` (src/bltin/bltin.h:64)
macro_rules! fflush {
    ($file:expr) => {
        crate::output::flushout($file)
    };
}

/// `#define fileno(f) ((f)->fd)` (src/bltin/bltin.h:65)
macro_rules! fileno {
    ($f:expr) => {
        (*$f).fd
    };
}

/// `#define ferror outerr` — `outerr(f)` is `(f)->flags`
/// (src/bltin/bltin.h:66, src/output.h:117)
macro_rules! ferror {
    ($f:expr) => {
        (*$f).flags
    };
}

/// `#define INITARGS(argv)` (src/bltin/bltin.h:68)
///
/// Empty in the `SHELL` build. The standalone build instead has
/// (src/bltin/bltin.h:83)
///
/// ```text
/// #define INITARGS(argv) if ((commandname = argv[0]) == NULL) \
///     {fputs("Argc is zero\n", stderr); exit(2);} else
/// ```
///
/// which captures `argv[0]` into `commandname` and aborts with
/// `"Argc is zero\n"` when there is none.
macro_rules! INITARGS {
    ($argv:expr) => {};
}

/// `#define error sh_error` (src/bltin/bltin.h:69)
macro_rules! error {
    ($($arg:expr),* $(,)?) => {
        crate::error::sh_error!($($arg),*)
    };
}

/// `#define warn sh_warn` (src/bltin/bltin.h:70)
///
/// Note `sh_warn` has no definition anywhere in the current dash tree; the
/// alias is vestigial and no builtin expands it.
macro_rules! warn {
    ($($arg:expr),* $(,)?) => {
        crate::error::sh_warn!($($arg),*)
    };
}

/// `#define warnx sh_warnx` (src/bltin/bltin.h:71)
macro_rules! warnx {
    ($($arg:expr),* $(,)?) => {
        crate::error::sh_warnx!($($arg),*)
    };
}

/// `#define exit sh_exit` (src/bltin/bltin.h:72)
///
/// Like `warn`, `sh_exit` has no definition in the current tree.
macro_rules! exit {
    ($($arg:expr),* $(,)?) => {
        crate::error::sh_exit!($($arg),*)
    };
}

/// `#define setprogname(s)` (src/bltin/bltin.h:73)
macro_rules! setprogname {
    ($s:expr) => {};
}

/// `#define getprogname() commandname` (src/bltin/bltin.h:74)
///
/// `commandname` is `extern const char *commandname;` at
/// src/bltin/bltin.h:89, defined in src/eval.c:76.
macro_rules! getprogname {
    () => {
        crate::eval::commandname
    };
}

/// `#define setlocate(l,s) 0` (src/bltin/bltin.h:75)
macro_rules! setlocate {
    ($l:expr, $s:expr) => {
        0
    };
}

/// `#define getenv(p) bltinlookup((p),0)` (src/bltin/bltin.h:77)
///
/// `bltinlookup` lost its second parameter long ago (src/var.h:179); the
/// two-argument spelling here is dead, as nothing in the sub-library calls
/// `getenv`.
macro_rules! getenv {
    ($p:expr) => {
        crate::var::bltinlookup($p)
    };
}

// ---------------------------------------------------------------------
// The sub-library itself.
// ---------------------------------------------------------------------

pub mod printf;
pub mod test;
pub mod times;

// src/bltin/bltin.h:86 — int echocmd(int, char **);
// Declared here, defined in printf.c.
// [spec:dash:def:bltin.echocmd-fn]
// [spec:dash:sem:bltin.echocmd-fn]
pub use self::printf::echocmd;
