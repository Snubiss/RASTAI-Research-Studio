//! The migration chain as data: one row per (document type, version it upgrades from), naming its step function
//! (contracts 5.1, ADR-0032 decision 5). Every type is at version 1, so the table is empty until a type's v2 lands;
//! that PR adds `migrate_<type>_v1_to_v2` next to the type and its row here, with a fixture in
//! `fixtures/documents/<type>/v1/` (tests/migrations.rs).

use serde_json::Value;

/// One upgrade step, from `from_version` to the next version.
pub struct MigrationStep {
    pub type_name: &'static str,
    pub from_version: u32,
    /// Changes the fields; the runner writes the new `schema_version` itself (migrate/mod.rs). An `Err` holds the
    /// reason for `RRS-E4002`.
    pub migrate: fn(&Value) -> Result<Value, String>,
}

/// Every migration step, in any order.
pub const MIGRATION_STEPS: &[MigrationStep] = &[];

/// The step that upgrades this type from this version, if there is one.
pub fn find_step<'a>(steps: &'a [MigrationStep], type_name: &str, from_version: u32) -> Option<&'a MigrationStep> {
    steps
        .iter()
        .find(|step| step.type_name == type_name && step.from_version == from_version)
}
