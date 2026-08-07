//! Literal ports of the build-time generators in `src/mk*.c`.
//! These are not part of the shell proper; they exist so every manifest
//! symbol has a target impl site and so the generated tables stay
//! derivable. See `docs/spec/port/src/mk*.md`.
//!
//! `mknodes` is the exception to "derivable": `crate::nodes` stopped being
//! its output when [dec:nsh:owned-data] made the parse tree an owned enum.
//! It still generates the C the reference build compiles. See that module.

pub mod mkinit;
pub mod mknodes;
pub mod mksignames;
pub mod mksyntax;
