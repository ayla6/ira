pub fn format_dll_overrides(overrides: &[(String, String)], desktop_integration: bool) -> String {
    let mut entries: Vec<String> = Vec::new();

    let mut user_set_winemenubuilder = false;
    for (dll, value) in overrides {
        if dll == "winemenubuilder" {
            user_set_winemenubuilder = true;
        }
        let normalized: String = value
            .split(',')
            .map(|token| {
                let t = token.trim();
                if t == "builtin" { "b" }
                else if t == "native" { "n" }
                else if t == "disabled" { "" }
                else { t }
            })
            .collect::<Vec<&str>>()
            .join(",");
        entries.push(format!("{}={}", dll, normalized));
    }
    if !user_set_winemenubuilder && !desktop_integration {
        entries.push("winemenubuilder=".to_string());
    }
    entries.join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_dll_overrides_empty() {
        let result = format_dll_overrides(&[], false);
        assert_eq!(result, "winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_enabled() {
        let result = format_dll_overrides(&[], true);
        assert_eq!(result, "");
    }

    #[test]
    fn test_format_dll_overrides_basic() {
        let overrides = vec![
            ("d3d11".to_string(), "native,builtin".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert_eq!(result, "d3d11=n,b;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_multiple() {
        let overrides = vec![
            ("d3d11".to_string(), "native,builtin".to_string()),
            ("d3d9".to_string(), "builtin,native".to_string()),
            ("winemenubuilder".to_string(), "".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert_eq!(result, "d3d11=n,b;d3d9=b,n;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_disabled() {
        let overrides = vec![
            ("d3d11".to_string(), "disabled".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert!(result.starts_with("d3d11=;"));
    }

    #[test]
    fn test_format_dll_overrides_native_only() {
        let overrides = vec![
            ("d3d11".to_string(), "native".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert_eq!(result, "d3d11=n;winemenubuilder=");
    }

    #[test]
    fn test_format_dll_overrides_builtin_only() {
        let overrides = vec![
            ("d3d11".to_string(), "builtin".to_string()),
        ];
        let result = format_dll_overrides(&overrides, false);
        assert_eq!(result, "d3d11=b;winemenubuilder=");
    }
}
