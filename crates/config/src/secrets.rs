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
        let status = Command::new("secret-tool")
            .args(["clear", "app", "ira", "key", key])
            .status()
            .map_err(|error| format!("failed to clear secret: {error}"))?;
        if !status.success() {
            return Err(format!("secret-tool failed to clear {key}"));
        }
        return Ok(());
    }
    let mut cmd = Command::new("secret-tool");
    cmd.args(["store", "--label=Ira Key", "app", "ira", "key", key]);
    cmd.stdin(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "secret-tool stdin was unavailable".to_string())?;
    stdin
        .write_all(value.as_bytes())
        .map_err(|error| format!("failed to write secret: {error}"))?;
    drop(stdin);
    let status = child.wait().map_err(|error| error.to_string())?;
    if !status.success() {
        return Err(format!("secret-tool failed to store {key}"));
    }
    Ok(())
}
