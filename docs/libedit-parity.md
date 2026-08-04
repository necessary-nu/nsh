# What the libedit port has to make true

`crates/dash/src/linedit.rs` stands in for libedit with rustyline. It is
the *only* remaining place where the Rust port and the C dash built from
this tree disagree about anything.

That is not a claim from reading the code. Running the POSIX suite
against both binaries (`posix/harness/run.py --shell <port> --reference
<C dash>`) gives 674 cases, of which:

  * 580 both pass,
  * 54 both fail — dash is non-conformant and the port reproduces it,
    which is the point of a bug-for-bug port,
  * 40 disagree, and **every one of the 40 is a line-editing case**.

So this file is the acceptance criterion for the libedit subcrate: make
these 40 agree with the C and the two shells are indistinguishable to
every test in the repo.

Reproduce with:

    posix/harness/run.py --shell target/debug/dash \
        --reference tests/.build/ref/src/dash --format json

## The one the port currently gets *right* and dash gets wrong

`edit-undo-all-changes` passes on the port and fails on dash. A literal
port has to reproduce dash's failure too. Do not treat this one as a
bonus.

## Two things rustyline does that libedit does not

Both show up in the raw output below and neither is shell behaviour, but
both are visible to a test that reads the terminal:

  * **Synchronized-output markers.** rustyline brackets redraws with
    `ESC [ ? 2026 h` / `ESC [ ? 2026 l`. libedit emits nothing of the
    kind. `ptydiff.py` strips ANSI, so it does not see this; the POSIX
    harness does.
  * **A count indicator.** rustyline prints `(arg: 2)` in place of the
    prompt while a vi count is being entered. libedit redraws the line
    with no such annotation.

## The cases


### `blt2-edit-motion-operand-column`  — port FAIL, dash PASS

Rules: `edit.motion-command-set`, `edit.delete-motion`

  * port: stdout missing 'R:def\n'

### `blt2-edit-motion-operand-first-nonblank`  — port FAIL, dash PASS

Rules: `edit.motion-command-set`, `edit.delete-motion`

  * port: stdout missing 'R:abcdef\n'

### `blt2-edit-save-buffer-single`  — port FAIL, dash PASS

Rules: `edit.word-bigword-terms`, `edit.delete-char`

  * port: stdout missing 'R:acdefb\n'
  * port: stdout unexpectedly contains 'R:acdefr\n'

### `edit-append-counted-bigword`  — port FAIL, dash PASS

Rules: `edit.append-last-bigword`

  * port: timed out after 4s
  * port: stdout missing 'R:aa\n'

### `edit-append-last-bigword`  — port FAIL, dash PASS

Rules: `edit.append-last-bigword`

  * port: timed out after 4s

### `edit-change-to-end-of-line`  — port FAIL, dash PASS

Rules: `edit.change-to-end-and-line`

  * port: stdout missing 'R:abcZ\n'

### `edit-command-case-toggle`  — port FAIL, dash PASS

Rules: `edit.command-case-toggle`

  * port: stdout missing 'R:abcdeF\n'

### `edit-command-case-toggle-count`  — port FAIL, dash PASS

Rules: `edit.command-case-toggle`, `edit.command-count`

  * port: stdout missing 'R:abcDEF\n'

### `edit-command-comment`  — port FAIL, dash PASS

Rules: `edit.command-comment`

  * port: timed out after 4s
  * port: stdout missing 'R:commented\n'
  * port: stdout missing 'R:zzz\n'
  * port: stdout unexpectedly contains 'R:abc\n'

### `edit-command-mode-unknown-alerts`  — port FAIL, dash PASS

Rules: `edit.escape-to-command-mode`

  * port: stdout missing '\x07'
  * port: stdout missing 'R:abc\n'

### `edit-command-newline-executes`  — port FAIL, dash PASS

Rules: `edit.command-newline`

  * port: timed out after 4s
  * port: stdout missing 'R:abc\n'

### `edit-command-repeat`  — port FAIL, dash PASS

Rules: `edit.command-repeat`

  * port: stdout missing 'R:abcd\n'

### `edit-delete-char-at-cursor`  — port FAIL, dash PASS

Rules: `edit.delete-char`

  * port: stdout missing 'R:abcde\n'

### `edit-delete-char-before-count`  — port FAIL, dash PASS

Rules: `edit.delete-char`, `edit.command-count`

  * port: stdout missing 'R:abf\n'

### `edit-delete-char-before-cursor`  — port FAIL, dash PASS

Rules: `edit.delete-char`

  * port: stdout missing 'R:abcdf\n'

