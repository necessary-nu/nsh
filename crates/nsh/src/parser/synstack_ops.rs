use super::{Syntax, synstack};

// [spec:dash:def:parser.synstack-push-fn]
// [spec:dash:sem:parser.synstack-push-fn]
pub(super) fn push(stack: &mut Vec<synstack>, syntax: *const Syntax) {
    stack.push(synstack {
        syntax,
        innerdq: 0,
        varpushed: 0,
        dblquote: 0,
        backq: 0,
        varnest: 0,
        parenlevel: 0,
        dqvarnest: 0,
    });
}

// [spec:dash:def:parser.synstack-pop-fn]
// [spec:dash:sem:parser.synstack-pop-fn]
pub(super) fn pop(stack: &mut Vec<synstack>) {
    let _ = stack.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:dash:sem:parser.synstack-push-fn/test]
    #[test]
    fn push_initialises_a_clean_frame() {
        let mut stack = Vec::new();
        let syntax = core::ptr::null();

        push(&mut stack, syntax);

        assert_eq!(stack.len(), 1);
        let frame = &stack[0];
        assert_eq!(frame.syntax, syntax);
        assert_eq!(frame.innerdq, 0);
        assert_eq!(frame.varpushed, 0);
        assert_eq!(frame.dblquote, 0);
        assert_eq!(frame.backq, 0);
        assert_eq!(frame.varnest, 0);
        assert_eq!(frame.parenlevel, 0);
        assert_eq!(frame.dqvarnest, 0);
    }

    // [spec:dash:sem:parser.synstack-pop-fn/test]
    #[test]
    fn pop_removes_only_the_top_frame() {
        let mut stack = Vec::new();
        push(&mut stack, core::ptr::null());
        push(&mut stack, core::ptr::dangling());

        pop(&mut stack);
        assert_eq!(stack.len(), 1);
        assert!(stack[0].syntax.is_null());

        pop(&mut stack);
        assert!(stack.is_empty());
        pop(&mut stack);
        assert!(stack.is_empty());
    }
}
