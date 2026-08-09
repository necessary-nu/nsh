# The libc surface, symbol by symbol

`docs/std-replacements.md` asks which *reimplementations* in this crate
the standard library already does — `mystring.rs`, `output.rs`'s
`vsnprintf`, the hash tables. This document asks the other half of the
question, which nothing has asked: **which direct `libc::` calls are
there because C had no alternative, and which are there because the port
was literal?**

The two are different work. A reimplementation is a file you delete. A
direct call is a line you rewrite, and there are 394 of them across 100
distinct symbols, spread over every module. They were invisible to the
plan because no node's title names them, which is how `libc::putenv`
survived into a crate that has `std::env` — and survived holding a
pointer into shell-owned storage that glibc does not copy.

Measured on master at the edition-2024 merge. Counts are call sites, not
mentions; comments and doc-references are excluded.

---

## 1. The summary

| Class | Symbols | Sites | What decides |
|---|---|---|---|
| A. `<[u8]>` / `bstr` has it exactly | 19 | 116 | the type system |
| B. `std` has it, with an argument | 12 | 39 | one local argument each |
| C. Trap — std has it and it differs | 8 | 18 | measurement, then the register |
| D. `std` does not have it | 61 | 221 | nothing; these stay |

**155 of 394 sites are A or B.** Class D is the majority and that is the
honest headline: this is a POSIX shell, most of what it does is
syscalls, and a shell that stopped calling `fork` would not be one.

---

## 2. Class A — the slice methods

These are `<[u8]>`, `bstr::ByteSlice` or `core::ptr` one-liners. Nothing
decides them but the compiler, once the operand is an owned value rather
than a `*mut c_char` — which is why they are *downstream of*
`[dec:nsh:owned-data]` and not independent of it. Converting the call
while the operand is still a raw pointer buys a cast, not a deletion.

| Symbol | Sites | Replacement |
|---|---|---|
| `strlen` | 39 | `CStr::count_bytes`, or the length the owned value already knows |
| `strchr` | 17 | `ByteSlice::find_byte` |
| `strcmp` | 15 | `==` on slices |
| `strcpy` | 7 | `copy_from_slice` |
| `strspn` | 5 | `iter().position(|b| !set.contains(b))` |
| `memmove` | 5 | `copy_within` |
| `memcpy` | 5 | `copy_from_slice` |
| `strpbrk` | 4 | `ByteSlice::find_byteset` |
| `strncmp` | 3 | slice compare on a prefix |
| `strcspn` | 3 | `ByteSlice::find_not_byteset` |
| `memset` | 3 | `fill` |
| `strstr` | 2 | `ByteSlice::find` |
| `stpcpy` | 2 | `copy_from_slice` plus the length |
| `strtok` | 2 | `split_str(b"/")` — see below, this one is a bug fix |
| `strdup` | 1 | `to_vec` |
| `strchrnul` | 1 | `find_byte(..).unwrap_or(len)` |
| `strcasecmp` | 1 | `eq_ignore_ascii_case` |
| `stpncpy` | 1 | `copy_from_slice` on a clamped range |
| `bsearch` | 1 | `binary_search_by` |

`strtok` is not a cleanup. It holds its parse position in a **libc
static**, so `cd.rs`'s `updatepwd` is non-reentrant against any other
`strtok` caller in the host process — a hazard that exists only because
this is becoming a library, and one that no amount of `move-state`
reaches, because the static is not ours.

Per file, so the work can be split without collisions:

```
expand.rs 21   exec.rs 12   bltin/test.rs 9   input.rs 8   histedit.rs 8
cd.rs 8        bltin/printf.rs 7   mystring.rs 6   miscbltin.rs 6
jobs.rs 5      var.rs 4     system.rs 3   shellmain.rs 3   parser.rs 3
output.rs 3    nodes.rs 3   memalloc.rs 3   trap.rs 2   options.rs 2
eval.rs 2      shell.rs 1   redir.rs 1   alias.rs 1
```

## 3. Class B — `std` has it, and one argument settles it

