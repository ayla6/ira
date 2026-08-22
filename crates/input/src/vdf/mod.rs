//! Steam controller config (VDF) support: a tolerant KeyValues parser and
//! an importer that maps workshop layouts onto Ira's action-set model.
//! Import only — Ira's JSON stays the native storage format.

mod import;
mod parse;


pub use import::{import_vdf, ImportReport};

/// Read and import the VDF file at `path`; used by the CLI.
pub fn import_vdf_file(path: &std::path::Path) -> Result<(crate::InputProfile, ImportReport), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    import_vdf(&text)
}

use crate::InputSource;

/// Human-readable source name for warnings.
pub(crate) fn source_debug_name(source: InputSource) -> String {
    match source {
        InputSource::Button(button) => format!("button {button:?}"),
        InputSource::Axis(axis) => format!("axis {axis:?}"),
        InputSource::AxisDirection { axis, direction } => {
            format!("axis {axis:?} {direction:?}")
        }
    }
}
