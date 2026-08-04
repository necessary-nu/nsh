//! Literal ports of the build-time generators in `src/mk*.c`.
//! These are not part of the shell proper; they exist so every manifest
//! symbol has a target impl site and so the generated tables stay
//! derivable. See `docs/spec/port/src/mk*.md`.

pub mod mkinit;
pub mod mknodes;
pub mod mksignames;
pub mod mksyntax;
