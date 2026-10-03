//! `information_schema` view definitions for Vaultgres.
//!
//! PostgreSQL-compatible virtual views that expose catalog metadata in
//! the SQL-standard `information_schema` shape (ISO/IEC 9075-11).
//!
//! This module defines:
//! - The set of supported view names (see [INFORMATION_SCHEMA_VIEWS]).
//! - The column schema of each view (used by the planner to type-check
//!   projections and by the scan executor to populate tuples).
//!
//! The actual row materialization lives in
//! `crate::executor::operators::information_schema_scan`.
//!
//! Tracked by [issue #23](https://github.com/neoalienson/Vaultgres/issues/23).

use crate::catalog::TableSchema;
use crate::parser::ast::ColumnDef;

/// Tuple of `(column_name, postgresql_data_type_name)` for an
/// `information_schema` view column. The second element is the
/// PostgreSQL-style type label used for documentation/compatibility —
/// at runtime tuples are populated as `Value` and surfaced through
/// the protocol layer unchanged.
pub type ColumnInfo = (String, &'static str);

/// The set of `information_schema` views currently implemented.
///
/// Adding a new view requires:
/// 1. Add the name here.
/// 2. Add a `*_schema()` function below returning its columns.
/// 3. Add a `match` arm in
///    `InformationSchemaScanExecutor::try_new` to materialize rows.
pub const INFORMATION_SCHEMA_VIEWS: &[&str] = &[
    "schemata",
    "tables",
    "columns",
    "table_constraints",
    "referential_constraints",
];

/// Convenience: map a view name to its schema, if known.
pub fn view_to_schema(view: &str) -> Option<TableSchema> {
    match view {
        "schemata" => Some(schemata_schema_table()),
        "tables" => Some(tables_schema_table()),
        "columns" => Some(columns_schema_table()),
        "table_constraints" => Some(table_constraints_schema_table()),
        "referential_constraints" => Some(referential_constraints_schema_table()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Schema definitions
// ---------------------------------------------------------------------------

/// `information_schema.schemata` columns.
pub fn schemata_schema() -> Vec<ColumnInfo> {
    vec![
        ("catalog_name".into(), "sql_identifier"),
        ("schema_name".into(), "sql_identifier"),
        ("schema_owner".into(), "sql_identifier"),
        ("default_character_set_catalog".into(), "sql_identifier"),
        ("default_character_set_schema".into(), "sql_identifier"),
        ("default_character_set_name".into(), "sql_identifier"),
    ]
}

fn schemata_schema_table() -> TableSchema {
    let cols: Vec<ColumnDef> = schemata_schema()
        .into_iter()
        .map(|(n, _)| ColumnDef {
            name: n,
            data_type: crate::parser::ast::DataType::Text,
            is_primary_key: false,
            is_unique: false,
            is_auto_increment: false,
            is_not_null: false,
            default_value: None,
            foreign_key: None,
        })
        .collect();
    TableSchema::new("schemata".to_string(), cols)
}

/// `information_schema.tables` columns.
pub fn tables_schema() -> Vec<ColumnInfo> {
    vec![
        ("table_catalog".into(), "sql_identifier"),
        ("table_schema".into(), "sql_identifier"),
        ("table_name".into(), "sql_identifier"),
        ("table_type".into(), "character_data"),
    ]
}

fn tables_schema_table() -> TableSchema {
    let cols: Vec<ColumnDef> = tables_schema()
        .into_iter()
        .map(|(n, _)| ColumnDef {
            name: n,
            data_type: crate::parser::ast::DataType::Text,
            is_primary_key: false,
            is_unique: false,
            is_auto_increment: false,
            is_not_null: false,
            default_value: None,
            foreign_key: None,
        })
        .collect();
    TableSchema::new("tables".to_string(), cols)
}

/// `information_schema.columns` columns (abbreviated to the columns
/// Vaultgres actually populates today; remaining columns surface as NULL).
pub fn columns_schema() -> Vec<ColumnInfo> {
    vec![
        ("table_catalog".into(), "sql_identifier"),
        ("table_schema".into(), "sql_identifier"),
        ("table_name".into(), "sql_identifier"),
        ("column_name".into(), "sql_identifier"),
        ("ordinal_position".into(), "cardinal_number"),
        ("column_default".into(), "character_data"),
        ("is_nullable".into(), "yes_or_no"),
        ("data_type".into(), "character_data"),
        ("character_maximum_length".into(), "cardinal_number"),
        ("character_octet_length".into(), "cardinal_number"),
        ("numeric_precision".into(), "cardinal_number"),
        ("numeric_scale".into(), "cardinal_number"),
        ("datetime_precision".into(), "cardinal_number"),
        ("character_set_catalog".into(), "sql_identifier"),
        ("character_set_schema".into(), "sql_identifier"),
        ("character_set_name".into(), "sql_identifier"),
        ("collation_catalog".into(), "sql_identifier"),
        ("collation_schema".into(), "sql_identifier"),
        ("collation_name".into(), "sql_identifier"),
        ("domain_catalog".into(), "sql_identifier"),
        ("domain_schema".into(), "sql_identifier"),
        ("domain_name".into(), "sql_identifier"),
        ("udt_catalog".into(), "sql_identifier"),
        ("udt_schema".into(), "sql_identifier"),
        ("udt_name".into(), "sql_identifier"),
    ]
}

fn columns_schema_table() -> TableSchema {
    let cols: Vec<ColumnDef> = columns_schema()
        .into_iter()
        .map(|(n, _)| ColumnDef {
            name: n,
            data_type: crate::parser::ast::DataType::Text,
            is_primary_key: false,
            is_unique: false,
            is_auto_increment: false,
            is_not_null: false,
            default_value: None,
            foreign_key: None,
        })
        .collect();
    TableSchema::new("columns".to_string(), cols)
}

/// `information_schema.table_constraints` columns.
pub fn table_constraints_schema() -> Vec<ColumnInfo> {
    vec![
        ("constraint_catalog".into(), "sql_identifier"),
        ("constraint_schema".into(), "sql_identifier"),
        ("constraint_name".into(), "sql_identifier"),
        ("table_catalog".into(), "sql_identifier"),
        ("table_schema".into(), "sql_identifier"),
        ("table_name".into(), "sql_identifier"),
        ("constraint_type".into(), "character_data"),
        ("is_deferrable".into(), "yes_or_no"),
        ("initially_deferred".into(), "yes_or_no"),
        ("enforced".into(), "yes_or_no"),
        ("column_list".into(), "character_data"),
    ]
}

fn table_constraints_schema_table() -> TableSchema {
    let cols: Vec<ColumnDef> = table_constraints_schema()
        .into_iter()
        .map(|(n, _)| ColumnDef {
            name: n,
            data_type: crate::parser::ast::DataType::Text,
            is_primary_key: false,
            is_unique: false,
            is_auto_increment: false,
            is_not_null: false,
            default_value: None,
            foreign_key: None,
        })
        .collect();
    TableSchema::new("table_constraints".to_string(), cols)
}

/// `information_schema.referential_constraints` columns.
pub fn referential_constraints_schema() -> Vec<ColumnInfo> {
    vec![
        ("constraint_catalog".into(), "sql_identifier"),
        ("constraint_schema".into(), "sql_identifier"),
        ("constraint_name".into(), "sql_identifier"),
        ("unique_constraint_catalog".into(), "sql_identifier"),
        ("unique_constraint_schema".into(), "sql_identifier"),
        ("unique_constraint_name".into(), "sql_identifier"),
        ("match_option".into(), "character_data"),
        ("update_rule".into(), "character_data"),
        ("delete_rule".into(), "character_data"),
    ]
}

fn referential_constraints_schema_table() -> TableSchema {
    let cols: Vec<ColumnDef> = referential_constraints_schema()
        .into_iter()
        .map(|(n, _)| ColumnDef {
            name: n,
            data_type: crate::parser::ast::DataType::Text,
            is_primary_key: false,
            is_unique: false,
            is_auto_increment: false,
            is_not_null: false,
            default_value: None,
            foreign_key: None,
        })
        .collect();
    TableSchema::new("referential_constraints".to_string(), cols)
}

/// Re-export so the scan executor (and other modules) can call
/// the schema helpers without owning them via a trait.
#[allow(dead_code)]
pub fn column_info_schema_alias() -> Vec<ColumnInfo> {
    columns_schema()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_to_schema_recognizes_all_supported_views() {
        for v in INFORMATION_SCHEMA_VIEWS {
            assert!(
                view_to_schema(v).is_some(),
                "view '{}' should resolve to a schema",
                v
            );
        }
    }

    #[test]
    fn view_to_schema_rejects_unknown_view() {
        assert!(view_to_schema("pg_class").is_none());
        assert!(view_to_schema("").is_none());
    }

    #[test]
    fn schemata_has_required_columns() {
        let cols = schemata_schema();
        let names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"catalog_name"));
        assert!(names.contains(&"schema_name"));
        assert!(names.contains(&"schema_owner"));
    }

    #[test]
    fn tables_has_required_columns() {
        let cols = tables_schema();
        let names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"table_catalog"));
        assert!(names.contains(&"table_schema"));
        assert!(names.contains(&"table_name"));
        assert!(names.contains(&"table_type"));
    }

    #[test]
    fn columns_has_required_columns() {
        let cols = columns_schema();
        let names: Vec<&str> = cols.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"column_name"));
        assert!(names.contains(&"ordinal_position"));
        assert!(names.contains(&"data_type"));
        assert!(names.contains(&"is_nullable"));
    }
}