### `edit-delete-to-end-of-line`  — port FAIL, dash PASS

Rules: `edit.delete-motion`

  * port: stdout missing 'R:abc\n'

### `edit-enter-insert-before-cursor`  — port FAIL, dash PASS

Rules: `edit.enter-insert-mode`

  * port: stdout missing 'R:abXd\n'

### `edit-enter-insert-replacing`  — port FAIL, dash PASS

Rules: `edit.enter-insert-mode`

  * port: stdout missing 'R:abcdzz\n'

### `edit-escape-enters-command-mode`  — port FAIL, dash PASS

Rules: `edit.insert-escape`, `edit.escape-to-command-mode`

  * port: stdout missing 'R:abcde\n'
  * port: stdout unexpectedly contains 'R:abcdef\n'

### `edit-history-goto-oldest`  — port FAIL, dash PASS

Rules: `edit.history-goto`

  * port: timed out after 4s
  * port: stdout missing 'R:one ok\n'

### `edit-history-next`  — port FAIL, dash PASS

Rules: `edit.history-prev-next`

  * port: timed out after 4s
  * port: stdout missing 'R:two ok\n'

### `edit-history-previous`  — port FAIL, dash PASS

Rules: `edit.history-prev-next`

  * port: timed out after 4s
  * port: stdout missing 'R:one ok\n'

### `edit-history-previous-past-limit`  — port FAIL, dash PASS

Rules: `edit.history-prev-next`

  * port: timed out after 4s
  * port: stdout missing '\x07'
  * port: stdout missing 'R:zzz\n'

### `edit-history-search-backward`  — port FAIL, dash PASS

Rules: `edit.history-search-backward`

  * port: timed out after 4s
  * port: stdout missing 'R:alpha ok\n'

### `edit-history-search-forward`  — port FAIL, dash PASS

Rules: `edit.history-search-forward`

  * port: timed out after 4s
  * port: stdout missing 'R:beta ok\n'

### `edit-history-search-pattern-glob`  — port FAIL, dash PASS

Rules: `edit.history-search-pattern`

  * port: timed out after 4s
  * port: stdout missing 'R:alpha ok\n'

### `edit-history-search-repeat`  — port FAIL, dash PASS

Rules: `edit.history-search-repeat`

  * port: timed out after 4s
  * port: stdout missing 'R:alpha1 ok\n'

### `edit-history-search-repeat-reverse`  — port FAIL, dash PASS

Rules: `edit.history-search-repeat`

  * port: timed out after 4s
  * port: stdout missing 'R:alpha2 ok\n'

### `edit-insert-newline-enters-history`  — port FAIL, dash PASS

Rules: `edit.insert-newline`

  * port: timed out after 4s
  * port: stdout missing 'R:inhistory\n'

### `edit-motion-char-backward`  — port FAIL, dash PASS

Rules: `edit.motion-char`

  * port: stdout missing 'R:abcef\n'

### `edit-motion-line-column`  — port FAIL, dash PASS

Rules: `edit.motion-line-position`

  * port: stdout missing 'R:abcdef\n'

### `edit-motion-line-end`  — port FAIL, dash PASS

Rules: `edit.motion-line-position`

  * port: stdout missing 'R:abcdef\n'

### `edit-put-after-cursor`  — port FAIL, dash PASS

Rules: `edit.put-save-buffer`

  * port: stdout missing 'R:abcdfe\n'

### `edit-put-before-cursor`  — port FAIL, dash PASS

Rules: `edit.put-save-buffer`

  * port: stdout missing 'R:abcdfe\n'

### `edit-replace-char`  — port FAIL, dash PASS

Rules: `edit.replace-char`

  * port: stdout missing 'R:abcdeZ\n'

### `edit-replace-char-count`  — port FAIL, dash PASS

Rules: `edit.replace-char`, `edit.command-count`

  * port: stdout missing 'R:abcZZZ\n'

### `edit-stty-erase-character`  — port FAIL, dash PASS

Rules: `edit.stty-characters`

  * port: stdout missing 'R:abc\n'

### `edit-undo-all-changes`  — port PASS, dash FAIL

Rules: `edit.undo`

  * dash: stdout missing 'R:abcdef ok\n'

### `edit-undo-last-change`  — port FAIL, dash PASS

Rules: `edit.undo`

  * port: stdout missing 'R:abcde\n'

### `edit-yank-to-end-of-line`  — port FAIL, dash PASS

Rules: `edit.yank-motion`

  * port: stdout missing 'R:abcdefr abcdef\n'

