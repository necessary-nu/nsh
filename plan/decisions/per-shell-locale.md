---
id [dec:nsh:per-shell-locale]
epitome "Each Shell owns an explicit POSIX locale object; the host environment and global locale are never its configuration channel."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core])
    rules (
        [spec:nsh:req:embedding-safety.process-environment-is-read-only]
        [spec:nsh:req:embedding-safety.process-locale-is-unchanged]
        [spec:nsh:def:shell-locale.owned-locale]
        [spec:nsh:req:shell-locale.handle-lifetime]
        [spec:nsh:sem:shell-locale.selection]
        [spec:nsh:sem:shell-locale.invalid-selection]
        [spec:nsh:req:shell-locale.operation-binding]
        [spec:nsh:req:shell-locale.instance-isolation]
    )
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep mutating `environ` and the process-global locale, protected by an nsh mutex."
        rejected_because "A safe library cannot make an embedder take its mutex before the embedder or another dependency reads the environment or calls setlocale. Serialising nsh callers would hide the race inside the API rather than establish its precondition."
    }
    {
        option "Select one locale for the duration of `Shell::run`."
        rejected_because "A script can assign LC_ALL, LC_*, or LANG during the run. A run-long selection would keep using the locale that was current on entry and make the assignment ineffective until the next public call."
    }
    {
        option "Replace POSIX locale semantics with a pure-Rust C/UTF-8 or Unicode locale engine."
        rejected_because "The shell handles arbitrary bytes and installed POSIX locales, including single-byte character sets and platform collation. Narrowing that contract is a deliberate language divergence, not a safety refactor, and the platform crate already exists to own the irreducible ABI."
    }
)
consequences {
    accepted (
        "`Shell` owns a locale object built from explicit category names. Assigning a locale variable replaces that object only after a complete new selection has been constructed."
        "Locale-dependent operations take that object explicitly. `nsh-platform` uses locale-taking functions where available and a short, restoring `uselocale` scope where the C API offers only an ambient operation."
        "The unsafe floor grows three lifecycle operations -- newlocale, uselocale and freelocale -- while deleting process-global setlocale and every process-environment mutation. The raw handle, its selection guard and its Drop implementation remain private to nsh-platform."
        "An executed child receives an envp assembled from the Shell variable table. The host's environ is never staging storage for a child."
    )
    deferred ("The current directory, child-reaping pool, process group, controlling terminal and signal inbox remain process-wide for the independent reasons recorded by their owning decisions. This decision changes none of those grants or caveats.")
}
edges {
    requires (
        [dec:nsh:shell-as-library]
        [dec:nsh:no-ambient-state]
        [dec:nsh:minimal-unsafe]
        [dec:nsh:bytes-not-text]
    )
    constrains ([dec:nsh:public-surface])
}
codifies (
    [spec:nsh:req:embedding-safety.process-environment-is-read-only]
    [spec:nsh:req:embedding-safety.process-locale-is-unchanged]
    [spec:nsh:def:shell-locale.owned-locale]
    [spec:nsh:req:shell-locale.handle-lifetime]
    [spec:nsh:sem:shell-locale.selection]
    [spec:nsh:sem:shell-locale.invalid-selection]
    [spec:nsh:req:shell-locale.operation-binding]
    [spec:nsh:req:shell-locale.instance-isolation]
)
---

## Rationale

`std::env::set_var` and `remove_var` are unsafe on Unix because the
environment is shared with C code that can read it without Rust's lock. A safe
wrapper cannot discharge that precondition: the wrapper controls only its own
callers, not the embedding program. `setlocale` is worse for an embedded shell
because it deliberately changes the interpretation used by every thread still
following the global locale.

POSIX supplies the missing ownership model. `newlocale` constructs a locale
object from explicit names without consulting or changing `environ`;
locale-taking functions consume that handle directly; `uselocale` selects one
for only the calling thread and returns the previous selection. A guard can
therefore borrow the owned handle, restore the previous selection in `Drop`,
and be made non-`Send` so restoration must happen on the selecting thread.

The transient thread selection is not the `thread_local!` shell state rejected
by [dec:nsh:no-ambient-state]. No shell datum is found through it and two Shell
values on one thread do not share it. It is a bounded C ABI precondition around
one operation, restored before control returns to the caller; the durable
choice remains a field of `Shell` and is visible to Rust's ownership system.

Environment and locale then separate cleanly. Shell variables are byte strings
owned by the shell. They are the source for both locale selection and the envp
passed to `execve`, but neither consumer needs those bytes installed in the
host process first. That is the property a safe embedding API can actually
guarantee.
