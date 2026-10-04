//! Private shared helpers for the models crate.

/// `#[serde(default = "...")]` target for fields that default to `true`.
/// Defined once here instead of as a one-line function in every struct
/// file that needs an opt-out boolean default.
pub(crate) fn default_true() -> bool {
    true
}
