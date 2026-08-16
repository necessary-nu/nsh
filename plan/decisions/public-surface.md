---
id [dec:nsh:public-surface]
epitome "`Shell` is the API; everything else is `pub(crate)`, and the frontend is a separate crate so the compiler says so."
state @decided
category @existence
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep one crate with a `[[bin]]` target, and mark internals `pub(crate)` as they are found."
        rejected_because "A binary inside the library crate can reach `crate::` internals, so nothing ever reports that the frontend used something an embedder cannot. The boundary would be enforced by inspection, which means it would be enforced until someone is in a hurry. The split makes it a compile error, and there is no other mechanism that does."
    }
    {
        option "Expose the modules as they are and let embedders take what they need."
        rejected_because "That is the present state, and it is why [dec:nsh:shell-as-library] is unfalsifiable. `crate::eval::exitstatus` is public. So is `memalloc::stalloc`. With ~1,000 public items every internal detail is already API, so no restructuring can be described as compatible or incompatible with anything."
    }
)
consequences {
    accepted (
        "The workspace becomes `crates/nsh` (lib) and `crates/nsh-cli` (bin), and the crate is renamed from `dash` to `nsh` in the same move."
        "The surface is `Shell` and its builder, `run` / `run_command`, the variable accessors, `status`, `expand_word`, `Streams`, and the value types `Error` / `ExitStatus` / `Source` / `Signal` / `Host` / `Disposition` / `SignalSink`. Everything else is `pub(crate)`. **The count is fifty `pub` items under `#![deny(missing_docs)]`, not twenty** -- `grep -cE '^\\s*pub (fn|const|struct|enum|trait|type|mod)' crates/nsh/src/api.rs` -- or sixty-three counting the thirteen enum variants the lint also requires documented. The number this decision first recorded counted entries in a table, not items a compiler sees. Against roughly a thousand today it is still two orders of magnitude, which is the property; the arithmetic is corrected so the check can be run rather than approximated. `docs/api-design.md` §2."
        "`expand_word` is on the list deliberately. Word expansion without a command is the thing embedders actually want and no `Command`-style API can offer, and it is the clearest single argument for a shell library over spawning one. It is two methods, because one word is zero, one or many fields: `expand_word` splits and globs, `expand_word_quoted` is the double-quoted form. **It executes.** Command substitution is part of word expansion and there is no mode that suppresses it, because the shell language has no such mode."
        "A `Host` trait carries what a library may not do on its own authority, and the frontend implements it. That is where [dec:nsh:host-owns-signals] lands structurally. **No `Host` method takes a `Shell`, and that is the design rather than an omission.** It makes a re-entrant `run` from inside a callback a compile error instead of a documented hazard; it makes `self.host.set_signal(..)` a field-disjoint borrow, so the host composes with the tables it does not touch; and it lets `Shell` own the host rather than threading it through 587 signatures a second time."
        "**`exec cmd` is the second thing on the trait, and it is new.** `execcmd` calls `shellexec`, which `execve`s in the *current* process (`eval.rs:1341-1350`, `exec.rs:118`), so `sh.run(b\"exec ls\")` in an embedded shell replaces the host's image. `Host::may_replace_process` gates it: a frontend says yes because that is what `exec` is for, an embedder's host refuses and gets the diagnostic and status a failed `exec` produces. Terminating the process, by contrast, is NOT on the trait -- after this design the library never needs to, since `run` returns and the frontend exits. The one surviving `_exit` is inside a forked child, where it is forced."
        "**`run` is `eval`, at the top level.** Two calls compose exactly as two `eval` commands do -- variables, functions, aliases, options, traps, cwd, jobs and `$?` all persist -- and for a concrete reason: `sh -c` and the `eval` built-in are already the same primitive, `evalstring` (`eval.rs:192`, `shellmain.rs:176`). What does not compose is the parse: `run(b\"if true; then\")` is the same syntax error `eval 'if true; then'` is, because a `run` that could be continued by the next one would have to block for input, which is what `Source::stream()` is for. `run` records the input-stack depth on entry, makes itself the unwind floor, and unwinds to that depth on every exit path including `Err`. `docs/api-design.md` §4."
        "**A `run` from inside a host callback does not compose because it does not exist.** `run` takes `&mut self`; every callback the shell invokes holds that borrow already; no callback is given a `Shell`. The case this decision deferred is rejected by the compiler rather than by a rule."
        "**Nothing the shell hands to a built-in, a callback or the host may borrow from the shell.** Ten built-ins re-enter evaluation (`.`, `eval`, `command`, `fc`, the trap dispatcher), so a builtin signature taking `&mut Shell` alongside `args: &[&BStr]` only works while `args` is owned elsewhere. Stating it now is the point of designing early: it decides how `no-ambient-state` stores an argument vector, and discovering it afterwards means storing it twice. **This consequence is unchanged and was never in question; only the return type it illustrated with has been corrected -- see the next item.**"
        "**AMENDED (+1). The builtin signature is `fn(&mut Shell, &[&BStr]) -> Result<Flow, Error>`, not `Result<ExitStatus, Error>`, and `ExitStatus` is the *embedder-facing* type.** This decision recorded the `ExitStatus` form before [dec:nsh:errors-are-values] step E existed. That step put control flow in the `Ok` position on purpose -- `exit`, `return`, `break`, `continue` and the `set -e` abort are not errors and must not sit in `Err` -- and `exit` is itself a builtin, so a table of one function-pointer type either lets every entry say \"the shell is exiting\" or forces `exit` to keep jumping. Three more need it for the same reason: `.`, `fc` and `eval` re-enter evaluation, so an `exit` inside them has to travel back out through them. Collapsing `Flow` to `ExitStatus` at the builtin boundary would re-lose exactly what that step bought.\n\nThe two types are layers, not rivals, and separating them dissolves the apparent conflict:\n\n* **`Flow` is the internal currency** of the builtin table and the evaluator. It is `pub(crate)` and no embedder sees it.\n* **`ExitStatus` is the surface type.** It is `Shell::run`'s return, what `Shell::status` answers, and what `Flow::Exit` maps to at the API boundary -- which is the one place the collapse is correct, because an embedder asking \"what happened\" wants a status and not the evaluator's reason for stopping.\n\nSo the surface list above is right as written and needs no edit: `ExitStatus` is a value type on it. What was wrong was one illustration inside a consequence about borrows, which named a return type it did not depend on."
        "**`Streams::install` retires and `set` becomes the only mode.** Once the shell has a per-instance descriptor table, a forked child materialises the map with `dup2` before `execve`, so redirection, pipelines and external commands all follow supplied streams without touching the process's descriptors. `docs/idiomatization.md` §7.5 guessed that external commands were the unfixable part; they are the fixable part. Two gaps survive and are documented rather than closed: `/dev/stdout`, `/dev/fd/N` and `/proc/self/fd/N` name the kernel's table, and `exec cmd` cannot be honoured at all. `Streams::capture` is backed by an unlinked temporary file, not a pipe, because a pipe with no concurrent reader deadlocks the shell on any script with real output."
        "The design has to be settled EARLY and implemented LATE. It determines what [dec:nsh:no-ambient-state] builds -- which fields live on `Shell`, what its borrow shape is -- so designing it afterwards means moving the state twice. `docs/api-design.md` §5 is the field list, at one field per `move-state` commit."
    )
    deferred ("Whether the per-instance descriptor table delivers what the paragraph above claims. The argument holds for every path through `redir.rs`; what it cannot rule out is a site that hands a raw descriptor number to a syscall without consulting the map, which is invisible under `Streams::inherit()` because the map is then the identity -- and the differential harness runs only that configuration. `crates/nsh/tests/streams_embed.rs` has to grow the redirection, pipeline and external-command cases BEFORE the table is built, and a failure is a reason to keep `install` rather than to weaken the tests.")
}
edges {
    requires ([dec:nsh:shell-as-library])
    enables ([dec:nsh:no-ambient-state])
}
---

