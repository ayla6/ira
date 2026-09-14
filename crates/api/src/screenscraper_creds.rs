//! ScreenScraper credentials. The per-application developer pair is baked
//! into the binary XOR-scrambled — the same scheme ES-DE uses — so a lazy
//! scrape of the repo doesn't hand out the identity, and whoever wants it
//! has to decide they want *our* identity specifically. This is friction,
//! not encryption.

/// Reported to ScreenScraper so they can attribute the traffic.
pub const SOFT_NAME: &str = "ira";

/// XOR key for the scrambled developer pair. Not a secret; it only has to
/// be long enough to cycle over the values below.
const SCRAMBLE_KEY: &[u8] = &[
    73, 98, 155, 99, 174, 249, 37, 59, 186, 175, 195, 15, 150, 0, 125, 69, 118, 19, 80, 216, 222,
    78, 207, 12,
];

/// The scrambled `devid`, as printed by `print_dev_arrays` when the
/// ScreenScraper developer account was granted.
const DEV_ID_SCRAMBLED: &[u8] = &[40, 27, 247, 2];
/// The scrambled `devpassword`. To rotate: rerun `print_dev_arrays` with
/// the new values and paste what it prints.
const DEV_PASSWORD_SCRAMBLED: &[u8] = &[39, 17, 244, 14, 226, 160, 92, 124, 224, 249, 181];

/// Symmetric XOR against a cycling key, ES-DE's `scramble` generalized to
/// any input length.
fn scramble(input: &[u8], key: &[u8]) -> Vec<u8> {
    input
        .iter()
        .enumerate()
        .map(|(i, byte)| byte ^ key[i % key.len()])
        .collect()
}

fn unscramble(scrambled: &[u8]) -> String {
    String::from_utf8_lossy(&scramble(scrambled, SCRAMBLE_KEY)).into_owned()
}

/// The decoded developer pair baked into this build.
fn baked_dev_pair() -> (String, String) {
    (unscramble(DEV_ID_SCRAMBLED), unscramble(DEV_PASSWORD_SCRAMBLED))
}

/// ScreenScraper credentials, in the API's own two-pair model: the
/// per-application developer pair every request must carry, plus the
/// caller's own ScreenScraper account (`ssid`/`sspassword`), which is
/// optional and only attributes the request to that account's quota.
#[derive(Clone, Debug, Default)]
pub struct ScraperCreds {
    pub dev_id: String,
    pub dev_password: String,
    pub user: String,
    pub password: String,
}

impl ScraperCreds {
    /// Ira's identity: the baked-in developer pair plus the personal
    /// account login for quota attribution, when the user has one.
    pub fn from_account(user: String, password: String) -> Self {
        let (dev_id, dev_password) = baked_dev_pair();
        Self {
            dev_id,
            dev_password,
            user,
            password,
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.dev_id.is_empty() && !self.dev_password.is_empty()
    }

    /// The `devid`/`devpassword` pair plus the account login when set.
    pub(crate) fn auth_params(&self) -> String {
        let mut auth = format!(
            "devid={}&devpassword={}",
            crate::util::urlencode(&self.dev_id),
            crate::util::urlencode(&self.dev_password)
        );
        if !self.user.is_empty() && !self.password.is_empty() {
            auth.push_str(&format!(
                "&ssid={}&sspassword={}",
                crate::util::urlencode(&self.user),
                crate::util::urlencode(&self.password)
            ));
        }
        auth
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scramble_round_trips_against_the_key() {
        let scrambled = scramble("LeonSe".as_bytes(), SCRAMBLE_KEY);
        assert_eq!(unscramble(&scrambled), "LeonSe");
        // Longer than the key: the cycling must stay reversible.
        let long = "a-credentials-value-longer-than-the-key";
        let scrambled = scramble(long.as_bytes(), SCRAMBLE_KEY);
        assert_eq!(unscramble(&scrambled), long);
    }

    #[test]
    fn test_baked_dev_pair_is_configured() {
        let creds = ScraperCreds::from_account("aya".into(), "s3cret".into());
        assert_eq!(creds.user, "aya");
        assert!(creds.is_configured(), "the granted developer pair is baked in");
        // A corrupted paste would decode to junk; the pair must be sane text.
        for value in [&creds.dev_id, &creds.dev_password] {
            assert!(!value.is_empty());
            assert!(value.chars().all(|c| c.is_ascii_graphic()));
        }
    }

    /// Prints the scrambled arrays to paste into `DEV_ID_SCRAMBLED` and
    /// `DEV_PASSWORD_SCRAMBLED` once the developer account is granted:
    ///
    /// ```text
    /// SS_DEVID=... SS_DEVPASSWORD=... \
    ///   cargo test -p ira-api print_dev_arrays -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "helper for pasting newly granted developer credentials"]
    fn print_dev_arrays() {
        let devid = std::env::var("SS_DEVID").expect("SS_DEVID not set");
        let devpassword = std::env::var("SS_DEVPASSWORD").expect("SS_DEVPASSWORD not set");
        for (name, value) in [("DEV_ID_SCRAMBLED", devid), ("DEV_PASSWORD_SCRAMBLED", devpassword)] {
            let bytes = scramble(value.as_bytes(), SCRAMBLE_KEY);
            let list = bytes
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            println!("const {name}: &[u8] = &[{list}];");
        }
    }
}
