/// All Steam-supported languages with their API codes and English display names.
/// Used for language preference selection and display in the UI.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SteamLanguage {
    pub code: &'static str,
    pub english_name: &'static str,
    pub native_name: &'static str,
}

pub const STEAM_LANGUAGES: &[SteamLanguage] = &[
    SteamLanguage {
        code: "arabic",
        english_name: "Arabic",
        native_name: "العربية",
    },
    SteamLanguage {
        code: "bulgarian",
        english_name: "Bulgarian",
        native_name: "български език",
    },
    SteamLanguage {
        code: "schinese",
        english_name: "Chinese (Simplified)",
        native_name: "简体中文",
    },
    SteamLanguage {
        code: "tchinese",
        english_name: "Chinese (Traditional)",
        native_name: "繁體中文",
    },
    SteamLanguage {
        code: "czech",
        english_name: "Czech",
        native_name: "čeština",
    },
    SteamLanguage {
        code: "danish",
        english_name: "Danish",
        native_name: "Dansk",
    },
    SteamLanguage {
        code: "dutch",
        english_name: "Dutch",
        native_name: "Nederlands",
    },
    SteamLanguage {
        code: "english",
        english_name: "English",
        native_name: "English",
    },
    SteamLanguage {
        code: "finnish",
        english_name: "Finnish",
        native_name: "Suomi",
    },
    SteamLanguage {
        code: "french",
        english_name: "French",
        native_name: "Français",
    },
    SteamLanguage {
        code: "german",
        english_name: "German",
        native_name: "Deutsch",
    },
    SteamLanguage {
        code: "greek",
        english_name: "Greek",
        native_name: "Ελληνικά",
    },
    SteamLanguage {
        code: "hungarian",
        english_name: "Hungarian",
        native_name: "Magyar",
    },
    SteamLanguage {
        code: "indonesian",
        english_name: "Indonesian",
        native_name: "Bahasa Indonesia",
    },
    SteamLanguage {
        code: "italian",
        english_name: "Italian",
        native_name: "Italiano",
    },
    SteamLanguage {
        code: "japanese",
        english_name: "Japanese",
        native_name: "日本語",
    },
    SteamLanguage {
        code: "koreana",
        english_name: "Korean",
        native_name: "한국어",
    },
    SteamLanguage {
        code: "malay",
        english_name: "Malay",
        native_name: "Bahasa Melayu",
    },
    SteamLanguage {
        code: "norwegian",
        english_name: "Norwegian",
        native_name: "Norsk",
    },
    SteamLanguage {
        code: "polish",
        english_name: "Polish",
        native_name: "Polski",
    },
    SteamLanguage {
        code: "portuguese",
        english_name: "Portuguese",
        native_name: "Português",
    },
    SteamLanguage {
        code: "brazilian",
        english_name: "Portuguese-Brazil",
        native_name: "Português-Brasil",
    },
    SteamLanguage {
        code: "romanian",
        english_name: "Romanian",
        native_name: "Română",
    },
    SteamLanguage {
        code: "russian",
        english_name: "Russian",
        native_name: "Русский",
    },
    SteamLanguage {
        code: "spanish",
        english_name: "Spanish-Spain",
        native_name: "Español-España",
    },
    SteamLanguage {
        code: "latam",
        english_name: "Spanish-Latin America",
        native_name: "Español-Latinoamérica",
    },
    SteamLanguage {
        code: "swedish",
        english_name: "Swedish",
        native_name: "Svenska",
    },
    SteamLanguage {
        code: "thai",
        english_name: "Thai",
        native_name: "ไทย",
    },
    SteamLanguage {
        code: "turkish",
        english_name: "Turkish",
        native_name: "Türkçe",
    },
    SteamLanguage {
        code: "ukrainian",
        english_name: "Ukrainian",
        native_name: "Українська",
    },
    SteamLanguage {
        code: "vietnamese",
        english_name: "Vietnamese",
        native_name: "Tiếng Việt",
    },
];

/// Get the English display name for a Steam language code.
/// Returns the code itself if not found.
pub fn steam_language_name(code: &str) -> &str {
    STEAM_LANGUAGES
        .iter()
        .find(|l| l.code == code)
        .map(|l| l.english_name)
        .unwrap_or(code)
}