## Rationale

`lib.rs` declares 35 `pub mod`, and the crate exposes something on the
order of a thousand public items. That is not an API with too much in it;
it is the absence of one. There is no line anywhere between what an
embedder may touch and what is internal, which means
[dec:nsh:shell-as-library] cannot be checked -- every internal detail is
already part of the surface, so no change to any of them is a change to
the API or not.

What an embedder should be writing:

```rust
let mut sh = Shell::builder()
    .arg0(BStr::new(b"myapp"))
    .streams(Streams::capture()?)
    .build()?;

let status = sh.run(b"for f in *.txt; do wc -l \"$f\"; done")?;
let out: BString = sh.take_captured_stdout()?;
```

The last line was `let out: &BStr = sh.captured_stdout();` when this
decision was written, and it does not compile. The borrow is tied to the
`&mut self` that reads the capture, so holding the output locks the shell
and run-look-run -- the entire reason to capture -- fails with four
`E0499`s. `crates/nsh/examples/embed.rs` was written before
`crates/nsh/src/api.rs` precisely so that the example got to judge the API,
and this is what it caught. `.env(std::env::vars_os())`, from the same
sketch, does not compile either: `OsString` is bytes on Unix but is not
`Into<BString>`, so the builder has `env` for explicit pairs and
`inherit_env()` for the process's own.

