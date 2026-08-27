//! Nintendo Switch rumble encoding, a faithful port of hid-nintendo's
//! tables and packing (`joycon_encode_rumble`): the firmware expects the
//! amplitude split across a frequency-carried word, and approximations get
//! silently ignored. The daemon replays game rumble on Switch-protocol pads
//! through [`rumble_output_report`].

/// (high word, low word, hertz) — hid-nintendo's `joycon_rumble_frequencies`.
const FREQUENCIES: [(u16, u16, u16); 159] = [
    (0x0000, 0x0001, 41),
    (0x0000, 0x0002, 42),
    (0x0000, 0x0003, 43),
    (0x0000, 0x0004, 44),
    (0x0000, 0x0005, 45),
    (0x0000, 0x0006, 46),
    (0x0000, 0x0007, 47),
    (0x0000, 0x0008, 48),
    (0x0000, 0x0009, 49),
    (0x0000, 0x000A, 50),
    (0x0000, 0x000B, 51),
    (0x0000, 0x000C, 52),
    (0x0000, 0x000D, 53),
    (0x0000, 0x000E, 54),
    (0x0000, 0x000F, 55),
    (0x0000, 0x0010, 57),
    (0x0000, 0x0011, 58),
    (0x0000, 0x0012, 59),
    (0x0000, 0x0013, 60),
    (0x0000, 0x0014, 62),
    (0x0000, 0x0015, 63),
    (0x0000, 0x0016, 64),
    (0x0000, 0x0017, 66),
    (0x0000, 0x0018, 67),
    (0x0000, 0x0019, 69),
    (0x0000, 0x001A, 70),
    (0x0000, 0x001B, 72),
    (0x0000, 0x001C, 73),
    (0x0000, 0x001D, 75),
    (0x0000, 0x001E, 77),
    (0x0000, 0x001F, 78),
    (0x0000, 0x0020, 80),
    (0x0400, 0x0021, 82),
    (0x0800, 0x0022, 84),
    (0x0C00, 0x0023, 85),
    (0x1000, 0x0024, 87),
    (0x1400, 0x0025, 89),
    (0x1800, 0x0026, 91),
    (0x1C00, 0x0027, 93),
    (0x2000, 0x0028, 95),
    (0x2400, 0x0029, 97),
    (0x2800, 0x002A, 99),
    (0x2C00, 0x002B, 102),
    (0x3000, 0x002C, 104),
    (0x3400, 0x002D, 106),
    (0x3800, 0x002E, 108),
    (0x3C00, 0x002F, 111),
    (0x4000, 0x0030, 113),
    (0x4400, 0x0031, 116),
    (0x4800, 0x0032, 118),
    (0x4C00, 0x0033, 121),
    (0x5000, 0x0034, 123),
    (0x5400, 0x0035, 126),
    (0x5800, 0x0036, 129),
    (0x5C00, 0x0037, 132),
    (0x6000, 0x0038, 135),
    (0x6400, 0x0039, 137),
    (0x6800, 0x003A, 141),
    (0x6C00, 0x003B, 144),
    (0x7000, 0x003C, 147),
    (0x7400, 0x003D, 150),
    (0x7800, 0x003E, 153),
    (0x7C00, 0x003F, 157),
    (0x8000, 0x0040, 160),
    (0x8400, 0x0041, 164),
    (0x8800, 0x0042, 167),
    (0x8C00, 0x0043, 171),
    (0x9000, 0x0044, 174),
    (0x9400, 0x0045, 178),
    (0x9800, 0x0046, 182),
    (0x9C00, 0x0047, 186),
    (0xA000, 0x0048, 190),
    (0xA400, 0x0049, 194),
    (0xA800, 0x004A, 199),
    (0xAC00, 0x004B, 203),
    (0xB000, 0x004C, 207),
    (0xB400, 0x004D, 212),
    (0xB800, 0x004E, 217),
    (0xBC00, 0x004F, 221),
    (0xC000, 0x0050, 226),
    (0xC400, 0x0051, 231),
    (0xC800, 0x0052, 236),
    (0xCC00, 0x0053, 241),
    (0xD000, 0x0054, 247),
    (0xD400, 0x0055, 252),
    (0xD800, 0x0056, 258),
    (0xDC00, 0x0057, 263),
    (0xE000, 0x0058, 269),
    (0xE400, 0x0059, 275),
    (0xE800, 0x005A, 281),
    (0xEC00, 0x005B, 287),
    (0xF000, 0x005C, 293),
    (0xF400, 0x005D, 300),
    (0xF800, 0x005E, 306),
    (0xFC00, 0x005F, 313),
    (0x0001, 0x0060, 320),
    (0x0401, 0x0061, 327),
    (0x0801, 0x0062, 334),
    (0x0C01, 0x0063, 341),
    (0x1001, 0x0064, 349),
    (0x1401, 0x0065, 357),
    (0x1801, 0x0066, 364),
    (0x1C01, 0x0067, 372),
    (0x2001, 0x0068, 381),
    (0x2401, 0x0069, 389),
    (0x2801, 0x006A, 397),
    (0x2C01, 0x006B, 406),
    (0x3001, 0x006C, 415),
    (0x3401, 0x006D, 424),
    (0x3801, 0x006E, 433),
    (0x3C01, 0x006F, 443),
    (0x4001, 0x0070, 453),
    (0x4401, 0x0071, 462),
    (0x4801, 0x0072, 473),
    (0x4C01, 0x0073, 483),
    (0x5001, 0x0074, 494),
    (0x5401, 0x0075, 504),
    (0x5801, 0x0076, 515),
    (0x5C01, 0x0077, 527),
    (0x6001, 0x0078, 538),
    (0x6401, 0x0079, 550),
    (0x6801, 0x007A, 562),
    (0x6C01, 0x007B, 574),
    (0x7001, 0x007C, 587),
    (0x7401, 0x007D, 600),
    (0x7801, 0x007E, 613),
    (0x7C01, 0x007F, 626),
    (0x8001, 0x0000, 640),
    (0x8401, 0x0000, 654),
    (0x8801, 0x0000, 668),
    (0x8C01, 0x0000, 683),
    (0x9001, 0x0000, 698),
    (0x9401, 0x0000, 713),
    (0x9801, 0x0000, 729),
    (0x9C01, 0x0000, 745),
    (0xA001, 0x0000, 761),
    (0xA401, 0x0000, 778),
    (0xA801, 0x0000, 795),
    (0xAC01, 0x0000, 812),
    (0xB001, 0x0000, 830),
    (0xB401, 0x0000, 848),
    (0xB801, 0x0000, 867),
    (0xBC01, 0x0000, 886),
    (0xC001, 0x0000, 905),
    (0xC401, 0x0000, 925),
    (0xC801, 0x0000, 945),
    (0xCC01, 0x0000, 966),
    (0xD001, 0x0000, 987),
    (0xD401, 0x0000, 1009),
    (0xD801, 0x0000, 1031),
    (0xDC01, 0x0000, 1053),
    (0xE001, 0x0000, 1076),
    (0xE401, 0x0000, 1100),
    (0xE801, 0x0000, 1124),
    (0xEC01, 0x0000, 1149),
    (0xF001, 0x0000, 1174),
    (0xF401, 0x0000, 1199),
    (0xF801, 0x0000, 1226),
    (0xFC01, 0x0000, 1253),
];

