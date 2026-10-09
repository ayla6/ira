//! Private shared helpers for the models crate.

/// `#[serde(default = "...")]` target for fields that default to `true`.
pub(crate) fn default_true() -> bool {
    true
}
