//! Text pushed in front of a frame's own, which is how an alias is re-read.
//!
//! An alias expansion is not a new input source. It is the same frame with
//! a different string under the cursor, so the line number, the pending
//! here-documents and the parse in progress all still belong to the line
//! the alias name appeared on. Pushing saves the cursor and swaps in a
//! copy of the text; popping puts the cursor back.
//!
//! The copy and the delayed release are what make that safe. An alias may
//! be redefined while its own expansion is being read, so the text being
//! read is owned here rather than borrowed from the alias table. And the
//! `ALIASINUSE` marking -- what stops `alias a=a` expanding forever --
//! may only be cleared once the reader has looked past the end of the
//! text, so a popped overlay is parked on the frame's `deferred_overlays`
//! and released at the next read rather than at the pop.

use super::*;

/// The C's `struct strpush`.
///
/// `prev` is the `Vec` order and `basestrpush` has no reason to exist, so
/// both are gone. `string` is a copy of the pushed text; in the C it is
/// `ap->name`, the *whole* `name=value` allocation that `ap->val` points
/// into, held so that redefining an alias mid-expansion does not free the
/// text being read. See `plan/decisions/owned-data.md`.
pub struct InputOverlay {
    /// `sp->prevstring`, as a cursor into the text that was current
    pub previous_position: usize,
    pub previous_line_remaining: usize,
    /// if push was associated with an alias
    pub alias_name: Option<BString>,
    /// the complete pushed text
    pub string: Vec<u8>,
    /// `sp->spfree`: the pending-free chain hidden while this string is read
    pub deferred_overlays: Vec<InputOverlay>,
    /// Number of outstanding calls to pungetc.
    pub unread_count: usize,
}

/// Clear `ALIASINUSE` on everything in `list`, newest first, which is the
/// order the C's `spfree` chain walks in. The `strpush` nodes themselves are
/// dropped with the `Vec`; the C's `ckfree` on each is what that replaces.
pub(super) fn release_input_overlays(
    shell: &mut crate::context::Shell,
    mut list: Vec<InputOverlay>,
) {
    while let Some(mut overlay) = list.pop() {
        if let Some(name) = &overlay.alias_name {
            shell.aliases.finish_expansion(BStr::new(name.as_slice()));
        }
        /* Only an entry that is still on `strpush` carries one; `popstring`
         * moves the chain out on the way past. */
        let carry = core::mem::take(&mut overlay.deferred_overlays);
        if !carry.is_empty() {
            release_input_overlays(shell, carry);
        }
    }
}

// [spec:dash:sem:input.freestrings-fn]
pub(super) fn clear_input_overlays(shell: &mut crate::context::Shell) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let list = core::mem::take(&mut current_input_frame(&mut shell.input).deferred_overlays);
        release_input_overlays(shell, list);
    });
}

/*
 * Push a string back onto the input at this current parsefile level.
 * We handle aliases this way.
 */

// [spec:dash:sem:input.pushstring-fn]
pub fn push_string_input(shell: &mut Shell, string: &BStr, alias_name: Option<BString>) {
    let string_length = string.len();
    crate::error::with_interrupts_deferred(shell, |shell| {
        if let Some(name) = &alias_name {
            shell.aliases.begin_expansion(BStr::new(name.as_slice()));
        }
        /*dprintf("*** calling pushstring: %s, %d\n", s, len);*/
        /* The C picks between `basestrpush` and a `ckmalloc` here; a `Vec`
         * needs neither, and the condition it picked on was only ever about
         * whether the inline slot was still spoken for. */
        let input_frame = current_input_frame(&mut shell.input);
        let string = string.to_vec();
        let overlay = InputOverlay {
            previous_position: input_frame.position,
            previous_line_remaining: input_frame.line_remaining,
            unread_count: input_frame.unread_count,
            deferred_overlays: core::mem::take(&mut input_frame.deferred_overlays),
            alias_name,
            string,
        };
        /* The C reads on through `ap->val`, which points into `ap->name`; this
         * reads the copy, so redefining the alias mid-expansion cannot pull the
         * text out from under the cursor and `popstring` has nothing to free. */
        input_frame.position = 0;
        input_frame.line_remaining = string_length;
        input_frame.unread_count = 0;
        input_frame.overlays.push(overlay);
    });
}

// [spec:dash:sem:input.popstring-fn]
// [spec:posix:req:token.alias-trailing-blank-chaining]
pub(super) fn pop_string_input(shell: &mut Shell) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let input_frame = current_input_frame(&mut shell.input);
        let mut overlay = input_frame.overlays.pop().unwrap();

        /* The C compares `nextc` against `sp->string`, which is `ap->name` —
         * the base of the allocation `ap->val` points into — so the test reads
         * as "always true" and the byte it then looks at is the one before the
         * cursor. Against the copy the same test means "at least one character
         * consumed", and the two agree: with none consumed the C reads the `=`
         * that ends the alias name, which is neither a space nor a tab. */
        let boundary = overlay.alias_name.is_some()
            && input_frame.position > 0
            && matches!(overlay.string[input_frame.position - 1], b' ' | b'\t');
        input_frame.position = overlay.previous_position;
        input_frame.line_remaining = overlay.previous_line_remaining;
        input_frame.unread_count = overlay.unread_count;
        /*dprintf("*** calling popstring: restoring to '%s'\n", parsenextc);*/
        /* `parsefile->spfree = sp` with `sp->spfree` already holding the chain
         * that was hidden when `sp` was pushed. Anything the current chain still
         * held is dropped, which is what the C's assignment does to it. */
        input_frame.deferred_overlays = core::mem::take(&mut overlay.deferred_overlays);
        input_frame.deferred_overlays.push(overlay);
        /* Set after the frame's borrow ends; it is a flag on the stack, not
         * on the frame, and nothing between here and there reads it. */
        if boundary {
            shell.input.alias_boundary = true;
        }
    });
}
