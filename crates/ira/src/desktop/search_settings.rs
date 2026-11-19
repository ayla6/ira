use gio::prelude::*;

fn settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup("org.gnome.desktop.search-providers", true)?;
    Some(gio::Settings::new_full(
        &schema,
        None::<&gio::SettingsBackend>,
        None,
    ))
}

fn provider_list(values: &[String], id: &str, include: bool) -> Vec<String> {
    let mut updated: Vec<_> = values
        .iter()
        .filter(|value| value.as_str() != id)
        .cloned()
        .collect();
    if include {
        updated.push(id.to_string());
    }
    updated
}

pub(super) fn set_enabled(id: &str, enabled: bool) -> Result<(), String> {
    let Some(settings) = settings() else {
        return Ok(());
    };
    // Shell reloads provider files on settings changes, rather than watching
    // provider directories. Re-enabling must trigger a reload after a new export.
    if enabled && settings.strv("enabled").iter().any(|value| value == id) {
        let values: Vec<_> = settings
            .strv("enabled")
            .iter()
            .map(ToString::to_string)
            .collect();
        settings
            .set_strv("enabled", provider_list(&values, id, false))
            .map_err(|error| {
                crate::tr!("Reload GNOME search providers: {}").replacen(
                    "{}",
                    &error.to_string(),
                    1,
                )
            })?;
        gio::Settings::sync();
    }
    settings.delay();
    for (key, include) in [("enabled", enabled), ("disabled", !enabled)] {
        let values: Vec<_> = settings.strv(key).iter().map(ToString::to_string).collect();
        let updated = provider_list(&values, id, include);
        if let Err(error) = settings.set_strv(key, updated.as_slice()) {
            settings.revert();
            return Err(format!("Update GNOME search settings: {error}"));
        }
    }
    settings.apply();
    gio::Settings::sync();
    Ok(())
}

pub(super) fn disabled(id: &str) -> bool {
    settings().is_some_and(|settings| {
        settings
            .strv("disabled")
            .iter()
            .any(|value| value.as_str() == id)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_list_changes_only_ira_and_avoids_duplicates() {
        let id = "appherder-ira.desktop".to_string();
        let values = vec!["other.desktop".into(), id.clone(), id.clone()];
        assert_eq!(provider_list(&values, &id, false), vec!["other.desktop"]);
        assert_eq!(
            provider_list(&values, &id, true),
            vec!["other.desktop".to_string(), id]
        );
        assert!(provider_list(&[], "ira.desktop", false).is_empty());
    }
}
