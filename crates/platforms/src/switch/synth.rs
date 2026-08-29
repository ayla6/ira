//! Synthetic encrypted NCA fixtures shared across the switch tests.

use aes::cipher::{KeyInit, KeyIvInit, StreamCipher};
use ctr::Ctr128BE;
use xts_mode::Xts128;

use super::nca::{HEADER_SIZE, SECTOR_SIZE};


pub(crate) const TEST_HEADER_KEY: [u8; 32] = [0x42; 32];
pub(crate) const TEST_KAEK: [u8; 16] = [0x57; 16];
/// The per-section key planted in key-area slot 2.
pub(crate) const TEST_BODY_KEY: [u8; 16] = [0x99; 16];

/// Builds a RomFS image whose root holds one `icon_en.dat`.
pub(crate) fn romfs_fixture(icon: &[u8]) -> Vec<u8> {
    let name = b"icon_en.dat";
    let padded = name.len().div_ceil(4) * 4;
    let meta_size = 0x20 + padded;
    let data_at = 0x50 + meta_size;
    let mut romfs = vec![0u8; data_at + icon.len()];
    romfs[0..8].copy_from_slice(&0x50u64.to_le_bytes());
    romfs[0x38..0x40].copy_from_slice(&0x50u64.to_le_bytes()); // meta table
    romfs[0x40..0x48].copy_from_slice(&(meta_size as u64).to_le_bytes());
    romfs[0x48..0x50].copy_from_slice(&(data_at as u64).to_le_bytes());
    romfs[0x50 + 8..0x50 + 16].copy_from_slice(&0u64.to_le_bytes()); // data offset
    romfs[0x50 + 16..0x50 + 24].copy_from_slice(&(icon.len() as u64).to_le_bytes());
    romfs[0x50 + 24..0x50 + 28].copy_from_slice(&0u32.to_le_bytes()); // next hash
    romfs[0x50 + 28..0x50 + 32].copy_from_slice(&(name.len() as u32).to_le_bytes());
    romfs[0x70..0x70 + name.len()].copy_from_slice(name);
    romfs[data_at..].copy_from_slice(icon);
    romfs
}

/// The `prod.keys` text matching the TEST_ constants above.
pub(crate) fn test_keys_text() -> String {
    format!(
        "header_key = {}\nkey_area_key_application_00 = {}\n",
        TEST_HEADER_KEY.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        TEST_KAEK.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    )
}

/// Builds a control NCA's bytes (header + encrypted section) whose
/// section 0 carries a RomFS with one `icon_en.dat` = `JPEGDATA`,
/// encrypted exactly the way a retail dump is: header under AES-XTS,
/// section body under AES-CTR with the key-area slot 2 key.
pub(crate) fn synthetic_control_nca(title_id: u64) -> Vec<u8> {
    let section_start = 0x4000u64;
    let romfs = romfs_fixture(b"JPEGDATA");
    let section_size = romfs.len() as u64;
    let mut header = [0u8; HEADER_SIZE];
    header[0x200..0x204].copy_from_slice(b"NCA3");
    header[0x205] = 2; // content type: control
    header[0x207] = 0; // key index: application
    header[0x210..0x218].copy_from_slice(&title_id.to_le_bytes());
    header[0x220] = 1; // key generation new → master_key_00
    let start_sector = (section_start / SECTOR_SIZE as u64) as u32;
    let end_sector = start_sector + (section_size.div_ceil(SECTOR_SIZE as u64)) as u32;
    header[0x240..0x244].copy_from_slice(&start_sector.to_le_bytes());
    header[0x244..0x248].copy_from_slice(&end_sector.to_le_bytes());
    header[0x404] = 3; // encryption type: AesCtr

    // Key area: four ECB-wrapped slots; bodies use slot 2.
    let kak_cipher = aes::Aes128::new_from_slice(&TEST_KAEK).unwrap();
    for slot in 0..4usize {
        let mut block = aes::Block::from([slot as u8; 16]);
        if slot == 2 {
            block = aes::Block::from(TEST_BODY_KEY);
        }
        use aes::cipher::BlockCipherEncrypt;
        kak_cipher.encrypt_block(&mut block);
        header[0x300 + slot * 16..0x300 + slot * 16 + 16].copy_from_slice(&block);
    }

    let sector_tweak =
        |sector: u128| -> xts_mode::Array<u8, aes::cipher::consts::U16> {
            xts_mode::Array(sector.to_be_bytes())
        };
    Xts128::<aes::Aes128>::new(
        aes::Aes128::new_from_slice(&TEST_HEADER_KEY[..16]).unwrap(),
        aes::Aes128::new_from_slice(&TEST_HEADER_KEY[16..]).unwrap(),
    )
    .encrypt_area(&mut header, SECTOR_SIZE, 0, sector_tweak);

    let mut nca = vec![0u8; section_start as usize];
    nca[..HEADER_SIZE].copy_from_slice(&header);
    // Sections occupy whole 0x200 sectors, so pad the body out to the
    // size the fs entry promises.
    let mut body = romfs;
    body.resize(body.len().div_ceil(SECTOR_SIZE) * SECTOR_SIZE, 0);
    let mut counter = [0u8; 16];
    counter[8..16].copy_from_slice(&(section_start / 16).to_be_bytes());
    Ctr128BE::<aes::Aes128>::new_from_slices(&TEST_BODY_KEY, &counter)
        .unwrap()
        .apply_keystream(&mut body);
    nca.extend_from_slice(&body);
    nca
}
