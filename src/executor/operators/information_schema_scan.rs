//! `information_schema` virtual-table scan executor.
//!
//! Resolves a `SELECT ... FROM information_schema.<view>` against the
//! catalog metadata (in-memory `Catalog` state) and returns tuples that
//! conform to the SQL-standard `information_schema` shape.
//!
//! PostgreSQL-compatible view names recognized here:
//! - `information_schema.schemata`
//! - `information_schema.tables`
//! - `information_schema.columns`
//! - `information_schema.table_constraints`
//! - `information_schema.referential_constraints`
//!
//! Tuple shape follows ISO/IEC 9075-11 information_schema definitions
//! (as implemented by PostgreSQL). Columns marked TODO are returned as
//! NULL with a `// todo: <reason>` comment in the source.

use super::executor::{Executor, ExecutorError, Tuple};
use crate::catalog::Value;
use crate::catalog::information_schema::{
    ColumnInfo, columns_schema, referential_constraints_schema, schemata_schema,
    table_constraints_schema, tables_schema, view_to_schema,
};
use crate::catalog::{Catalog, TableSchema};
use std::sync::Arc;

/// A scan executor that produces rows from `information_schema` views.
///
/// The view name is the part after `information_schema.` (e.g. for
/// `FROM information_schema.tables`, the view name is `tables`).
pub struct InformationSchemaScanExecutor {
    /// Name of the `information_schema` view to scan.
    view: String,
    /// Schema describing the rows that this executor returns.
    schema: TableSchema,
    /// Snapshot of the rows to emit, materialized at construction time.
    /// `Vec<Option<HashMap<String, Value>>>` per [issue #23](https://github.com/neoalienson/Vaultgres/issues/23).
    rows: Vec<Tuple>,
    /// Cursor over `rows`.
    cursor: usize,
}

impl InformationSchemaScanExecutor {
    /// Construct from a view name (the part after `information_schema.`).
    ///
    /// Returns `None` if the name is not a recognized `information_schema` view.
    pub fn try_new(view: &str, catalog: Arc<Catalog>) -> Result<Option<Self>, ExecutorError> {
        let Some(schema) = view_to_schema(view) else {
            return Ok(None);
        };

        let rows = match view {
            "schemata" => schemata_rows(&catalog),
            "tables" => tables_rows(&catalog),
            "columns" => columns_rows(&catalog),
            "table_constraints" => table_constraints_rows(&catalog),
            "referential_constraints" => referential_constraints_rows(&catalog),
            // unreachable: view_to_schema only returns Some for known views
            _ => return Ok(None),
        };

        Ok(Some(Self { view: view.to_string(), schema, rows, cursor: 0 }))
    }

    /// Convenience: full FROM clause (`information_schema.<view>`) -> executor.
    pub fn from_clause(
        from_table_name: &str,
        catalog: Arc<Catalog>,
    ) -> Result<Option<Self>, ExecutorError> {
        let Some(rest) = from_table_name.strip_prefix("information_schema.") else {
            return Ok(None);
        };
        Self::try_new(rest, catalog)
    }

    /// Schema of the rows this executor emits.
    pub fn schema(&self) -> &TableSchema {
        &self.schema
    }

    /// View name this executor scans.
    pub fn view(&self) -> &str {
        &self.view
    }
}

impl Executor for InformationSchemaScanExecutor {
    fn next(&mut self) -> Result<Option<Tuple>, ExecutorError> {
        if self.cursor >= self.rows.len() {
            return Ok(None);
        }
        let row = self.rows[self.cursor].clone();
        self.cursor += 1;
        Ok(Some(row))
    }
}

// ---------------------------------------------------------------------------
// Row builders
// ---------------------------------------------------------------------------

/// Helper: build a tuple with NULL for any columns missing from `cols`.
fn tuple_with(cols: &[(&str, Value)], schema_cols: &[ColumnInfo]) -> Tuple {
    let mut out = Tuple::new();
    for (name, _dtype) in schema_cols {
        let v = cols
            .iter()
            .find(|(n, _)| *n == name.as_str())
            .map(|(_, v)| v.clone())
            .unwrap_or(Value::Null);
        out.insert(name.clone(), v);
    }
    out
}