/// (high addend, low word, amplitude code ceiling) — hid-nintendo's
/// `joycon_rumble_amplitudes`.
const AMPLITUDES: [(u8, u16, u16); 100] = [
    (0x00, 0x0040, 0),
    (0x02, 0x8040, 10),
    (0x04, 0x0041, 12),
    (0x06, 0x8041, 14),
    (0x08, 0x0042, 17),
    (0x0A, 0x8042, 20),
    (0x0C, 0x0043, 24),
    (0x0E, 0x8043, 28),
    (0x10, 0x0044, 33),
    (0x12, 0x8044, 40),
    (0x14, 0x0045, 47),
    (0x16, 0x8045, 56),
    (0x18, 0x0046, 67),
    (0x1A, 0x8046, 80),
    (0x1C, 0x0047, 95),
    (0x1E, 0x8047, 112),
    (0x20, 0x0048, 117),
    (0x22, 0x8048, 123),
    (0x24, 0x0049, 128),
    (0x26, 0x8049, 134),
    (0x28, 0x004A, 140),
    (0x2A, 0x804A, 146),
    (0x2C, 0x004B, 152),
    (0x2E, 0x804B, 159),
    (0x30, 0x004C, 166),
    (0x32, 0x804C, 173),
    (0x34, 0x004D, 181),
    (0x36, 0x804D, 189),
    (0x38, 0x004E, 198),
    (0x3A, 0x804E, 206),
    (0x3C, 0x004F, 215),
    (0x3E, 0x804F, 225),
    (0x40, 0x0050, 230),
    (0x42, 0x8050, 235),
    (0x44, 0x0051, 240),
    (0x46, 0x8051, 245),
    (0x48, 0x0052, 251),
    (0x4A, 0x8052, 256),
    (0x4C, 0x0053, 262),
    (0x4E, 0x8053, 268),
    (0x50, 0x0054, 273),
    (0x52, 0x8054, 279),
    (0x54, 0x0055, 286),
    (0x56, 0x8055, 292),
    (0x58, 0x0056, 298),
    (0x5A, 0x8056, 305),
    (0x5C, 0x0057, 311),
    (0x5E, 0x8057, 318),
    (0x60, 0x0058, 325),
    (0x62, 0x8058, 332),
    (0x64, 0x0059, 340),
    (0x66, 0x8059, 347),
    (0x68, 0x005A, 355),
    (0x6A, 0x805A, 362),
    (0x6C, 0x005B, 370),
    (0x6E, 0x805B, 378),
    (0x70, 0x005C, 387),
    (0x72, 0x805C, 395),
    (0x74, 0x005D, 404),
    (0x76, 0x805D, 413),
    (0x78, 0x005E, 422),
    (0x7A, 0x805E, 431),
    (0x7C, 0x005F, 440),
    (0x7E, 0x805F, 450),
    (0x80, 0x0060, 460),
    (0x82, 0x8060, 470),
    (0x84, 0x0061, 480),
    (0x86, 0x8061, 491),
    (0x88, 0x0062, 501),
    (0x8A, 0x8062, 512),
    (0x8C, 0x0063, 524),
    (0x8E, 0x8063, 535),
    (0x90, 0x0064, 547),
    (0x92, 0x8064, 559),
    (0x94, 0x0065, 571),
    (0x96, 0x8065, 584),
    (0x98, 0x0066, 596),
    (0x9A, 0x8066, 609),
    (0x9C, 0x0067, 623),
    (0x9E, 0x8067, 636),
    (0xA0, 0x0068, 650),
    (0xA2, 0x8068, 665),
    (0xA4, 0x0069, 679),
    (0xA6, 0x8069, 694),
    (0xA8, 0x006A, 709),
    (0xAA, 0x806A, 725),
    (0xAC, 0x006B, 741),
    (0xAE, 0x806B, 757),
    (0xB0, 0x006C, 773),
    (0xB2, 0x806C, 790),
    (0xB4, 0x006D, 808),
    (0xB6, 0x806D, 825),
    (0xB8, 0x006E, 843),
    (0xBA, 0x806E, 862),
    (0xBC, 0x006F, 881),
    (0xBE, 0x806F, 900),
    (0xC0, 0x0070, 920),
    (0xC2, 0x8070, 940),
    (0xC4, 0x0071, 960),
    (0xC6, 0x8071, 981),
];

