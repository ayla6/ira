//! Primitive operations on `Vec<(String, String)>` launch environments.
//! Leaf module: everything environment-related funnels through here so the
//! set-or-replace pattern is written once.

/// Sets `key` to `value`, replacing any previous entry.
pub(crate) fn env_set(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    env.retain(|(k, _)| k != key);
    env.push((key.to_string(), value.to_string()));
}

/// Prepends `value` to `key`, merging with an existing entry
/// colon-separated (LD_PRELOAD / LD_LIBRARY_PATH style).
pub(crate) fn env_prepend(env: &mut Vec<(String, String)>, key: &str, value: &str) {
    let merged = match env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()) {
        Some(prev) if !prev.is_empty() => format!("{}:{}", value, prev),
        _ => value.to_string(),
    };
    env_set(env, key, &merged);
}

pub(crate) fn has_exec(name: &str) -> bool {
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).find(|d| d.join(name).is_file()))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_set_replaces_existing_entry() {
        let mut env = vec![("PATH".to_string(), "/usr/bin".to_string())];
        env_set(&mut env, "PATH", "/bin");
        env_set(&mut env, "NEW", "1");
        assert_eq!(
            env,
            [
                ("PATH".to_string(), "/bin".to_string()),
                ("NEW".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn test_env_prepend_merges_colon_separated() {
        let mut env = vec![("LD_PRELOAD".to_string(), "/existing.so".to_string())];
        env_prepend(&mut env, "LD_PRELOAD", "/new.so");
        assert_eq!(
            env,
            [("LD_PRELOAD".to_string(), "/new.so:/existing.so".to_string())]
        );
        let mut env = vec![("LD_PRELOAD".to_string(), String::new())];
        env_prepend(&mut env, "LD_PRELOAD", "/new.so");
        assert_eq!(env, [("LD_PRELOAD".to_string(), "/new.so".to_string())]);
    }
}
