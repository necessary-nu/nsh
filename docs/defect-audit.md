# Dash defect disposition audit

`nsh` uses Dash as a differential oracle, not as the authority for observable
shell behavior. POSIX comes first, then explicit nsh behavior, then sanctioned
divergences. This audit records the translated defects that previously carried
an instruction to reproduce C behavior even when that behavior was invalid or
undefined.

## Live behavior corrected

| Area | Inherited defect | nsh disposition | Authority and evidence |
|---|---|---|---|
| Input frames | Popping a nested input frame can walk the outer frame's string stack, dereference a null link, and leave an alias permanently marked in use. | The dying frame owns and releases all of its active and deferred overlays before it is dropped. | Owned-state invariant; `input::tests::popped_frame_releases_aliases`. |
| `fc -s` operand parsing | The reference writes a NUL into the expanded `old=new` argument while splitting it and thereby exposes an incidental mutation through `_`. | Substitution parsing borrows the length-delimited operand and leaves the evaluator's argument intact. | POSIX specifies the substitution, not mutation of command-argument storage; `fc::tests::substitution_operand_stays_intact`. |
| `jobs` command text | An assignment-only job reads an uninitialized stack buffer as an empty command; a `case` command prints only its first pattern. | Structural command rendering includes assignments, wrapper redirections, and every case pattern. | POSIX `jobs` requires the `<command>` field to be the associated command; focused renderer tests cover both inherited omissions. |
| `ulimit` | Decimal accumulation and unit multiplication wrap an unsigned resource-limit value; the reference's negative-value overflow test is dead. | Both operations use checked arithmetic and report `bad number` outside the supported `u64` range. | POSIX requires numerals through the implementation's maximum supported value to be recognized; the boundary and both overflow paths are tested. |

## Undefined behavior replaced by deterministic semantics

| Area | Reference behavior | nsh disposition |
|---|---|---|
| Expansion syntax selection | `memtodest` can index before `is_type`; the observed byte depends on binary layout. | `DestinationSyntax::Unframed` represents the only stable meaning, “escape nothing”, without an array lookup. |
| IFS NUL-only splitting | The reference can consult an uninitialized `ifs0` value when the separator is NUL. | Absence of a first IFS character is `None`; NUL is not classified as IFS whitespace. |
| Arithmetic | Signed overflow and invalid C shift counts are undefined. | Shell arithmetic is deterministic wrapping `i64`; shift counts are masked modulo 64. Division by zero and `MIN / -1` remain explicit errors. |
| Prompt expansion | Reassigning `PS1` during its own expansion can leave the reference's pointer dangling on the error path. | The parser owns a copy for the complete expansion. |
| Background wrapper line | Dash can read an uninitialized line number after a fork failure. | The wrapper stores the source command's captured line number. This remains registered as a sanctioned Rust-side correction. |

## Invalid internal states made impossible

The following release-build fallthroughs remain useful facts about the C
reference, but are not executable states in the Rust core:

- AST evaluation and job rendering exhaustively match the structural `Node`
  enum; there is no unknown union tag to reinterpret as negation or a pipeline.
- Command lookup exhaustively matches `Command`; there is no unknown command
  tag to reinterpret as an external executable.
- Job ordering exposes separate `position_running`, `position_stopped`, and
  removal operations; no numeric mode can fall through to deletion.
- Builtin option scanners reject unsupported options before typed dispatch;
  unknown `kill` or `command` option letters cannot fall through to another
  arm.
- `RedirectionMode` is an enum. The reference's overlapping `REDIR_SAVEFD2`
  bits cannot manufacture a null saved-frame access.
- Arithmetic operators are an enum. An unknown operator cannot fall through
  to division.

## Ownership defects removed

- Clearing traps moves or drops an owned `TrapAction`; it cannot free a null
  slot while leaking the previous action.
- Variable assignment and removal return `Result`, not a pointer that may
  already have been freed.
- Command text starts as an initialized `BString`, never an uninitialized stack
  allocation.
- The shell-stack allocator and its narrowing length conversions do not exist
  in the Rust core.

## Dead compatibility machinery omitted

The never-installed `expmeta` exception handler, DEBUG-only tracing module,
generated C configuration branches, and release-only default fallthrough arms
have no Rust counterparts. Omitting them does not add a replacement behavior;
ordinary resource ownership and typed dispatch cover the reachable operations.

## Sanctioned improvements

The native line editor passes the POSIX history goto-number and anchored-search
cases that Dash's libedit fails. Those results are accepted improvements, not
parity failures and not candidates for deliberate regression.

This document is the disposition record for
`[spec:nsh:sem:idiom.specified-defects+1]`. The older Dash-derived rules remain
reference provenance; they cannot override the precedence above.
