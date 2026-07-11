use std::os::unix::fs::PermissionsExt;

pub fn validate_executable(exe: &str) -> Result<(), String> {
    let path = std::path::Path::new(exe);
    if !path.is_file() {
        return Err(format!("Executable not found: {}", exe));
    }
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let perms = metadata.permissions();
    if perms.mode() & 0o111 == 0 {
        return Err(format!("Executable is not executable (chmod +x): {}", exe));
    }
    Ok(())
}

pub fn build_native_command(exe: &str, args: &[String]) -> Vec<String> {
    let mut cmd = vec![exe.to_string()];
    cmd.extend_from_slice(args);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn test_validate_executable_nonexistent() {
        let result = validate_executable("/nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn test_build_native_command_no_args() {
        let cmd = build_native_command("/usr/bin/game", &[]);
        assert_eq!(cmd, vec!["/usr/bin/game"]);
    }

    #[test]
    fn test_build_native_command_with_args() {
        let cmd = build_native_command("/usr/bin/game", &["--foo".to_string(), "bar".to_string()]);
        assert_eq!(cmd, vec!["/usr/bin/game", "--foo", "bar"]);
    }

    #[test]
    fn test_validate_executable_valid_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.sh");
        std::fs::write(&path, "#!/bin/sh").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let result = validate_executable(path.to_string_lossy().as_ref());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_executable_not_executable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test.txt");
        std::fs::write(&path, "hello").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let result = validate_executable(path.to_string_lossy().as_ref());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("chmod +x"));
    }
}
