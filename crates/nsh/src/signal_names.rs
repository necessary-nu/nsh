//! Signal names selected by the active platform backend.

pub const SIGNAL_SLOT_COUNT: usize = nsh_platform::SIGNAL_COUNT;
pub use nsh_platform::SIGNAL_NAMES;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_exit_and_a_sentinel() {
        assert_eq!(SIGNAL_NAMES.len(), SIGNAL_SLOT_COUNT + 1);
        assert_eq!(SIGNAL_NAMES[0].to_bytes(), b"EXIT");
        assert_eq!(SIGNAL_NAMES[SIGNAL_SLOT_COUNT].to_bytes(), b"");
    }
}
