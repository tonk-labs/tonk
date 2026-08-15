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
    /// The subtree's advisory scale code — a log-scale estimate of how
    /// many entries sit beneath this node.
    pub scale: Option<u64>,
    /// Hitchhiker ops buffered on this node (always 0 for a segment).
    pub novelty: Option<u64>,
    /// The format manifest the node embeds: the configuration that
    /// produced this shape, as `(label, value)` rows in display order.
    pub manifest: Vec<(String, u64)>,
}

/// One entry in a segment — the `tree/entry` shape.
#[derive(Clone)]
pub struct TreeEntry {
    /// The entry key's decoded components (the worker's `key-parts`).
    pub key_parts: Vec<KeyPart>,
    pub retracted: bool,
    pub entity: Option<String>,
    pub attribute: Option<String>,
    pub type_name: Option<String>,
    pub value: Option<serde_json::Value>,
    /// Which index this key belongs to — `entity`, `attribute`,
    /// `value`, `history`, `coverage`, or `blob`. A segment holds more
    /// than facts, and only this says which kind a row is.
    pub ordering: Option<String>,
    /// Claim version: the origin half (hex) and the edition counter.
    pub origin: Option<String>,
    pub edition: Option<u64>,
    /// Prior versions in this entry's cause, and how many were folded
    /// into it. A covering record is the one that `supersedes` others.
    pub cause: Option<u64>,
    pub collapsed: Option<u64>,
    pub supersedes: Option<u64>,
    pub retraction: bool,
    /// A spilled value's block reference, when the value did not inline.
    pub spill: Option<String>,
    /// For a blob-index row: the referenced hash and its size.
    pub blob: Option<String>,
    pub blob_size: Option<u64>,
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

/// The node's embedded manifest as ordered `(label, value)` rows. The
/// order is the manifest's own reading order — format version first,
/// then the parameters that decide the tree's shape — rather than the
/// map's alphabetical one.
fn manifest_rows(map: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, u64)> {
    let Some(m) = map.get("manifest").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    [
        "version",
        "fanout",
        "max-separator",
        "inline",
        "spill-prefix",
        "max-segment",
    ]
    .into_iter()
    .filter_map(|key| {
        m.get(key)
            .and_then(|v| v.as_u64())
            .map(|value| (key.to_owned(), value))
    })
    .collect()
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
            scale: u(f, "scale"),
            novelty: u(f, "novelty"),
            manifest: manifest_rows(f),
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
        Ok(rows
            .iter()
            .map(|r| {
                let f = &r.fields;
                TreeEntry {
                    key_parts: parts(f, "key-parts"),
                    retracted: f.get("retracted").and_then(|v| v.as_bool()) == Some(true),
                    entity: s(f, "entity"),
                    attribute: s(f, "attribute"),
                    type_name: s(f, "type"),
                    value: f.get("value").cloned(),
                    ordering: s(f, "ordering"),
                    origin: s(f, "origin"),
                    edition: u(f, "edition"),
                    cause: u(f, "cause"),
                    collapsed: u(f, "collapsed"),
                    supersedes: u(f, "supersedes"),
                    retraction: f.get("retraction").and_then(|v| v.as_bool()) == Some(true),
                    spill: s(f, "spill"),
                    blob: s(f, "blob"),
                    blob_size: u(f, "blob-size"),
                }
            })
            .collect())
    }
}
