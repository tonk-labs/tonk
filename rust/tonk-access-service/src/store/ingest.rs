//! Ingest storage: one recorded row per invocation.
//!
//! The ordered SQL files under `migrations-ingest/` are the schema's
//! source of truth, applied by Wrangler to the ingest D1 database and by
//! [`SqliteIngest`] natively. Deliberately a separate database from
//! control: bulky, write-heavy, disposable once charged and archived,
//! with its own 10 GB ceiling.

use async_trait::async_trait;

use super::StoreError;

/// One invocation, as recorded on the hot path.
#[derive(Debug, Clone)]
pub struct InvocationRecord {
    /// When it was authorized, as a unix timestamp in seconds.
    pub ts: u64,
    /// The invocation's CID, the evidence key.
    pub cid: String,
    /// The consumer space the invocation addressed.
    pub consumer: String,
    /// Who signed the invocation.
    pub issuer: String,
    /// The command path, `/`-joined.
    pub cmd: String,
    /// `ok` or `denied`.
    pub outcome: &'static str,
    /// Why a denied invocation was denied.
    pub reason: Option<String>,
    /// Declared write bytes, zero elsewhere.
    pub bytes: u64,
    /// Identifier of the flattened proof set.
    pub chain: String,
    /// The invocation's exact bytes.
    pub body: Vec<u8>,
    /// The proof set: each delegation's CID and exact bytes. Written
    /// with `INSERT OR IGNORE`, so a chain already seen costs writes
    /// only the first time.
    pub proofs: Vec<(String, Vec<u8>)>,
}

/// Where invocation records land. Declared through the dual
/// `async_trait` forms, like [`Store`](super::Store).
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
pub trait IngestStore {
    /// Durably record one invocation with its chain and blocks, in one
    /// batch.
    async fn record(&self, record: &InvocationRecord) -> Result<(), StoreError>;
}

// Shared query text, as in the control store.

pub const INSERT_INVOCATION: &str = r#"
INSERT INTO invocation (ts, cid, consumer, issuer, cmd, outcome, reason,
                        bytes, compute, chain, body)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10)
"#;

pub const INSERT_CHAIN: &str = r#"
INSERT OR IGNORE INTO chain (chain, proof) VALUES (?1, ?2)
"#;

pub const INSERT_BLOCK: &str = r#"
INSERT OR IGNORE INTO block (cid, body) VALUES (?1, ?2)
"#;

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
mod sqlite {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use rusqlite::{Connection, params};

    use super::super::StoreError;
    use super::{INSERT_BLOCK, INSERT_CHAIN, INSERT_INVOCATION, IngestStore, InvocationRecord};

    /// Native `rusqlite`-backed [`IngestStore`], for tests and local
    /// development.
    pub struct SqliteIngest(Mutex<Connection>);

    impl SqliteIngest {
        /// Open a fresh in-memory database and apply every migration
        /// under `migrations-ingest/`.
        pub fn in_memory() -> Result<Self, StoreError> {
            let conn = Connection::open_in_memory()
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            Self::prepare(conn)
        }

        /// Open (or create) a file-backed database, applying the
        /// migrations only on first open.
        pub fn open(path: &std::path::Path) -> Result<Self, StoreError> {
            let conn =
                Connection::open(path).map_err(|err| StoreError::Internal(err.to_string()))?;
            Self::prepare(conn)
        }

        fn prepare(conn: Connection) -> Result<Self, StoreError> {
            let version: i64 = conn
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            if version == 0 {
                conn.execute_batch(include_str!("../../migrations-ingest/0001_ingest.sql"))
                    .map_err(|err| StoreError::Internal(err.to_string()))?;
                conn.pragma_update(None, "user_version", 1)
                    .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            Ok(Self(Mutex::new(conn)))
        }

        /// How many invocations are recorded, for test inspection.
        pub fn invocations(&self) -> Result<u64, StoreError> {
            let conn = self.0.lock().expect("ingest mutex poisoned");
            conn.query_row("SELECT COUNT(*) FROM invocation", [], |row| row.get(0))
                .map_err(|err| StoreError::Internal(err.to_string()))
        }
    }

    #[async_trait]
    impl IngestStore for SqliteIngest {
        async fn record(&self, record: &InvocationRecord) -> Result<(), StoreError> {
            let mut conn = self.0.lock().expect("ingest mutex poisoned");
            let tx = conn
                .transaction()
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            tx.execute(
                INSERT_INVOCATION,
                params![
                    record.ts as i64,
                    record.cid,
                    record.consumer,
                    record.issuer,
                    record.cmd,
                    record.outcome,
                    record.reason,
                    record.bytes as i64,
                    record.chain,
                    record.body,
                ],
            )
            .map_err(|err| StoreError::Internal(err.to_string()))?;
            for (cid, body) in &record.proofs {
                tx.execute(INSERT_CHAIN, params![record.chain, cid])
                    .map_err(|err| StoreError::Internal(err.to_string()))?;
                tx.execute(INSERT_BLOCK, params![cid, body])
                    .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            tx.commit()
                .map_err(|err| StoreError::Internal(err.to_string()))
        }
    }
}

#[cfg(all(feature = "helpers", not(target_arch = "wasm32")))]
pub use sqlite::SqliteIngest;

#[cfg(target_arch = "wasm32")]
mod d1 {
    use async_trait::async_trait;
    use worker::d1::D1Database;
    use worker::wasm_bindgen::JsValue;

    use super::super::StoreError;
    use super::{INSERT_BLOCK, INSERT_CHAIN, INSERT_INVOCATION, IngestStore, InvocationRecord};

    /// Cloudflare D1-backed [`IngestStore`], for production use.
    pub struct D1Ingest(D1Database);

    impl D1Ingest {
        /// Wrap a D1 database binding.
        pub fn new(db: D1Database) -> Self {
            Self(db)
        }
    }

    fn map_err(err: worker::Error) -> StoreError {
        StoreError::Internal(err.to_string())
    }

    #[async_trait(?Send)]
    impl IngestStore for D1Ingest {
        async fn record(&self, record: &InvocationRecord) -> Result<(), StoreError> {
            let mut statements = Vec::with_capacity(1 + record.proofs.len() * 2);
            statements.push(
                self.0
                    .prepare(INSERT_INVOCATION)
                    .bind(&[
                        JsValue::from_f64(record.ts as f64),
                        JsValue::from(record.cid.as_str()),
                        JsValue::from(record.consumer.as_str()),
                        JsValue::from(record.issuer.as_str()),
                        JsValue::from(record.cmd.as_str()),
                        JsValue::from(record.outcome),
                        record
                            .reason
                            .as_deref()
                            .map(JsValue::from)
                            .unwrap_or(JsValue::NULL),
                        JsValue::from_f64(record.bytes as f64),
                        JsValue::from(record.chain.as_str()),
                        js_sys::Uint8Array::from(record.body.as_slice()).into(),
                    ])
                    .map_err(map_err)?,
            );
            for (cid, body) in &record.proofs {
                statements.push(
                    self.0
                        .prepare(INSERT_CHAIN)
                        .bind(&[
                            JsValue::from(record.chain.as_str()),
                            JsValue::from(cid.as_str()),
                        ])
                        .map_err(map_err)?,
                );
                statements.push(
                    self.0
                        .prepare(INSERT_BLOCK)
                        .bind(&[
                            JsValue::from(cid.as_str()),
                            js_sys::Uint8Array::from(body.as_slice()).into(),
                        ])
                        .map_err(map_err)?,
                );
            }
            self.0.batch(statements).await.map_err(map_err)?;
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use d1::D1Ingest;
