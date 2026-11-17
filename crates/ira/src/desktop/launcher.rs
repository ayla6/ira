use gio::prelude::*;
use std::path::Path;

fn matches_command(command: &str, executable: &Path) -> bool {
    let Ok(arguments) = glib::shell_parse_argv(command) else {
        return false;
    };
    let Some(arguments) = arguments
        .iter()
        .map(|argument| argument.to_str())
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    let mut arguments = arguments.into_iter();
    let Some(mut program) = arguments.next() else {
        return false;
    };
    if Path::new(program)
        .file_name()
        .is_some_and(|name| name == "env")
    {
        let Some(next) =
            arguments.find(|argument| !argument.contains('=') && !argument.starts_with('-'))
        else {
            return false;
        };
        program = next;
    }
    let program = program.replace("%%", "%");
    let same_program = Path::new(&program) == executable
        || (program == "ira" && executable.file_name().is_some_and(|name| name == "ira"));
    same_program
        && !arguments.any(|argument| argument == "--launch-game" || argument == "--search-provider")
}

fn resolve(executable: &Path) -> Option<String> {
    let mut ids: Vec<_> = gio::AppInfo::all()
        .into_iter()
        .filter_map(|app| {
            let id = app.id()?;
            let command = app.commandline()?;
            if !app.should_show() || !matches_command(command.to_str()?, executable) {
                return None;
            }
            Some(id.to_string())
        })
        .collect();
    ids.sort();
    ids.into_iter().next()
}

pub(super) fn find(executable: &Path) -> Result<String, String> {
    resolve(executable).ok_or_else(|| crate::tr!("No installed Ira launcher was found. Register the AppImage with your app manager before enabling desktop search."))
}

pub fn resolve_window_identity() -> Result<Option<String>, String> {
    let executable = super::paths::executable()?;
    Ok(resolve(&executable).and_then(|id| window_id(&id).map(str::to_owned)))
}

fn window_id(desktop_id: &str) -> Option<&str> {
    desktop_id
        .strip_suffix(".desktop")
        .filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_id_uses_the_manager_filename_without_changing_the_bus_id() {
        assert_eq!(window_id("ira.desktop"), Some("ira"));
        assert_eq!(window_id("managed-ira.desktop"), Some("managed-ira"));
        assert_eq!(
            window_id("com.github.ira.desktop"),
            Some(super::super::protocol::APP_ID)
        );
        assert_eq!(window_id(".desktop"), None);
        assert_eq!(window_id("ira"), None);
    }

    #[test]
    fn test_matches_command_accepts_manager_env_wrapper_and_spaces() {
        let image = Path::new("/home/user/My Apps/ira.appimage");
        assert!(matches_command(
            "env DESKTOPINTEGRATION=1 '/home/user/My Apps/ira.appimage'",
            image
        ));
        assert!(matches_command("'/home/user/My Apps/ira.appimage'", image));
        assert!(!matches_command(
            "other '/home/user/My Apps/ira.appimage'",
            image
        ));
        assert!(!matches_command(
            "'/home/user/My Apps/ira.appimage' --launch-game 1",
            image
        ));
        assert!(!matches_command("'broken", image));
        assert!(matches_command(
            "'/home/user/Ira %%f.AppImage'",
            Path::new("/home/user/Ira %f.AppImage")
        ));
    }
}
