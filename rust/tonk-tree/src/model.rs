//! Tree data types and the worker-backed loader.
//!
//! Nodes and entries mirror the `tree/*` formula conclusions the worker
//! returns. The loader POSTs a formula query (string predicate + terms)
//! to the branch's `/query` endpoint and maps the rows.

use serde::Deserialize;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Headers, RequestInit, Response, window};

/// A node hash, the `#<base58>` string the worker emits.
pub type NodeHash = String;

/// Whether a node holds child links (index) or entries (segment).
#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Index,
    Segment,
}

/// One decoded, self-describing component of a key, as the worker's
/// `key_parts` emits it. `kind` selects the UI color/glyph, `text` is the
/// human rendering (a `did:key:…` entity, a `db.meta/name` attribute, a typed
/// value, a decimal edition), and `hex` is the raw bytes for the tooltip.
#[derive(Clone, Deserialize)]
pub struct KeyPart {
    pub kind: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub hex: String,
}

/// One node's fields — the `tree/node` / `tree/child` shape.
#[derive(Clone)]
pub struct TreeNode {
    pub hash: NodeHash,
    pub kind: Kind,
    pub size: u64,
    pub count: u64,
    /// The bound key's raw hex — kept for the front-coding pivot, which
    /// compares raw bytes across siblings.
    pub bound: Option<NodeHash>,
    /// The bound key's decoded components (the worker's `bound-parts`).
    pub bound_parts: Vec<KeyPart>,
    pub rank: Option<u64>,
    pub cached: bool,
    /// Ops buffered against this node's subtree, pending a flush. On a CHILD
    /// row it is the count for that child's link; on an index NODE it is the
    /// total across the node's links (the worker's `novelty`). Zero ⇒ flushed.
    pub novelty: u64,
    /// How many of an index node's links carry a novelty buffer (the worker's
    /// `buffered-links`). `None` for a segment (no links).
    pub buffered_links: Option<u64>,
}

/// One entry in a segment (the `tree/entry` shape) or one buffered op on an
/// index node (the `tree/novelty` shape) — both reconstruct a fact from a key.
#[derive(Clone)]
pub struct TreeEntry {
    /// The entry key's decoded components (the worker's `key-parts`).
    pub key_parts: Vec<KeyPart>,
    pub retracted: bool,
    pub entity: Option<String>,
    pub attribute: Option<String>,
    pub type_name: Option<String>,
    pub value: Option<serde_json::Value>,
}

/// Raw Conclusion row off the wire.
#[derive(Deserialize)]
struct Row {
    this: String,
    #[serde(default)]
    fields: serde_json::Map<String, serde_json::Value>,
}

