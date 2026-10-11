//! rastai-codegen: the dev-only code generators (layout: rust-control-plane.md section 4.9). Each bin in src/bin/ is
//! a thin main over these modules. M1-12 lands the catalog generator; the formats and names generators join later.
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod catalog_javascript;
pub mod catalog_python;
pub mod catalog_ranges;
pub mod catalog_rust;
pub mod catalog_source;
pub mod error;
pub mod literals;
pub mod reference_docs;
pub mod repo_root;
pub mod write_if_changed;
