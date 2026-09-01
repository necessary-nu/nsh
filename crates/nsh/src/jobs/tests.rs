use super::wait::record_child_status;
use super::*;
use crate::nodes::{CaseClause, CaseCommand, SimpleCommand, SourceLine, SourceTokens, WordNode};
use crate::word::ParsedWord;

fn word(text: &[u8]) -> Node {
    Node::Word(WordNode {
        tokens: SourceTokens::none(),
        word: ParsedWord::literal(BString::from(text)),
    })
}

#[test]
fn child_status_derives_job_state() {
    let first = ProcessId::new(1).unwrap();
    let second = ProcessId::new(2).unwrap();
    let mut job = Job::new();
    job.processes = vec![
        ProcessRecord {
            process_id: first,
            status: None,
            command_text: BString::default(),
        },
        ProcessRecord {
            process_id: second,
            status: None,
            command_text: BString::default(),
        },
    ];

    assert_eq!(
        record_child_status(&mut job, first, ChildStatus::Exited(0)),
        Some(JobState::Running)
    );
    assert_eq!(
        record_child_status(
            &mut job,
            second,
            ChildStatus::Stopped(nsh_platform::terminal_stop_signal()),
        ),
        Some(JobState::Stopped)
    );
    assert_eq!(
        record_child_status(&mut job, second, ChildStatus::Exited(0)),
        Some(JobState::Done)
    );
}

#[test]
fn immediate_notification_gates() {
    assert!(notify_completion_now(
        WaitMode::Block,
        JobState::Done,
        true,
        true,
        true,
        false,
    ));
    assert!(!notify_completion_now(
        WaitMode::Poll,
        JobState::Done,
        true,
        true,
        true,
        false,
    ));
    assert!(!notify_completion_now(
        WaitMode::Block,
        JobState::Stopped,
        true,
        true,
        true,
        false,
    ));
    assert!(!notify_completion_now(
        WaitMode::Block,
        JobState::Done,
        false,
        true,
        true,
        false,
    ));
    assert!(!notify_completion_now(
        WaitMode::Block,
        JobState::Done,
        true,
        false,
        true,
        false,
    ));
    assert!(!notify_completion_now(
        WaitMode::Block,
        JobState::Done,
        true,
        true,
        false,
        false,
    ));
    assert!(!notify_completion_now(
        WaitMode::Block,
        JobState::Done,
        true,
        true,
        true,
        true,
    ));
}

// [spec:posix:req:builtin.jobs.stdout-default-format/test]
// [spec:nsh:sem:idiom.specified-defects+1/test]
#[test]
fn job_text_includes_assignment_only_commands() {
    let command = Node::Command(Box::new(SimpleCommand {
        tokens: SourceTokens::none(),
        line: SourceLine::new(1),
        assignments: vec![word(b"answer=42")],
        arguments: Vec::new(),
        redirections: Vec::new(),
    }));

    assert_eq!(render_command(&command), BString::from(b"answer=42"));
}

// [spec:posix:req:builtin.jobs.stdout-default-format/test]
// [spec:nsh:sem:idiom.specified-defects+1/test]
#[test]
fn job_text_includes_every_case_pattern() {
    let command = Node::Case(CaseCommand {
        tokens: SourceTokens::none(),
        line: SourceLine::new(1),
        word: Box::new(word(b"value")),
        clauses: vec![CaseClause {
            tokens: SourceTokens::none(),
            patterns: vec![word(b"first"), word(b"second")],
            body: Some(Box::new(Node::Command(Box::new(SimpleCommand {
                tokens: SourceTokens::none(),
                line: SourceLine::new(1),
                assignments: Vec::new(),
                arguments: vec![word(b"echo")],
                redirections: Vec::new(),
            })))),
            fallthrough: false,
        }],
    });

    assert_eq!(
        render_command(&command),
        BString::from(b"case value in first|second) echo;; esac")
    );
}
