//! Maintenance tool for the DMCA notice-card filter: prints the 8x8
//! average hash of each given image using the exact same code path the
//! auto-downloader compares with. When SGDB serves a new card layout,
//! run this on it and add the hash to `DMCA_CARD_HASHES`.
//!
//! Usage: dmca_hash <image> [image...]

use ira_parser::image_average_hash;

fn main() {
    for path in std::env::args().skip(1) {
        match std::fs::read(&path) {
            Ok(bytes) => match image_average_hash(&bytes) {
                Some(hash) => println!("{path}: {hash:#018x}"),
                None => println!("{path}: undecodable"),
            },
            Err(e) => eprintln!("{path}: {e}"),
        }
    }
}
