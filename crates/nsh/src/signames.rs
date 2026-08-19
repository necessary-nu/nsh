//! Signal names selected by the active platform backend.

pub const NSIG: usize = nsh_platform::SIGNAL_COUNT;
pub const LASTSIG: usize = NSIG - 1;
pub use nsh_platform::SIGNAL_NAMES as signal_names;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_exit_and_a_sentinel() {
        assert_eq!(signal_names.len(), NSIG + 1);
        assert_eq!(signal_names[0].to_bytes(), b"EXIT");
        assert_eq!(signal_names[NSIG].to_bytes(), b"");
    }
}