| Symbol | Sites | Replacement | The argument |
|---|---|---|---|
| `free` | 9 | ownership | `[dec:nsh:owned-data]`; these are the last raw frees |
| `malloc` / `realloc` | 2 | `Vec` | same |
| `__errno_location` | 18 | `io::Error::last_os_error()` | only where the value is *reported*; where it is *compared* (`EINTR`, `EBADF`) the raw int is clearer |
| `putenv` | 1 | `std::env::set_var` | `setenv` copies; `putenv` stores the caller's pointer, and glibc does not copy. This is a live use-after-free — see §5 |
| `getopt` | 1 | a parser that owns its state | `optind` is a libc global; same class as `strtok` |
| `getcwd` | 1 | `std::env::current_dir` | but see `docs/std-replacements.md` §5.3 before touching anything else in `cd.rs` |
| `chdir` | 1 | `std::env::set_current_dir` | process-global either way; std only saves the `CString` |
| `snprintf` | 2 | `write!` | `docs/std-replacements.md` §4.2 |
| `abort` | 5 | `process::abort` | identical |
| `fdopen`/`fclose`/`fileno`/`fputs` | 8 | the writer | rides `output-is-a-writer` |

## 4. Class C — the traps

std has these and **the behaviour differs**. Each is measured in
`docs/std-replacements.md` §5; none may be swapped without either
showing the difference unobservable or registering it under
`docs/divergences.md`.

| Symbol | Sites | Why not |
|---|---|---|
| `isalpha` `isalnum` `isspace` `isdigit` | 6 | locale-dependent; `is_ascii_*` is not the same function outside C/POSIX. §5.5, and §8 item 1 records that this one is *argued, not measured* — a single-byte locale and a two-line probe settles it |
| `strcoll` | 3 | locale collation; `<[u8]>::cmp` is not it, and `msort` is stable where `sort_unstable` is not. §5.2 |
| `atoi` | 5 | `atoi` cannot fail and saturates; `str::parse` errors. §5.8 |
| `strtod` | 1 | locale decimal point |
| `fnmatch` | 1 | `pmatch` is dash's own and the `glob` crate is further still. §5.4 |
| `strerror` | 8 | `error.rs:363-377` overrides `ENOENT`/`ENOTDIR` with dash's own strings before falling back. §5.9 |
| `strsignal` | 1 | no std equivalent |
| `stat64` / `lstat64` / `fstat64` | 19 | `fs::Metadata` does not expose everything `test` reads, and `test.rs` compares device and inode |

## 5. The one that is already a defect

```rust
unsafe fn changelocale(val: *const c_char) {
    libc::putenv(val as *mut c_char);
    libc::setlocale(libc::LC_ALL, ...);
}
```

`varfunc` passes `vp->text`, which since the `owned-vars` merge is a
`Box<[u8]>` the shell owns. glibc's `putenv` stores that pointer in
`environ` without copying. Reassign or `unset` any of `LC_ALL`,
`LC_COLLATE`, `LC_CTYPE`, `LC_NUMERIC` or `LANG` and the box drops —
`environ` keeps pointing at freed memory, and the next `setlocale`
reads it. On `unset` it is worse: the text becomes a bare name, and
`putenv` with no `=` fails `EINVAL`, so glibc does not even drop the
stale entry.

dash has this too. Under `[dec:nsh:we-own-the-defects]` that is not a
defence. `std::env::set_var` copies into glibc-owned storage, and
`exec.rs:127` builds its own `envp` and never reads `environ`, so
`setlocale` is the only consumer and cannot tell the difference.

## 6. What this changes about the plan

`docs/std-replacements.md` §3 lists libc calls as *riders* — "what to
delete while you are already in there". That framing is why 394 sites
have no owner: a rider is only done if someone is already in that file
for another reason, and no node's title sends anyone to `histedit.rs`
or `bltin/test.rs`.

Class A is not a rider. It is the second half of `[dec:nsh:owned-data]`
— the same conversion seen from the call site rather than from the
allocation — and it should be scheduled per file alongside the container
work, not after it.

Class D stays, and saying so is the deliverable. A reader who does not
find `fork`, `sigprocmask` and `tcsetpgrp` listed here as *deliberately
kept* will spend an afternoon proposing `std::process::Command`.
`docs/std-replacements.md` §4.9, §4.11 and §4.12 give the long form of
why each is the wrong shape.
