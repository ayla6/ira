//! Filter option tables and the filter card for the Steam layout search,
//! mirroring steaminputdb's search form: one controller kind list and the
//! workshop feature tags, offered as require/exclude sets.

use super::css::CSS_BOXED_LIST;
use super::helpers::string_list_from;
use adw::prelude::*;
use ira_api::steam_input::SteamLayoutSort;
use std::sync::OnceLock;

/// (display label, workshop tag) for every controller kind Valve
/// distinguishes on layout tags — steaminputdb's CONTROLLER_LIST. Cached:
/// the table is immutable but consulted for every row subtitle, filter
/// build and query.
pub(super) fn controller_filter_options() -> &'static [(String, String)] {
    static OPTIONS: OnceLock<Vec<(String, String)>> = OnceLock::new();
    OPTIONS.get_or_init(|| {
        vec![
            (crate::tr!("Steam Controller"), "controller_triton".into()),
            (
                crate::tr!("Steam Controller (2015)"),
                "controller_steamcontroller_gordon".into(),
            ),
            (crate::tr!("Steam Deck"), "controller_neptune".into()),
            (crate::tr!("DualSense"), "controller_ps5".into()),
            (crate::tr!("DualShock 4"), "controller_ps4".into()),
            (crate::tr!("Xbox 360"), "controller_xbox360".into()),
            (crate::tr!("Xbox One / Elite"), "controller_xboxone".into()),
            (crate::tr!("Xbox Elite"), "controller_xboxelite".into()),
            (crate::tr!("Switch Pro"), "controller_switch_pro".into()),
            (crate::tr!("Switch 2 Pro"), "controller_switch2_pro".into()),
            (crate::tr!("8BitDo"), "controller_8bitdo".into()),
            (crate::tr!("Generic"), "controller_generic".into()),
        ]
    })
}

pub(super) fn controller_display_label(tag: &str) -> String {
    let kind = tag.trim_start_matches("controller_");
    controller_filter_options()
        .iter()
        .find(|(_, filter_tag)| filter_tag == kind)
        .map(|(label, _)| label.clone())
        .unwrap_or_else(|| capitalize(kind))
}

/// (display label, workshop tag) for the filterable layout features. The
/// keyboard tag really is misspelled `feature_keboard` in Valve's data.
pub(super) fn feature_filter_options() -> Vec<(String, String)> {
    vec![
        (crate::tr!("Gamepad"), "feature_gamepad".into()),
        (crate::tr!("Keyboard"), "feature_keboard".into()),
        (crate::tr!("Mouse"), "feature_mouse".into()),
        (crate::tr!("Gyro"), "feature_gyro".into()),
        (crate::tr!("Touch menus"), "feature_touchmenu".into()),
        (crate::tr!("Radial menus"), "feature_radialmenu".into()),
        (crate::tr!("Mode shifts"), "feature_modeshift".into()),
        (crate::tr!("Mouse regions"), "feature_mouseregion".into()),
        (crate::tr!("Action sets"), "feature_actionset".into()),
    ]
}

fn sort_label(sort: SteamLayoutSort) -> String {
    match sort {
        SteamLayoutSort::Rank => crate::tr!("Rank"),
        SteamLayoutSort::PublicationDate => crate::tr!("Date"),
        SteamLayoutSort::Trending30Days => crate::tr!("Trending (30 days)"),
        SteamLayoutSort::TotalSubscriptions => crate::tr!("Most subscribed"),
        SteamLayoutSort::VotesUp => crate::tr!("Most upvoted"),
        SteamLayoutSort::TextSearch => crate::tr!("Relevance"),
    }
}

/// A feature switch with its workshop tag; kept so the dialog can collect
/// the active tags per requirement direction.
pub(super) struct FeatureRow {
    pub tag: String,
    pub switch: adw::SwitchRow,
}

