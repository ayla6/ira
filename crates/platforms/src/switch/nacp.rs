//! `control.nacp` parsing: sixteen language slots of 0x300 bytes, each
//! starting with the localized application name as a NUL-terminated
//! UTF-8 string (layout per switchbrew.org/wiki/NACP_Format). Slot order
//! is fixed: AmericanEnglish first, then the rest of the console's
//! language list.

/// One language slot in the application title table.
const SLOT_SIZE: usize = 0x300;
/// The name field at the start of each slot.
const NAME_SIZE: usize = 0x200;
/// The table always holds sixteen language slots.
const LANGUAGE_SLOTS: usize = 16;

/// The display title: the first language slot in NACP order
/// (AmericanEnglish first) whose name is non-empty valid UTF-8. Empty,
/// whitespace-only, and invalid-UTF-8 slots are skipped; `None` when no
/// slot yields a usable name.
pub fn display_title(nacp: &[u8]) -> Option<String> {
    (0..LANGUAGE_SLOTS).find_map(|slot| slot_name(nacp, slot).map(str::to_string))
}

/// The trimmed name in one slot, or `None` when the slot does not fit
/// the buffer, is empty, or is not valid UTF-8.
fn slot_name(nacp: &[u8], slot: usize) -> Option<&str> {
    let at = slot * SLOT_SIZE;
    let name = nacp.get(at..at + NAME_SIZE)?;
    let end = name.iter().position(|b| *b == 0).unwrap_or(NAME_SIZE);
    std::str::from_utf8(&name[..end])
        .ok()
        .map(str::trim)
        .filter(|title| !title.is_empty())
}

/// Test fixture: a full NACP table with the given `(slot, name)` entries
/// written at their slot starts, everything else zeroed.
#[cfg(test)]
pub(crate) fn test_table(titles: &[(usize, &str)]) -> Vec<u8> {
    let mut out = vec![0u8; SLOT_SIZE * LANGUAGE_SLOTS];
    for (slot, name) in titles {
        let at = slot * SLOT_SIZE;
        out[at..at + name.len()].copy_from_slice(name.as_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_title_first_non_empty_slot_wins() {
        let nacp = test_table(&[(0, "American Name"), (2, "Japanese Name")]);
        assert_eq!(display_title(&nacp), Some("American Name".to_string()));

        let nacp = test_table(&[(1, "British Name"), (2, "Japanese Name")]);
        assert_eq!(display_title(&nacp), Some("British Name".to_string()));
    }

    #[test]
    fn test_display_title_stops_at_nul_padding() {
        let mut nacp = test_table(&[(0, "Real Name")]);
        // Trailing junk after the NUL inside the name field is ignored.
        nacp[10..0x40].fill(b'X');
        assert_eq!(display_title(&nacp), Some("Real Name".to_string()));
    }

    #[test]
    fn test_display_title_skips_invalid_utf8_slots() {
        let mut nacp = test_table(&[(1, "Valid Name")]);
        nacp[0] = 0xff;
        nacp[1] = 0xfe;
        assert_eq!(display_title(&nacp), Some("Valid Name".to_string()));
        // No valid slot anywhere: None.
        nacp[0x300] = 0xff;
        assert_eq!(display_title(&nacp), None);
    }

    #[test]
    fn test_display_title_treats_whitespace_as_empty() {
        let nacp = test_table(&[(0, "   "), (1, "British Name")]);
        assert_eq!(display_title(&nacp), Some("British Name".to_string()));
    }

    #[test]
    fn test_display_title_empty_table_is_none() {
        assert_eq!(display_title(&test_table(&[])), None);
        assert_eq!(display_title(&[]), None);
    }

    #[test]
    fn test_display_title_reads_last_fitting_slot() {
        // Only slot 15 fits the buffer; slots 0–14 are skipped whole.
        let nacp = test_table(&[(15, "Last Slot")]);
        assert_eq!(display_title(&nacp), Some("Last Slot".to_string()));

        let truncated = test_table(&[(15, "Last Slot")]);
        let truncated = &truncated[..0x300 * 15 + 0x100];
        assert_eq!(display_title(truncated), None);
    }
}