/// Helper: convert `DataType` to a SQL-standard "data_type" string.
fn data_type_name(dt: &crate::parser::ast::DataType) -> &'static str {
    use crate::parser::ast::DataType as D;
    match dt {
        D::Int => "integer",
        D::Serial => "integer",
        D::Float => "double precision",
        D::Text => "text",
        D::Varchar(_) => "character varying",
        D::Boolean => "boolean",
        D::Date => "date",
        D::Time => "time without time zone",
        D::Timestamp => "timestamp without time zone",
        D::Decimal(_, _) => "numeric",
        D::Bytea => "bytea",
        D::Json => "json",
        D::Jsonb => "jsonb",
        D::Enum(_) => "USER-DEFINED",
        D::Composite(_) => "USER-DEFINED",
        D::Array(inner) => match inner.as_ref() {
            D::Int => "integer[]",
            D::Text => "text[]",
            D::Boolean => "boolean[]",
            D::Float => "double precision[]",
            _ => "ARRAY",
        },
        D::Int4Range => "int4range",
        D::Int8Range => "int8range",
        D::NumRange => "numrange",
        D::DateRange => "daterange",
        D::TsRange => "tsrange",
        D::TsTzRange => "tstzrange",
    }
}

/// ISO/IEC 9075-11 "schemata" — one row per schema in the catalog.
///
/// Vaultgres does not implement multi-schema databases yet, so this
/// always emits a single row for the `public` schema.
fn schemata_rows(catalog: &Catalog) -> Vec<Tuple> {
    // Vaultgres has a single implicit schema today. Even when the catalog
    // has zero user tables, the `public` schema exists by SQL convention.
    let _ = catalog; // currently unused, reserved for future multi-schema support
    vec![tuple_with(
        &[
            ("catalog_name", Value::Text("vaultgres".to_string())),
            ("schema_name", Value::Text("public".to_string())),
            ("schema_owner", Value::Text("postgres".to_string())),
            ("default_character_set_catalog", Value::Null),
            ("default_character_set_schema", Value::Null),
            ("default_character_set_name", Value::Null),
        ],
        &schemata_schema(),
    )]
}

/// ISO/IEC 9075-11 "tables" — one row per base table or view in the schema.
fn tables_rows(catalog: &Catalog) -> Vec<Tuple> {
    let table_names = catalog.list_tables();
    let mut out = Vec::with_capacity(table_names.len());
    for name in &table_names {
        let table_type = if catalog.get_view(name).is_some() { "VIEW" } else { "BASE TABLE" };
        out.push(tuple_with(
            &[
                ("table_catalog", Value::Text("vaultgres".to_string())),
                ("table_schema", Value::Text("public".to_string())),
                ("table_name", Value::Text(name.clone())),
                ("table_type", Value::Text(table_type.to_string())),
            ],
            &tables_schema(),
        ));
    }
    out
}

/// ISO/IEC 9075-11 "columns" — one row per column of every table/view.
fn columns_rows(catalog: &Catalog) -> Vec<Tuple> {
    let table_names = catalog.list_tables();
    let mut out = Vec::new();
    for table_name in &table_names {
        let Some(table) = catalog.get_table(table_name) else {
            continue;
        };
        for (ordinal, col) in table.columns.iter().enumerate() {
            out.push(tuple_with(
                &[
                    ("table_catalog", Value::Text("vaultgres".to_string())),
                    ("table_schema", Value::Text("public".to_string())),
                    ("table_name", Value::Text(table.name.clone())),
                    ("column_name", Value::Text(col.name.clone())),
                    ("ordinal_position", Value::Int((ordinal + 1) as i64)),
                    ("column_default", Value::Null), // todo: emit Expr::to_sql() when added
                    (
                        "is_nullable",
                        Value::Text(if col.is_not_null { "NO" } else { "YES" }.to_string()),
                    ),
                    ("data_type", Value::Text(data_type_name(&col.data_type).to_string())),
                    ("character_maximum_length", Value::Null),
                    ("character_octet_length", Value::Null),
                    ("numeric_precision", Value::Null),
                    ("numeric_scale", Value::Null),
                    ("datetime_precision", Value::Null),
                    ("character_set_catalog", Value::Null),
                    ("character_set_schema", Value::Null),
                    ("character_set_name", Value::Null),
                    ("collation_catalog", Value::Null),
                    ("collation_schema", Value::Null),
                    ("collation_name", Value::Null),
                    ("domain_catalog", Value::Null),
                    ("domain_schema", Value::Null),
                    ("domain_name", Value::Null),
                    ("udt_catalog", Value::Null),
                    ("udt_schema", Value::Null),
                    ("udt_name", Value::Null),
                ],
                &columns_schema(),
            ));
        }
    }
    out
}