/// The boxed filter card: sort, controller kind, require/exclude feature
/// expanders, and the optional this-game-only scope. Rows re-submitting the
/// search is the caller's job (it needs the dialog context).
pub(super) struct FilterCard {
    pub list: gtk4::ListBox,
    pub sort_row: adw::ComboRow,
    pub controller_row: adw::ComboRow,
    pub include_rows: Vec<FeatureRow>,
    pub exclude_rows: Vec<FeatureRow>,
    pub app_only_row: adw::SwitchRow,
}

pub(super) fn build_filter_card(sorts: &[SteamLayoutSort], scoped_to_game: bool) -> FilterCard {
    let filters = gtk4::ListBox::new();
    filters.add_css_class(CSS_BOXED_LIST);
    filters.set_selection_mode(gtk4::SelectionMode::None);

    let sort_labels: Vec<String> = sorts.iter().map(|sort| sort_label(*sort)).collect();
    let sort_row = adw::ComboRow::new();
    sort_row.set_title(&crate::tr!("Sort by"));
    sort_row.set_model(Some(&string_list_from(&sort_labels)));
    filters.append(&sort_row);

    let mut controller_labels = vec![crate::tr!("Any controller")];
    controller_labels.extend(
        controller_filter_options()
            .iter()
            .map(|(label, _)| label.clone()),
    );
    let controller_row = adw::ComboRow::new();
    controller_row.set_title(&crate::tr!("Controller"));
    controller_row.set_model(Some(&string_list_from(&controller_labels)));
    filters.append(&controller_row);

    let include_rows = feature_expander(&filters, &crate::tr!("Must have features"));
    let exclude_rows = feature_expander(&filters, &crate::tr!("Must not have features"));

    // A known Steam app id scopes the query to that game's pool; everything
    // else searches all workshop layouts by text only.
    let app_only_row = adw::SwitchRow::new();
    app_only_row.set_title(&crate::tr!("This game only"));
    app_only_row.set_subtitle(&crate::tr!("Only layouts published for this game"));
    app_only_row.set_active(scoped_to_game);
    app_only_row.set_visible(scoped_to_game);
    filters.append(&app_only_row);

    FilterCard {
        list: filters,
        sort_row,
        controller_row,
        include_rows,
        exclude_rows,
        app_only_row,
    }
}

fn feature_expander(group: &gtk4::ListBox, title: &str) -> Vec<FeatureRow> {
    let expander = adw::ExpanderRow::new();
    expander.set_title(title);
    let rows: Vec<FeatureRow> = feature_filter_options()
        .into_iter()
        .map(|(label, tag)| {
            let switch = adw::SwitchRow::new();
            switch.set_title(&label);
            expander.add_row(&switch);
            FeatureRow { tag, switch }
        })
        .collect();
    group.append(&expander);
    rows
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_controller_filter_options_unique_prefixed_tags() {
        let options = controller_filter_options();
        assert!(!options.is_empty());
        let tags: HashSet<&str> = options.iter().map(|(_, tag)| tag.as_str()).collect();
        assert_eq!(tags.len(), options.len(), "workshop tags must be unique");
        assert!(options
            .iter()
            .all(|(_, tag)| tag.starts_with("controller_")));
    }

    #[test]
    fn test_controller_filter_options_cached_in_place() {
        let a = controller_filter_options().as_ptr();
        let b = controller_filter_options().as_ptr();
        assert!(std::ptr::eq(a, b), "must return the same cached table");
    }

    #[test]
    fn test_controller_display_label_known_tag() {
        // Pins existing display behavior: table tags carry the `controller_`
        // prefix while lookups compare against the stripped kind, so today
        // every tag lands on the capitalized fallback.
        assert_eq!(controller_display_label("controller_neptune"), "Neptune");
    }

    #[test]
    fn test_controller_display_label_unknown_falls_back_capitalized() {
        assert_eq!(controller_display_label("controller_wii"), "Wii");
        assert_eq!(controller_display_label("wii"), "Wii");
    }

    #[test]
    fn test_capitalize_variants() {
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("deck"), "Deck");
        assert_eq!(capitalize("steam deck"), "Steam deck");
    }
}
