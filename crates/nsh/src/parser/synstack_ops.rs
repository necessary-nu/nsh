use super::{BackquoteContext, SyntaxContext, synstack};

// [spec:dash:def:parser.synstack-push-fn]
// [spec:dash:sem:parser.synstack-push-fn]
// [spec:nsh:req:idiom.lexer-tokens]
pub(super) fn push(stack: &mut Vec<synstack>, syntax: SyntaxContext) {
    stack.push(synstack {
        syntax,
        inner_double_quote: false,
        variable_context_pushed: false,
        double_quoted: false,
        backquote: BackquoteContext::None,
        variable_depth: 0,
        parenthesis_depth: 0,
        double_quote_variable_depth: 0,
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
        let syntax = SyntaxContext::Base;

        push(&mut stack, syntax);

        assert_eq!(stack.len(), 1);
        let frame = &stack[0];
        assert_eq!(frame.syntax, syntax);
        assert!(!frame.inner_double_quote);
        assert!(!frame.variable_context_pushed);
        assert!(!frame.double_quoted);
        assert_eq!(frame.backquote, BackquoteContext::None);
        assert_eq!(frame.variable_depth, 0);
        assert_eq!(frame.parenthesis_depth, 0);
        assert_eq!(frame.double_quote_variable_depth, 0);
    }

    // [spec:dash:sem:parser.synstack-pop-fn/test]
    #[test]
    fn pop_removes_only_the_top_frame() {
        let mut stack = Vec::new();
        push(&mut stack, SyntaxContext::Base);
        push(&mut stack, SyntaxContext::DoubleQuoted);

        pop(&mut stack);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack[0].syntax, SyntaxContext::Base);

        pop(&mut stack);
        assert!(stack.is_empty());
        pop(&mut stack);
        assert!(stack.is_empty());
    }
}