fn s(map: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<String> {
    map.get(k).and_then(|v| v.as_str()).map(str::to_owned)
}
fn u(map: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<u64> {
    map.get(k).and_then(|v| v.as_u64())
}
/// Parse a `[{kind, text, hex}, …]` parts array off a fields map. Absent or
/// malformed → an empty list (the caller falls back to the raw hex).
fn parts(map: &serde_json::Map<String, serde_json::Value>, k: &str) -> Vec<KeyPart> {
    map.get(k)
        .and_then(|v| serde_json::from_value::<Vec<KeyPart>>(v.clone()).ok())
        .unwrap_or_default()
}

/// The worker-backed tree loader for a `{repo, branch}`. Cheap to clone
/// (just a URL), so callers can clone it out of shared state before an
/// await rather than holding a borrow across it.
#[derive(Clone)]
pub struct Loader {
    url: String,
}

impl Loader {
    /// Build a loader for a routing [`Location`] — a named space
    /// (`/api/repository/{repo}/branch/{branch}/query`) or the profile
    /// endpoint (`/api/profile/branch/{branch}/query`). The profile has
    /// no repository segment, so a `main@profile:tonk` context targets
    /// the parallel profile surface rather than a named repo.
    ///
    /// A HOST-RELATIVE URL (no origin prefix) so the request routes correctly
    /// everywhere: on a normal page it resolves same-origin; inside a sealed
    /// (opaque-origin) guest `window.location.origin` is the string "null", so
    /// an absolute URL would be `null/api/…` and fail — but the guest's
    /// `window.fetch` proxy reroutes host-relative `/api/…` paths over the
    /// bridge to the host's real origin (where the SW serves them).
    pub fn new(location: &tonk_host::location::Location) -> Self {
        let branch = location.effective_branch();
        let url = match location.space() {
            Some(repo) => format!("/api/repository/{repo}/branch/{branch}/query"),
            None => format!("/api/profile/branch/{branch}/query"),
        };
        Self { url }
    }

    /// POST a formula query and return the decoded rows.
    async fn query(&self, formula: &str, terms: serde_json::Value) -> Result<Vec<Row>, String> {
        let body = json!({ "predicate": formula, "terms": terms }).to_string();

        let headers = Headers::new().map_err(|e| format!("headers: {e:?}"))?;
        headers
            .set("content-type", "application/json")
            .map_err(|e| format!("header: {e:?}"))?;

        let opts = RequestInit::new();
        opts.set_method("POST");
        opts.set_body(&body.into());
        opts.set_headers(&headers);

        // Fetch the STRING path — never a `Request`, which resolves the
        // relative URL against `baseURI` at construction. In a sealed
        // (opaque-origin) guest that baseURI is `null`, so a `Request`
        // would carry an absolute `null/api/…` URL the portal's
        // `window.fetch` override cannot recognize as host-relative and
        // relay. The string path stays `/api/…` until the override sees it.
        let win = window().ok_or("no window")?;
        let resp_val = JsFuture::from(win.fetch_with_str_and_init(&self.url, &opts))
            .await
            .map_err(|e| format!("fetch: {e:?}"))?;
        let resp: Response = resp_val.dyn_into().map_err(|_| "not a Response")?;
        let text = JsFuture::from(resp.text().map_err(|e| format!("text: {e:?}"))?)
            .await
            .map_err(|e| format!("text body: {e:?}"))?
            .as_string()
            .unwrap_or_default();

        if !resp.ok() {
            return Err(format!("{formula} → {}: {}", resp.status(), text));
        }
        serde_json::from_str(&text).map_err(|e| format!("decode: {e}"))
    }

    fn to_node(row: &Row) -> TreeNode {
        let f = &row.fields;
        let kind = if s(f, "kind").as_deref() == Some("segment") {
            Kind::Segment
        } else {
            Kind::Index
        };
        TreeNode {
            // A child row carries its own hash in `child`; a node row in `this`.
            hash: s(f, "child").unwrap_or_else(|| row.this.clone()),
            kind,
            size: u(f, "size").unwrap_or(0),
            count: u(f, "count").unwrap_or(0),
            bound: s(f, "bound"),
            bound_parts: parts(f, "bound-parts"),
            rank: u(f, "rank"),
            cached: f.get("cached").and_then(|v| v.as_bool()) != Some(false),
            novelty: u(f, "novelty").unwrap_or(0),
            buffered_links: u(f, "buffered-links"),
        }
    }

    pub async fn root(&self) -> Result<Option<TreeNode>, String> {
        let rows = self.query("tree/node", json!({})).await?;
        Ok(rows.first().map(Self::to_node))
    }

    /// Re-read one node by hash. Used after a node is fetched (on expand) to
    /// pick up its now-cached fields (kind/size/count and cached: true).
    pub async fn node(&self, hash: &str) -> Result<Option<TreeNode>, String> {
        let rows = self.query("tree/node", json!({ "hash": hash })).await?;
        Ok(rows.first().map(Self::to_node))
    }

    pub async fn children(&self, hash: &str) -> Result<Vec<TreeNode>, String> {
        let rows = self.query("tree/child", json!({ "hash": hash })).await?;
        Ok(rows.iter().map(Self::to_node).collect())
    }

    pub async fn entries(&self, hash: &str) -> Result<Vec<TreeEntry>, String> {
        let rows = self.query("tree/entry", json!({ "hash": hash })).await?;
        Ok(rows.iter().map(row_to_entry).collect())
    }

    /// The ops buffered against an index node's subtrees (the `tree/novelty`
    /// shape). Empty for a flushed index or a segment. Reuses [`TreeEntry`]:
    /// an assert reconstructs a fact, a retract is a tombstone.
    pub async fn novelty(&self, hash: &str) -> Result<Vec<TreeEntry>, String> {
        let rows = self.query("tree/novelty", json!({ "hash": hash })).await?;
        Ok(rows.iter().map(row_to_entry).collect())
    }
}

/// Decode a `tree/entry` or `tree/novelty` row into a [`TreeEntry`]; both
/// reconstruct a fact (or a retraction tombstone) from a key.
fn row_to_entry(r: &Row) -> TreeEntry {
    let f = &r.fields;
    TreeEntry {
        key_parts: parts(f, "key-parts"),
        retracted: f.get("retracted").and_then(|v| v.as_bool()) == Some(true),
        entity: s(f, "entity"),
        attribute: s(f, "attribute"),
        type_name: s(f, "type"),
        value: f.get("value").cloned(),
    }
}
