pub const GETTEXT_PACKAGE: &str = "ira";

pub fn init() {
    gettextrs::setlocale(gettextrs::LocaleCategory::LcAll, "");
    gettextrs::bindtextdomain(
        GETTEXT_PACKAGE,
        std::env::var("IRA_LOCALEDIR").unwrap_or_else(|_| "/usr/share/locale".to_string()),
    )
    .expect("failed to bind Ira translation domain");
    gettextrs::textdomain(GETTEXT_PACKAGE).expect("failed to set Ira translation domain");
}

#[macro_export]
macro_rules! tr {
    ($message:literal) => {
        gettextrs::gettext($message)
    };
}
