//! Temporary check: print SHELL_ALIGN(size_of) for every node struct so it
//! can be diffed against the C. Not part of the shell.
use dash::nodes::*;
const fn align(n: usize) -> usize { (n + 7) & !7 }
macro_rules! p { ($t:ty, $n:expr) => { println!("{} {}", $n, align(core::mem::size_of::<$t>())) } }
fn main() {
    p!(ncmd,"ncmd"); p!(npipe,"npipe"); p!(nredir,"nredir"); p!(nbinary,"nbinary");
    p!(nif,"nif"); p!(nfor,"nfor"); p!(ncase,"ncase"); p!(nclist,"nclist");
    p!(narg,"narg"); p!(nfile,"nfile"); p!(ndup,"ndup"); p!(nhere,"nhere"); p!(nnot,"nnot");
    println!("node {}", core::mem::size_of::<node>());
}
