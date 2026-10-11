//! rastai-codegen: the dev-only code generators (layout: rust-control-plane.md section 4.9). Each bin in src/bin/ is
//! a thin main over these modules. M1-12 landed the catalog generator and M1-16 the formats generator; the names
//! generator joins later (M1-57).
//!
//! This file is a list of `pub mod` lines (contracts 12.2).

pub mod catalog_javascript;
pub mod catalog_python;
pub mod catalog_ranges;
pub mod catalog_rust;
pub mod catalog_source;
pub mod command_source;
pub mod error;
pub mod javascript_commands;
pub mod javascript_types;
pub mod json_schema_files;
pub mod literals;
pub mod output_root;
pub mod python_dataclasses;
pub mod reference_docs;
pub mod repo_root;
pub mod type_modules;
pub mod write_if_changed;
