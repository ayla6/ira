//! Steam controller config (VDF) support: a tolerant KeyValues parser and
//! an importer that maps workshop layouts onto Ira's action-set model.
//! Import only — Ira's JSON stays the native storage format.

mod import;
mod parse;

pub use import::{import_vdf, import_vdf_file, ImportReport};