Three things separate that from spawning `/bin/sh`: no second process,
no quoting round-trip in or out ([dec:nsh:bytes-not-text]), and errors
that arrive as values rather than as a status and some text on a pipe
([dec:nsh:errors-are-values]). A fourth is easy to promise and hard to
deliver -- two `Shell` values sharing nothing -- and it is the whole
content of [dec:nsh:no-ambient-state].

## The amendment (+1): `Flow` inside, `ExitStatus` at the surface

This decision was written before the exception mechanism was replaced, and
it recorded the builtin signature as
`fn(&mut Shell, &[&BStr]) -> Result<ExitStatus, Error>`. By the time
`public-api` reached step 5 the code had `Result<Flow, Error>` and a
comment explaining why -- so the plan said one thing while the code argued
another, which is the state this amendment exists to end.

**The code is right.** `[dec:nsh:errors-are-values]` step E is the reason:
it moved control flow into the `Ok` position deliberately, because `exit`
is not a failure and putting it in `Err` is what made the old shell need a
non-local jump. `exit` is a *builtin*. A builtin table has one
function-pointer type, so either every entry can say "the shell is
exiting" or `exit` alone keeps jumping -- and three others (`.`, `fc`,
`eval`) re-enter evaluation and have to carry an inner `exit` back out
through themselves.

The conflict was only ever apparent, because the two types belong to
different layers. `Flow` is what a builtin hands the evaluator; nothing
outside the crate sees it. `ExitStatus` is what an embedder gets from
`Shell::run` and `Shell::status`, and mapping `Flow::Exit` onto it at that
boundary is not a loss -- it is the right collapse, since a caller asking
"what happened" wants the status, not the evaluator's reason for stopping.
`Shell::has_exited` carries the part of `Flow::Exit` that survives.

Recorded before step 5 was implemented rather than after, so the
implementation is following the decision rather than the decision being
back-filled from the implementation.

## Why the crate split is the load-bearing part

The surface could in principle be closed by marking things `pub(crate)`
one at a time. It would not stay closed. While the frontend is a
`[[bin]]` inside the library crate it can reach `crate::` internals
freely, so the only thing standing between the boundary and its erosion
is whoever is reading the diff.

Splitting `crates/nsh-cli` out changes the kind of guarantee. The
frontend then links the library as an external crate and can use nothing
but its public API, so **anything it needs that is not public stops
compiling**. That converts [dec:nsh:shell-as-library] from an intention
into a build failure, and it costs a workspace member and one path
dependency.

The rename travels with it because the rename is the only piece of this
work whose cost strictly grows with delay: every commit message, module
path and decision written before it has to be written again after.

## Why the design comes early and the implementation late

These are separable and the ordering matters in opposite directions.

The *design* has to precede [dec:nsh:no-ambient-state], because moving
the statics onto an instance is exactly the act of deciding what `Shell`
is. Doing it without knowing the intended surface means choosing the
fields, the borrow shape and the mutability by accident, and then moving
them again once the API is written.

The *implementation* has to follow [dec:nsh:host-owns-signals], because
the `Host` trait is the last thing to take shape and closing the surface
before it exists would only mean reopening it.

## Where the design lives

`docs/api-design.md` is the design, and `crates/nsh/src/api.rs` is the same
surface as compiling Rust with `todo!()` bodies and `#![deny(missing_docs)]`
on. The sketch is compiled rather than merely written because three of the
questions above are answered by the borrow checker and not by prose --
whether a built-in can re-enter evaluation, whether `Host` can take a
`&mut Shell`, whether a captured stream can be borrowed -- and two of them
came out the other way from the sketch this decision first recorded. The
module is deleted by the `public-api` node, which replaces it with the
implementation.
