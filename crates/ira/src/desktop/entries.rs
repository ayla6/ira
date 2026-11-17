use super::protocol::*;

pub(crate) const MANAGED_KEY: &str = "X-Ira-Managed";

pub(crate) fn value(text: &str) -> String {
    text.replace('\0', "")
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn quoted_argument(text: &str) -> Result<String, String> {
    if text.contains(['\0', '\n', '\r']) {
        return Err("Executable path contains a control character".into());
    }
    let quoted = text
        .chars()
        .flat_map(|ch| {
            if matches!(ch, '\\' | '"' | '$' | '`') {
                vec!['\\', ch]
            } else {
                vec![ch]
            }
        })
        .collect::<String>();
    Ok(format!("\"{}\"", quoted.replace('\\', "\\\\")))
}

pub(crate) fn exec_argument(text: &str) -> Result<String, String> {
    quoted_argument(text).map(|argument| argument.replace('%', "%%"))
}

pub(crate) fn game_entry(
    executable: &str,
    name: &str,
    icon: &str,
    game: i64,
    variant: Option<i64>,
) -> Result<String, String> {
    let executable = exec_argument(executable)?;
    let variant = variant
        .map(|id| format!(" --{VARIANT_OPTION} {id}"))
        .unwrap_or_default();
    // Ira owns the window; the game shortcut is only a launch action.
    Ok(format!("[Desktop Entry]\nType=Application\nName={}\nExec={executable} --{LAUNCH_OPTION} {game}{variant}\nIcon={}\nTerminal=false\nCategories=Game;\nStartupNotify=false\n{MANAGED_KEY}=true\n", value(name), value(icon)))
}

pub(crate) fn provider_entry(desktop_id: &str) -> String {
    format!("[Shell Search Provider]\nDesktopId={}\nBusName={PROVIDER_ID}\nObjectPath={PROVIDER_PATH}\nVersion=2\nDefaultDisabled=false\n{MANAGED_KEY}=true\n", value(desktop_id))
}

pub(crate) fn service_entry(executable: &str) -> Result<String, String> {
    // The service file unescapes backslashes before parsing Exec arguments;
    // shell-style apostrophe escaping would make the entire service invalid.
    let executable = quoted_argument(executable)?;
    Ok(format!("[D-BUS Service]\nName={PROVIDER_ID}\nExec={executable} --{PROVIDER_OPTION}\n{MANAGED_KEY}=true\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_entry_escapes_names_and_keeps_launch_arguments() {
        let data = game_entry(
            "/home/My Apps/Ira %f.AppImage",
            "Title\nExec=bad",
            "/icons/a.png",
            12,
            Some(3),
        )
        .unwrap();
        let key = glib::KeyFile::new();
        key.load_from_data(&data, glib::KeyFileFlags::NONE).unwrap();
        assert_eq!(
            key.string("Desktop Entry", "Name").unwrap(),
            "Title\nExec=bad"
        );
        assert!(data.contains("Ira %%f.AppImage\" --launch-game 12 --variant 3"));
        assert_eq!(
            data.lines()
                .filter(|line| line.starts_with("Exec="))
                .count(),
            1
        );
        assert!(exec_argument("/x\nbad").is_err());
    }

    #[test]
    fn test_exec_argument_quotes_reserved_characters_at_both_levels() {
        let argument = exec_argument("/a\\b/$x`z\".AppImage").unwrap();
        let file = glib::KeyFile::new();
        file.load_from_data(
            &format!("[Desktop Entry]\nExec={argument}\n"),
            glib::KeyFileFlags::NONE,
        )
        .unwrap();
        let decoded = file.string("Desktop Entry", "Exec").unwrap();
        let args = glib::shell_parse_argv(decoded.as_str()).unwrap();
        assert_eq!(args[0], "/a\\b/$x`z\".AppImage");
    }

    #[test]
    fn test_service_entry_handles_quotes_and_literal_percent() {
        let path = "/home/It's My Apps/Ira %f \"$x\\.AppImage";
        let data = service_entry(path).unwrap();
        let key = glib::KeyFile::new();
        key.load_from_data(&data, glib::KeyFileFlags::NONE).unwrap();
        let exec = key.string("D-BUS Service", "Exec").unwrap();
        let args = glib::shell_parse_argv(exec.as_str()).unwrap();
        assert_eq!(args[0], path);
        assert_eq!(args[1], "--search-provider");
        assert!(provider_entry("ira.desktop").contains(&format!("BusName={PROVIDER_ID}\n")));
        assert!(provider_entry("ira.desktop").contains("DesktopId=ira.desktop\n"));
    }
}
