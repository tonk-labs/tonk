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

/// Every migration, in application order, embedded at compile time.
///
/// `include_str!` rather than reading the directory: CI runs this suite from
/// a `cargo nextest archive`, which bundles the compiled test binaries but
/// not arbitrary data files, so a runtime `read_dir` finds nothing and the
/// test fails there while passing locally. Embedding makes each migration a
/// build input that travels inside the archive.
///
/// Listed by hand for the same reason — the archive has no directory to
/// enumerate. A new migration must be added here, and `it_applies_every_
/// migration_on_disk` fails until it is.
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_control.sql",
        include_str!("../migrations/0001_control.sql"),
    ),
    (
        "0002_deletion.sql",
        include_str!("../migrations/0002_deletion.sql"),
    ),
    (
        "0003_deprovision.sql",
        include_str!("../migrations/0003_deprovision.sql"),
    ),
    (
        "0004_consumer_kind.sql",
        include_str!("../migrations/0004_consumer_kind.sql"),
    ),
    (
        "0005_customer_email.sql",
        include_str!("../migrations/0005_customer_email.sql"),
    ),
    (
        "0006_account_schema.sql",
        include_str!("../migrations/0006_account_schema.sql"),
    ),
];

/// The schema every migration adds up to, as `table -> columns`.
fn schema() -> BTreeMap<String, Vec<String>> {
    let connection = rusqlite::Connection::open_in_memory().expect("in-memory database");
    for (name, sql) in MIGRATIONS {
        connection
            .execute_batch(sql)
            .unwrap_or_else(|error| panic!("{name} failed: {error}"));
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
fn diagrammed() -> Option<BTreeMap<String, Vec<String>>> {
    // Read at runtime, unlike the migrations above: `docs/` is outside the
    // nix source filter (`nix/rust.nix` includes only `rust/`), so
    // `include_str!` cannot reach it from a sandboxed build. The archive CI
    // runs from carries no docs either, so this test simply does not run
    // there — which is why the migrations, whose absence made the test panic
    // rather than skip, are the half that had to be embedded.
    let document = match std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/access-control-schema.md"),
    ) {
        Ok(document) => document,
        // No document to compare against, so there is nothing to check. An
        // empty diagram would fail as "draws no tables", which reads as
        // drift rather than as the absent source tree it is.
        Err(_) => return None,
    };
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
    Some(tables)
}

#[test]
fn it_draws_every_table_the_migrations_create() {
    let schema = schema();
    let Some(drawn) = diagrammed() else {
        return; // No source tree: see `diagrammed`.
    };

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
    let Some(drawn) = diagrammed() else {
        return; // No source tree: see `diagrammed`.
    };

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

/// The embedded list covers every migration on disk.
///
/// `MIGRATIONS` is hand-written because the archive has no directory to
/// enumerate, so nothing but this test notices when a new migration is added
/// and not listed — the schema would then be checked against a document
/// describing a database the service does not have.
///
/// Reads the directory deliberately: this assertion is only meaningful where
/// the source tree exists, and it is skipped rather than failed where it does
/// not, which is exactly the archive case the embedding exists for.
#[test]
fn it_applies_every_migration_on_disk() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let Ok(entries) = std::fs::read_dir(&directory) else {
        // Running from an archive: no source tree, nothing to compare.
        return;
    };
    let mut on_disk: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    on_disk.sort();

    let listed: Vec<String> = MIGRATIONS
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    assert_eq!(
        listed, on_disk,
        "MIGRATIONS is out of step with rust/tonk-access-service/migrations; \
         add the new file to the list in this test"
    );
}