/// hid-nintendo's default motor frequencies.
pub const DEFAULT_LOW_FREQ: u16 = 160;
pub const DEFAULT_HIGH_FREQ: u16 = 320;
/// `joycon_max_rumble_amp`: evdev's 0..=65535 maps onto this code range.
const MAX_AMP_CODE: u32 = 1003;

/// The kernel's frequency lookup: the first entry whose ceiling covers the
/// request, clamped to the last when nothing does.
fn find_freq(freq: u16) -> (u16, u16) {
    let mut index = 0;
    if freq > FREQUENCIES[0].2 {
        index = FREQUENCIES.len() - 1;
        for candidate in 1..FREQUENCIES.len() - 1 {
            if freq > FREQUENCIES[candidate - 1].2 && freq <= FREQUENCIES[candidate].2 {
                index = candidate;
                break;
            }
        }
    }
    (FREQUENCIES[index].0, FREQUENCIES[index].1)
}

/// The kernel's amplitude lookup over the same search shape.
fn find_amp(amp: u16) -> (u8, u16) {
    let mut index = 0;
    if amp > AMPLITUDES[0].2 {
        index = AMPLITUDES.len() - 1;
        for candidate in 1..AMPLITUDES.len() - 1 {
            if amp > AMPLITUDES[candidate - 1].2 && amp <= AMPLITUDES[candidate].2 {
                index = candidate;
                break;
            }
        }
    }
    (AMPLITUDES[index].0, AMPLITUDES[index].1)
}

