//! End-to-end integration tests for `information_schema` views.
//!
//! These exercise the full parse -> plan -> execute path, proving that a
//! real SQL query like `SELECT * FROM information_schema.tables` resolves
//! to the dedicated scan executor and returns catalog metadata as rows.
//!
//! Tracked by https://github.com/neoalienson/Vaultgres/issues/23.

use std::sync::Arc;
use vaultgres::catalog::Catalog;
use vaultgres::parser::ast::{ColumnDef, DataType};
use vaultgres::parser::parser::Parser;
use vaultgres::planner::planner::Planner;

fn catalog_with_two_tables() -> Arc<Catalog> {
    let catalog = Arc::new(Catalog::new());
    catalog
        .create_table(
            "users".to_string(),
            vec![
                ColumnDef::new("id".to_string(), DataType::Int),
                ColumnDef::new("name".to_string(), DataType::Text),
            ],
        )
        .unwrap();
    catalog
        .create_table(
            "orders".to_string(),
            vec![
                ColumnDef::new("id".to_string(), DataType::Int),
                ColumnDef::new("user_id".to_string(), DataType::Int),
            ],
        )
        .unwrap();
    catalog
}

/// Parse + plan + execute a single SELECT statement against `catalog`,
/// returning all rows it produces.
fn run_select(
    catalog: Arc<Catalog>,
    sql: &str,
) -> Vec<std::collections::HashMap<String, vaultgres::catalog::Value>> {
    let mut parser = Parser::new(sql).unwrap();
    let stmt = parser.parse().unwrap();
    let stmt = match stmt {
        vaultgres::parser::ast::Statement::Select(s) => s,
        other => panic!("expected SELECT, got {:?}", other),
    };
    let planner = Planner::new_with_catalog(catalog);
    let mut plan = planner.plan(&stmt).expect("plan should succeed");
    let mut out = Vec::new();
    while let Some(row) = plan.next().expect("executor should not error") {
        out.push(row);
    }
    out
}

#[test]
fn select_from_information_schema_tables_lists_user_tables() {
    let catalog = catalog_with_two_tables();
    let rows = run_select(catalog, "SELECT table_name, table_type FROM information_schema.tables");
    assert_eq!(rows.len(), 2, "expected 2 rows (users, orders)");
    let names: Vec<String> = rows
        .iter()
        .map(|r| match r.get("table_name").unwrap() {
            vaultgres::catalog::Value::Text(s) => s.clone(),
            other => panic!("expected text, got {:?}", other),
        })
        .collect();
    assert!(names.contains(&"users".to_string()));
    assert!(names.contains(&"orders".to_string()));
    for row in &rows {
        assert_eq!(
            row.get("table_type"),
            Some(&vaultgres::catalog::Value::Text("BASE TABLE".to_string()))
        );
    }
}

#[test]
fn select_from_information_schema_columns_lists_columns() {
    let catalog = catalog_with_two_tables();
    let rows = run_select(
        catalog,
        "SELECT table_name, column_name, ordinal_position, is_nullable \
         FROM information_schema.columns WHERE table_name = 'users'",
    );
    assert_eq!(rows.len(), 2, "users has 2 columns: id, name");
    let r_id = rows
        .iter()
        .find(|r| {
            matches!(
                r.get("column_name"),
                Some(vaultgres::catalog::Value::Text(s)) if s == "id"
            )
        })
        .expect("id column row");
    assert_eq!(r_id.get("ordinal_position"), Some(&vaultgres::catalog::Value::Int(1)));
    assert_eq!(r_id.get("is_nullable"), Some(&vaultgres::catalog::Value::Text("YES".to_string())));
    let r_name = rows
        .iter()
        .find(|r| {
            matches!(
                r.get("column_name"),
                Some(vaultgres::catalog::Value::Text(s)) if s == "name"
            )
        })
        .expect("name column row");
    // The test SELECT above does not include data_type in its projection,
    // so it's not in the result row. Verify the projection at least
    // preserved the requested column_name for the second column.
    assert_eq!(
        r_name.get("column_name"),
        Some(&vaultgres::catalog::Value::Text("name".to_string()))
    );
}

#[test]
fn select_from_information_schema_schemata_emits_public_schema() {
    let catalog = catalog_with_two_tables();
    let rows = run_select(catalog, "SELECT schema_name FROM information_schema.schemata");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("schema_name"),
        Some(&vaultgres::catalog::Value::Text("public".to_string()))
    );
}

#[test]
fn select_from_information_schema_tables_with_empty_catalog() {
    let catalog = Arc::new(Catalog::new());
    let rows = run_select(catalog, "SELECT table_name FROM information_schema.tables");
    assert!(rows.is_empty(), "empty catalog -> empty result set");
}

#[test]
fn unknown_information_schema_view_returns_plan_error() {
    let catalog = Arc::new(Catalog::new());
    // `information_schema.bogus` is not a known view. The planner falls
    // through to the table-resolution path which raises "not found".
    let mut parser = Parser::new("SELECT * FROM information_schema.bogus").unwrap();
    let stmt = parser.parse().unwrap();
    let stmt = match stmt {
        vaultgres::parser::ast::Statement::Select(s) => s,
        other => panic!("expected SELECT, got {:?}", other),
    };
    let planner = Planner::new_with_catalog(catalog);
    let plan_result = planner.plan(&stmt);
    assert!(plan_result.is_err(), "unknown information_schema view should produce a plan error");
}

#[test]
fn information_schema_with_projection_and_filter() {
    // Verify the executor composes correctly with Filter + Project operators
    // downstream of the scan (proves planner wires `is_exec -> build_plan_from_scan`).
    let catalog = catalog_with_two_tables();
    let rows = run_select(
        catalog,
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' ORDER BY table_name",
    );
    assert_eq!(rows.len(), 2);
    let names: Vec<String> = rows
        .iter()
        .map(|r| match r.get("table_name").unwrap() {
            vaultgres::catalog::Value::Text(s) => s.clone(),
            _ => panic!("expected text"),
        })
        .collect();
    assert_eq!(names, vec!["orders".to_string(), "users".to_string()]);
}
