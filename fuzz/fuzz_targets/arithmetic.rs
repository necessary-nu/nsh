//! Arithmetic expansion checked against GNU Bash over bounded expressions.
//!
//! The generated script contains only decimal integers and a fixed operator.
//! The fuzzer chooses values and operations; it does not contribute shell
//! source text.

#![no_main]

mod support;

use libfuzzer_sys::arbitrary::{self, Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Less,
    Greater,
    Equal,
    NotEqual,
    BitAnd,
    BitOr,
    BitXor,
}

impl Op {
    fn text(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::Less => "<",
            Self::Greater => ">",
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Case {
    left: i16,
    right: i16,
    op: Op,
}

impl Case {
    fn script(&self) -> String {
        let left = i64::from(self.left).rem_euclid(2048) - 1024;
        let right = match self.op {
            Op::Div | Op::Mod => i64::from(self.right).rem_euclid(97) + 1,
            Op::Shl | Op::Shr => i64::from(self.right).rem_euclid(8),
            _ => i64::from(self.right).rem_euclid(2048) - 1024,
        };

        format!(
            "printf '[%s]\\n' \"$(( {left} {} {right} ))\"\n",
            self.op.text()
        )
    }
}

fuzz_target!(|data: &[u8]| {
    let mut unstructured = Unstructured::new(data);
    let Ok(case) = Case::arbitrary(&mut unstructured) else {
        return;
    };
    let script = case.script();

    support::assert_matches_bash("arithmetic", data, script.as_bytes(), Vec::new());
});
