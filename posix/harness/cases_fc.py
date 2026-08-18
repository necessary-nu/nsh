"""`fc`'s editor path, which needed a writable /tmp rather than an excuse.

Three rules were `manual` because dash builds its scratch file as
`sprintf(editfile, "%s_shXXXXXX", _PATH_TMP)` -- a compile-time `/tmp`,
not `$TMPDIR` -- and the sandbox mounts the whole root read-only, so the
editor path could not run at all. That is a property of the wrapper, not
of the requirement, so the wrapper grew a per-case writable `/tmp`
(`Case.writable_tmp`) instead.

It is opt-in rather than always on: a bind over `/tmp` hides everything
beneath it, and both the case root and -- depending on how it was built --
the shell under test can live there. Only these cases need it.

`fc` needs a history to edit, and dash only creates one for an
interactive shell, so these run on the pty.
"""

from __future__ import annotations

from model import Case, FileFixture


# An "editor" in the sense POSIX means: a utility name found via PATH,
# handed the temporary file to rewrite. `.bin` is on the case's PATH.
EDITOR = FileFixture(
    content="#!/bin/sh\nprintf 'printf EDITED\\\\n\\n' > \"$1\"\n",
    mode=0o755,
)

FAILING_EDITOR = FileFixture(
    content=(
        "#!/bin/sh\n"
        "printf \"%s\\n\" \"printf 'SHOULD-NOT-RUN\\\\n' > ran\" > \"$1\"\n"
        "exit 7\n"
    ),
    mode=0o755,
)

STATUS_EDITOR = FileFixture(
    content="#!/bin/sh\nprintf '( exit 6 )\\n' > \"$1\"\n",
    mode=0o755,
)


CASES: tuple[Case, ...] = (
    # [spec:posix:req:builtin.fc.opt-e/test]
    Case(
        id="fc-opt-e-names-an-editor",
        rules=("builtin.fc.opt-e",),
        mode="interactive",
        writable_tmp=True,
        # "-e editor -- Use the editor named by editor to edit the
        # commands. The editor string is a utility name, subject to search
        # via the PATH variable." The edited text is then executed, so
        # EDITED on the transcript is the whole assertion.
        #
        # THIS FAILS, AND IT IS A REAL dash BUG, not a harness artefact.
        # histedit.c parses the option with `getopt(argc, argv, ":e:lnrs")`
        # -- which sets libc's `optarg` -- and then reads dash's own
        # `optionarg`, the global that `nextopt` sets (options.c:61,
        # "set by nextopt (like getopt)"). The two are different variables,
        # so `editor` stays NULL, the DEFEDITOR fallback wins, and
        # `fc -e whatever` silently runs `ed`:
        #
        #     fc: ed: not found
        #
        # The sibling case below passes, which localises it precisely: the
        # FCEDIT path assigns `editor` from bltinlookup and works, so only
        # the -e option-argument is lost. The port reproduces the bug
        # faithfully, which is the point -- both shells fail this case
        # identically.
        script="printf 'first\\n'\nfc -e fakeed\nexit\n",
        files={".bin/fakeed": EDITOR},
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("EDITED",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.fc.env-fcedit/test]
    # [spec:posix:req:sh.envvar-fcedit/test]
    Case(
        id="fc-fcedit-supplies-the-default-editor",
        rules=("builtin.fc.env-fcedit", "sh.envvar-fcedit"),
        mode="interactive",
        writable_tmp=True,
        # "FCEDIT ... shall determine the default value for the -e editor
        # option's editor option-argument" -- so `fc` with no -e must use
        # it.
        script="printf 'first\\n'\nfc\nexit\n",
        files={".bin/fakeed": EDITOR},
        environment={"PS1": "", "PS2": "", "FCEDIT": "fakeed"},
        stdout=None,
        stdout_contains=("EDITED",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.fc.edit-and-reexecute/test]
    Case(
        id="fc-edited-line-enters-history",
        rules=("builtin.fc.edit-and-reexecute",),
        mode="interactive",
        writable_tmp=True,
        script=(
            ": ORIGINAL\n"
            "fc -e fakeed >/dev/null\n"
            "printf 'edited-history=%s\\n' "
            "\"$(fc -l -n | grep -c 'printf EDITED')\"\n"
            "printf 'fc-history=%s\\n' "
            "\"$(fc -l -n | grep -c 'fc -e fakeed')\"\n"
            "exit 0\n"
        ),
        files={".bin/fakeed": EDITOR},
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("edited-history=1", "fc-history=0"),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.fc.edit-and-reexecute/test]
    # [spec:posix:req:builtin.fc.exit-status/test]
    Case(
        id="fc-failed-editor-suppresses-reexecution",
        rules=("builtin.fc.edit-and-reexecute", "builtin.fc.exit-status"),
        mode="interactive",
        writable_tmp=True,
        script=(
            ": ORIGINAL\n"
            "fc -e failededit >/dev/null 2>&1\n"
            "editor_status=$?\n"
            "if [ -e ran ]; then ran=yes; else ran=no; fi\n"
            "if fc -l -n | grep -q 'fc -e failededit'; "
            "then kept=yes; else kept=no; fi\n"
            "printf 'status=%s,ran=%s,kept=%s\\n' "
            "\"$editor_status\" \"$ran\" \"$kept\"\n"
            "exit 0\n"
        ),
        files={".bin/failededit": FAILING_EDITOR},
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("status=7,ran=no,kept=no",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:builtin.fc.exit-status/test]
    Case(
        id="fc-edited-command-exit-status",
        rules=("builtin.fc.exit-status",),
        mode="interactive",
        writable_tmp=True,
        script=(
            ": ORIGINAL\n"
            "fc -e statusedit >/dev/null 2>&1\n"
            "printf 'edited-status=%s\\n' \"$?\"\n"
            "exit 0\n"
        ),
        files={".bin/statusedit": STATUS_EDITOR},
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("edited-status=6",),
        status="any",
        timeout=15.0,
    ),
)