/// ISO/IEC 9075-11 "table_constraints" — one row per table-level constraint
/// (PRIMARY KEY, UNIQUE, CHECK, FOREIGN KEY).
fn table_constraints_rows(catalog: &Catalog) -> Vec<Tuple> {
    let mut out = Vec::new();
    let mut cc_seq: i64 = 0; // constraint_catalog local sequence (per-session)
    for table_name in catalog.list_tables() {
        let Some(table) = catalog.get_table(&table_name) else {
            continue;
        };
        if let Some(pk_cols) = &table.primary_key {
            cc_seq += 1;
            out.push(make_constraint_row(
                &table.name,
                format!("{}_pkey", table.name),
                "PRIMARY KEY",
                Some(pk_cols.join(", ")),
            ));
            // record sequence position so later constraint numbers stay monotonic
            let _ = cc_seq;
        }
        for (i, uk) in table.unique_constraints.iter().enumerate() {
            out.push(make_constraint_row(
                &table.name,
                format!("{}_{}_key", table.name, uk.name.as_deref().unwrap_or("unique")),
                "UNIQUE",
                Some(uk.columns.join(", ")),
            ));
            let _ = i;
        }
        for (i, _ck) in table.check_constraints.iter().enumerate() {
            out.push(make_constraint_row(
                &table.name,
                format!("{}_check{}", table.name, i + 1),
                "CHECK",
                None, // todo: emit check_expr to_sql() when supported
            ));
        }
        for (i, fk) in table.foreign_keys.iter().enumerate() {
            out.push(make_constraint_row(
                &table.name,
                format!("fk_{}_{}", table.name, i + 1),
                "FOREIGN KEY",
                Some(fk.columns.join(", ")),
            ));
        }
    }
    out
}

fn make_constraint_row(
    table_name: &str,
    constraint_name: String,
    constraint_type: &str,
    column_list: Option<String>,
) -> Tuple {
    tuple_with(
        &[
            ("constraint_catalog", Value::Text("vaultgres".to_string())),
            ("constraint_schema", Value::Text("public".to_string())),
            ("constraint_name", Value::Text(constraint_name)),
            ("table_catalog", Value::Text("vaultgres".to_string())),
            ("table_schema", Value::Text("public".to_string())),
            ("table_name", Value::Text(table_name.to_string())),
            ("constraint_type", Value::Text(constraint_type.to_string())),
            ("is_deferrable", Value::Text("NO".to_string())),
            ("initially_deferred", Value::Text("NO".to_string())),
            ("enforced", Value::Text("YES".to_string())),
            ("column_list", column_list.map(Value::Text).unwrap_or(Value::Null)),
        ],
        &table_constraints_schema(),
    )
}

/// ISO/IEC 9075-11 "referential_constraints" — one row per FK relationship.
fn referential_constraints_rows(catalog: &Catalog) -> Vec<Tuple> {
    let mut out = Vec::new();
    for table_name in catalog.list_tables() {
        let Some(table) = catalog.get_table(&table_name) else {
            continue;
        };
        for (i, fk) in table.foreign_keys.iter().enumerate() {
            // Vaultgres does not store an explicit constraint name on FKs;
            // synthesize a deterministic one so information_schema is well-formed.
            let constraint_name = format!("fk_{}_{}", table.name, i + 1);
            let unique_constraint_name = format!("{}_pkey", fk.ref_table);
            out.push(tuple_with(
                &[
                    ("constraint_catalog", Value::Text("vaultgres".to_string())),
                    ("constraint_schema", Value::Text("public".to_string())),
                    ("constraint_name", Value::Text(constraint_name)),
                    ("unique_constraint_catalog", Value::Text("vaultgres".to_string())),
                    ("unique_constraint_schema", Value::Text("public".to_string())),
                    ("unique_constraint_name", Value::Text(unique_constraint_name)),
                    ("match_option", Value::Text("NONE".to_string())),
                    ("update_rule", Value::Text(format!("{:?}", fk.on_update).to_uppercase())),
                    ("delete_rule", Value::Text(format!("{:?}", fk.on_delete).to_uppercase())),
                ],
                &referential_constraints_schema(),
            ));
        }
    }
    out
}

fn match_on_action(action: crate::parser::ast::ForeignKeyAction) -> String {
    use crate::parser::ast::ForeignKeyAction as A;
    match action {
        A::Cascade => "CASCADE".to_string(),
        A::SetNull => "SET NULL".to_string(),
        A::Restrict => "RESTRICT".to_string(),
    }
}

