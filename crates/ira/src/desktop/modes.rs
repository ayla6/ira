use super::{integration, protocol::*, search_provider};

pub fn special_mode() -> Option<glib::ExitCode> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let provider = format!("--{PROVIDER_OPTION}");
    let register = format!("--{REGISTER_OPTION}");
    let unregister = format!("--{UNREGISTER_OPTION}");
    if !arguments.iter().any(|argument| {
        [provider.as_str(), register.as_str(), unregister.as_str()]
            .iter()
            .any(|flag| argument == *flag)
    }) {
        return None;
    }
    if arguments.len() != 1 {
        eprintln!("Desktop integration modes must be used alone");
        return Some(glib::ExitCode::FAILURE);
    }
    if arguments[0] == provider.as_str() {
        return Some(search_provider::run());
    }
    crate::i18n::init();
    let enabling = arguments[0] == register.as_str();
    let result = if enabling {
        integration::install_for_user()
    } else {
        integration::uninstall_for_user()
    };
    match result {
        Ok(()) => {
            let message = if enabling {
                crate::tr!(
                    "Desktop search installed using your existing launcher. GNOME may require a new login to discover it."
                )
            } else {
                crate::tr!("Desktop integration disabled. Individual game shortcuts were kept.")
            };
            println!("{message}");
            Some(glib::ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("Desktop integration: {error}");
            Some(glib::ExitCode::FAILURE)
        }
    }
}
