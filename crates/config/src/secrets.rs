use std::io::Write;
use std::process::Command;

pub(crate) fn get_secret(key: &str) -> String {
    let out = Command::new("secret-tool")
        .args(["lookup", "app", "ira", "key", key])
        .output();
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        _ => String::new(),
    }
}

pub(crate) fn set_secret(key: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        let _ = Command::new("secret-tool")
            .args(["clear", "app", "ira", "key", key])
            .output();
        return Ok(());
    }
    let mut cmd = Command::new("secret-tool");
    cmd.args(["store", "--label=Ira Key", "app", "ira", "key", key]);
    cmd.stdin(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(value.as_bytes());
    }
    child.wait().map_err(|e| e.to_string())?;
    Ok(())
}