/// `InformationSchemaScanExecutor` schema helpers re-exported through
/// the executor module so tests below don't have to import them
/// separately.
#[allow(dead_code)]
fn _schema_helpers_for_tests() {
    let _ = schemata_schema();
    let _ = columns_schema();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::parser::ast::{ColumnDef, DataType};

    fn fresh_catalog() -> Arc<Catalog> {
        Arc::new(Catalog::new())
    }

    #[test]
    fn unknown_view_returns_none() {
        let cat = fresh_catalog();
        let exec =
            InformationSchemaScanExecutor::from_clause("information_schema.bogus", cat).unwrap();
        assert!(exec.is_none());
    }

    #[test]
    fn schemata_has_public_schema() {
        let cat = fresh_catalog();
        let mut exec =
            InformationSchemaScanExecutor::from_clause("information_schema.schemata", cat)
                .unwrap()
                .expect("schemata should be recognized");
        let row = exec.next().unwrap().expect("one row");
        assert_eq!(row.get("schema_name"), Some(&Value::Text("public".to_string())));
        assert!(exec.next().unwrap().is_none());
    }

    #[test]
    fn tables_empty_catalog_returns_no_rows() {
        let cat = fresh_catalog();
        let mut exec = InformationSchemaScanExecutor::from_clause("information_schema.tables", cat)
            .unwrap()
            .expect("tables should be recognized");
        assert!(exec.next().unwrap().is_none());
    }

    #[test]
    fn tables_lists_user_tables() {
        let cat = fresh_catalog();
        cat.create_table(
            "widgets".to_string(),
            vec![ColumnDef {
                name: "id".to_string(),
                data_type: DataType::Int,
                is_primary_key: true,
                is_unique: false,
                is_auto_increment: false,
                is_not_null: true,
                default_value: None,
                foreign_key: None,
            }],
        )
        .unwrap();
        let mut exec = InformationSchemaScanExecutor::from_clause("information_schema.tables", cat)
            .unwrap()
            .unwrap();
        let row = exec.next().unwrap().expect("one row");
        assert_eq!(row.get("table_name"), Some(&Value::Text("widgets".to_string())));
        assert_eq!(row.get("table_type"), Some(&Value::Text("BASE TABLE".to_string())));
        assert!(exec.next().unwrap().is_none());
    }

    #[test]
    fn columns_lists_columns_of_each_table() {
        let cat = fresh_catalog();
        cat.create_table(
            "products".to_string(),
            vec![
                ColumnDef {
                    name: "id".to_string(),
                    data_type: DataType::Int,
                    is_primary_key: true,
                    is_unique: false,
                    is_auto_increment: false,
                    is_not_null: true,
                    default_value: None,
                    foreign_key: None,
                },
                ColumnDef {
                    name: "name".to_string(),
                    data_type: DataType::Text,
                    is_primary_key: false,
                    is_unique: false,
                    is_auto_increment: false,
                    is_not_null: false,
                    default_value: None,
                    foreign_key: None,
                },
            ],
        )
        .unwrap();
        let mut exec =
            InformationSchemaScanExecutor::from_clause("information_schema.columns", cat)
                .unwrap()
                .unwrap();
        let r1 = exec.next().unwrap().expect("row 1");
        assert_eq!(r1.get("table_name"), Some(&Value::Text("products".to_string())));
        assert_eq!(r1.get("column_name"), Some(&Value::Text("id".to_string())));
        assert_eq!(r1.get("ordinal_position"), Some(&Value::Int(1)));
        assert_eq!(r1.get("is_nullable"), Some(&Value::Text("NO".to_string())));
        assert_eq!(r1.get("data_type"), Some(&Value::Text("integer".to_string())));
        let r2 = exec.next().unwrap().expect("row 2");
        assert_eq!(r2.get("column_name"), Some(&Value::Text("name".to_string())));
        assert_eq!(r2.get("is_nullable"), Some(&Value::Text("YES".to_string())));
        assert_eq!(r2.get("data_type"), Some(&Value::Text("text".to_string())));
        assert!(exec.next().unwrap().is_none());
    }

    #[test]
    fn unknown_prefix_returns_none() {
        let cat = fresh_catalog();
        let exec = InformationSchemaScanExecutor::from_clause("public.widgets", cat).unwrap();
        assert!(exec.is_none());
    }
}
