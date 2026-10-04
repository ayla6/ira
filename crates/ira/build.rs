fn main() {
    glib_build_tools::compile_resources(
        &["resources"],
        "resources/ira.gresource.xml",
        "ira.gresource",
    );
    // Build identity for the startup log line: when a stale process or
    // binary is suspected, the log shows exactly what is running.
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "nogit".to_string());
    let dirty = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!("cargo:rustc-env=IRA_BUILD_HASH={hash}{}", if dirty { "-dirty" } else { "" });
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
}