/// One motor's four wire bytes, packed exactly as `joycon_encode_rumble`.
fn encode_half(freq_low: u16, freq_high: u16, amp: u16) -> [u8; 4] {
    let (high_word, _) = find_freq(freq_high);
    let (_, low_word) = find_freq(freq_low);
    let (amp_high, amp_low) = find_amp(amp);
    [
        (high_word >> 8) as u8,
        (high_word & 0xFF) as u8 + amp_high,
        low_word as u8 + (amp_low >> 8) as u8,
        (amp_low & 0xFF) as u8,
    ]
}

/// The rumble-only output report (0x10): timer, then the strong (left)
/// motor's half followed by the weak (right) motor's.
pub fn rumble_output_report(timer: u8, strong: u16, weak: u16) -> [u8; 10] {
    let left = encode_half(
        DEFAULT_LOW_FREQ,
        DEFAULT_HIGH_FREQ,
        amplitude_code(strong),
    );
    let right = encode_half(
        DEFAULT_LOW_FREQ,
        DEFAULT_HIGH_FREQ,
        amplitude_code(weak),
    );
    [
        0x10, timer, left[0], left[1], left[2], left[3], right[0], right[1], right[2], right[3],
    ]
}

/// evdev magnitude onto the firmware's amplitude code range.
fn amplitude_code(magnitude: u16) -> u16 {
    (u32::from(magnitude) * MAX_AMP_CODE / 65_535) as u16
}

#[cfg(test)]
mod tests {
    use super::{find_amp, find_freq, rumble_output_report};

    #[test]
    fn test_frequency_lookup_matches_kernel_boundaries() {
        assert_eq!(find_freq(41), (0x0000, 0x0001));
        assert_eq!(find_freq(160), (0x8000, 0x0040));
        assert_eq!(find_freq(320), (0x0001, 0x0060));
        // Above the table the last entry applies, never a panic.
        assert_eq!(find_freq(2000), (0xFC01, 0x0000));
    }

    #[test]
    fn test_amplitude_lookup_matches_kernel_boundaries() {
        assert_eq!(find_amp(0), (0x00, 0x0040));
        assert_eq!(find_amp(10), (0x02, 0x8040));
        // Above the ceiling the strongest entry applies.
        let (high, _) = find_amp(1200);
        assert_eq!(high, 198);
    }

    #[test]
    fn test_full_strength_report_decodes_back_near_full() {
        // The daemon's twin decoder approximates report byte 3 as the
        // amplitude code scaled by 200; full strength must land close to
        // full evdev magnitude and monotonicity must hold down the range.
        let full = rumble_output_report(1, 65_535, 65_535);
        assert_eq!(full[0], 0x10);
        let decoded_strong = u32::from(full[3] & 0xFE) * 65_535 / 200;
        assert!(decoded_strong > 60_000, "decoded {decoded_strong}");
        let half = rumble_output_report(2, 32_767, 0);
        let decoded_half = u32::from(half[3] & 0xFE) * 65_535 / 200;
        assert!(decoded_half < decoded_strong);
        // Zero magnitude must decode to exactly zero: that is the stop
        // command the firmware honors.
        assert_eq!(rumble_output_report(3, 0, 0)[3] & 0xFE, 0);
        assert_eq!(rumble_output_report(3, 0, 0)[7] & 0xFE, 0);
    }
}
