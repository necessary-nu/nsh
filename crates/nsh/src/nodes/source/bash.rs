//! Printing the forms only Bash's grammar has.
//!
//! `[[ ]]`, `((` `))`, an arithmetic `for`, an array assignment and a
//! process substitution. They are separate for the same reason the parser
//! and the expander keep a `bash` module: the dialect is a decision, and a
//! file boundary is where a reader can see it.

use super::*;

impl<'a> Printer<'a> {
    pub(super) fn bash_command(&mut self, node: &BashNode, indent: usize) {
        match node {
            BashNode::Conditional(command) => {
                self.out.extend_from_slice(b"[[ ");
                self.conditional(&command.expression, indent);
                self.out.extend_from_slice(b" ]]");
            }
            BashNode::ArithmeticCommand(command) => {
                // No padding: the expression the tree holds is the one the
                // source wrote, and a space added here comes back as part of
                // it. [spec:nsh:req:idiom.printable-ast+2]
                let expression = command.expression.as_bstr();
                if expression.is_empty() {
                    self.out.extend_from_slice(b"(())");
                } else {
                    self.out.extend_from_slice(b"((");
                    self.out.extend_from_slice(expression);
                    self.out.extend_from_slice(b"))");
                }
            }
            BashNode::ArithmeticFor(command) => self.arithmetic_for(command, indent),
            BashNode::Function(function) => {
                let style = match function.style {
                    BashFunctionStyle::Function => DefinitionStyle::Keyword,
                    BashFunctionStyle::FunctionParens => DefinitionStyle::KeywordParens,
                };
                self.nested_function(function.name.as_bstr(), &function.body, indent, style);
            }
            BashNode::ArrayAssignment(assignment) => self.array_assignment(assignment, indent),
            BashNode::ProcessSubstitution(substitution) => {
                self.process_substitution(substitution, indent);
            }
        }
    }

    fn arithmetic_for(&mut self, command: &BashArithmeticFor, indent: usize) {
        self.out.extend_from_slice(b"for ((");
        self.out.extend_from_slice(trimmed(command.init.as_bstr()));
        self.out.extend_from_slice(b"; ");
        self.out.extend_from_slice(trimmed(command.test.as_bstr()));
        self.out.extend_from_slice(b"; ");
        self.out
            .extend_from_slice(trimmed(command.update.as_bstr()));
        self.out.extend_from_slice(b"))");
        self.newline(indent);
        self.out.extend_from_slice(b"do");
        self.newline(indent + STEP);
        self.terminated_list(&command.body, indent + STEP);
        self.newline(indent);
        self.out.extend_from_slice(b"done");
    }

    fn conditional(&mut self, expression: &BashConditionalExpr, indent: usize) {
        match expression {
            BashConditionalExpr::Empty => {}
            BashConditionalExpr::Word(word) => self.word(word, indent),
            BashConditionalExpr::Unary { operator, operand } => {
                self.out.extend_from_slice(operator.as_bstr());
                self.out.push(b' ');
                self.word(operand, indent);
            }
            BashConditionalExpr::Binary {
                left,
                operator,
                right,
            } => {
                self.word(left, indent);
                self.out.push(b' ');
                self.out.extend_from_slice(operator.as_bstr());
                self.out.push(b' ');
                self.word(right, indent);
            }
            BashConditionalExpr::Not(inner) => {
                self.out.extend_from_slice(b"! ");
                self.conditional(inner, indent);
            }
            BashConditionalExpr::And(left, right) => {
                self.conditional(left, indent);
                self.out.extend_from_slice(b" && ");
                self.conditional(right, indent);
            }
            BashConditionalExpr::Or(left, right) => {
                self.conditional(left, indent);
                self.out.extend_from_slice(b" || ");
                self.conditional(right, indent);
            }
            BashConditionalExpr::Group(inner) => {
                self.out.extend_from_slice(b"( ");
                self.conditional(inner, indent);
                self.out.extend_from_slice(b" )");
            }
        }
    }

    fn array_assignment(&mut self, assignment: &BashArrayAssignment, indent: usize) {
        self.out.extend_from_slice(assignment.name.as_bstr());
        if let Some(subscript) = &assignment.subscript {
            self.out.push(b'[');
            self.word(subscript, indent);
            self.out.push(b']');
        }
        self.out
            .extend_from_slice(operator_text(assignment.operator));
        match &assignment.value {
            BashArrayValue::Word(word) => self.word(word, indent),
            BashArrayValue::Compound(elements) => {
                self.out.push(b'(');
                for (position, element) in elements.iter().enumerate() {
                    if position > 0 {
                        self.out.push(b' ');
                    }
                    self.array_element(element, indent);
                }
                self.out.push(b')');
            }
        }
    }

    fn array_element(&mut self, element: &BashArrayElement, indent: usize) {
        if let Some(subscript) = &element.subscript {
            self.out.push(b'[');
            self.word(subscript, indent);
            self.out.push(b']');
            self.out.extend_from_slice(operator_text(element.operator));
        }
        self.word(&element.value, indent);
    }

    pub(super) fn process_substitution(&mut self, node: &BashProcessSubstitution, indent: usize) {
        self.out.extend_from_slice(match node.direction {
            BashProcessDirection::Input => b"<(",
            BashProcessDirection::Output => b">(",
        });
        if let Some(body) = &node.body {
            self.list(body, indent);
        }
        self.out.push(b')');
    }
}
