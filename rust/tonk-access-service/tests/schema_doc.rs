//! `docs/access-control-schema.md` draws the control database, and
//! nothing generates it. A diagram that has drifted is worse than none:
//! the next reader trusts it and builds on a column that is not there.
//!
//! This applies every migration to an in-memory database and compares
//! the result against the ER diagram, table by table and column by
//! column. Prose in a skill was not enough — the diagram claimed a
//! `ledger` column that never existed and omitted the `access` one that
//! does, and stayed that way until someone read it closely.
#![cfg(all(feature = "helpers", not(target_arch = "wasm32")))]

use std::collections::BTreeMap;
use std::path::Path;

/// The schema every migration adds up to, as `table -> columns`.
fn schema() -> BTreeMap<String, Vec<String>> {
    let connection = rusqlite::Connection::open_in_memory().expect("in-memory database");
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut migrations: Vec<_> = std::fs::read_dir(&directory)
        .expect("migrations directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "sql"))
        .collect();
    // Numeric prefixes, so lexical order is application order.
    migrations.sort();
    for migration in &migrations {
        let sql = std::fs::read_to_string(migration).expect("migration is readable");
        connection
            .execute_batch(&sql)
            .unwrap_or_else(|error| panic!("{} failed: {error}", migration.display()));
    }

    let mut tables = BTreeMap::new();
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .expect("table listing");
    let names: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("table names")
        .collect::<Result<_, _>>()
        .expect("table names");
    for name in names {
        let mut columns = connection
            .prepare(&format!("PRAGMA table_info({name})"))
            .expect("column listing");
        let listed: Vec<String> = columns
            .query_map([], |row| row.get::<_, String>(1))
            .expect("column names")
            .collect::<Result<_, _>>()
            .expect("column names");
        tables.insert(name, listed);
    }
    tables
}

/// The same shape, read out of the document's `erDiagram` block.
///
/// Entities the diagram draws without a table — `account` is a
/// relationship the schema does not store — carry no column block, so
/// they simply do not appear here.
fn diagrammed() -> BTreeMap<String, Vec<String>> {
    let document = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/access-control-schema.md"),
    )
    .expect("the schema document is readable");
    let block = document
        .split("```mermaid\nerDiagram\n")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("the document carries an erDiagram block");

    let mut tables = BTreeMap::new();
    let mut current: Option<(String, Vec<String>)> = None;
    for line in block.lines() {
        let trimmed = line.trim();
        if let Some(name) = trimmed.strip_suffix('{') {
            current = Some((name.trim().to_owned(), Vec::new()));
        } else if trimmed == "}" {
            if let Some((name, columns)) = current.take() {
                tables.insert(name, columns);
            }
        } else if let Some((_, columns)) = current.as_mut() {
            // `TYPE name PK "comment"` — the column is the second word.
            if let Some(column) = trimmed.split_whitespace().nth(1) {
                columns.push(column.to_owned());
            }
        }
    }
    tables
}

#[test]
fn it_draws_every_table_the_migrations_create() {
    let schema = schema();
    let drawn = diagrammed();

    let missing: Vec<_> = schema.keys().filter(|t| !drawn.contains_key(*t)).collect();
    assert!(
        missing.is_empty(),
        "the schema document does not draw {missing:?} — see .claude/skills/control-schema/"
    );

    let invented: Vec<_> = drawn.keys().filter(|t| !schema.contains_key(*t)).collect();
    assert!(
        invented.is_empty(),
        "the schema document draws {invented:?}, which no migration creates"
    );
}

#[test]
fn it_draws_every_column_and_invents_none() {
    let schema = schema();
    let drawn = diagrammed();

    for (table, columns) in &schema {
        let Some(shown) = drawn.get(table) else {
            continue; // The table test above reports this.
        };
        let missing: Vec<_> = columns.iter().filter(|c| !shown.contains(c)).collect();
        assert!(
            missing.is_empty(),
            "`{table}` has {missing:?} in the schema but not in the diagram — \
             see .claude/skills/control-schema/"
        );
        let invented: Vec<_> = shown.iter().filter(|c| !columns.contains(c)).collect();
        assert!(
            invented.is_empty(),
            "the diagram gives `{table}` {invented:?}, which no migration creates"
        );
    }
}
