//! Wire types for the preview daemon: the render capability
//! payloads and the daemon↔page envelope.

use serde::{Deserialize, Serialize};
use tonk_schema::conclusion::Conclusion;

/// Capability name for template-preview rendering.
pub const CAPABILITY_RENDER_PREVIEW: &str = "render-preview";

/// Client → daemon payload for [`CAPABILITY_RENDER_PREVIEW`]:
/// the candidate template plus the already-projected live data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderRequest {
    /// Candidate `<tonk-view>` template HTML (inline, not yet a
    /// committed `view!:` row).
    pub template: String,
    /// Projected conclusions for `(model, this)` — what the real
    /// element would receive from its entity subscription.
    pub conclusions: Vec<Conclusion>,
}

/// Page → daemon reply payload: the real renderer's output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderReply {
    /// `innerHTML` of the `<tonk-view>` after rendering.
    pub html: String,
    /// How many conclusions were fed to the renderer.
    pub row_count: usize,
}

/// Daemon → page envelope. The daemon spine is capability-routed:
/// payloads are opaque JSON, parsed only by the matching handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageRequest {
    /// Correlation id matched against [`PageReply::id`].
    pub id: u64,
    /// Capability name (e.g. [`CAPABILITY_RENDER_PREVIEW`]).
    pub capability: String,
    /// Capability-specific payload.
    pub payload: serde_json::Value,
}

/// Page → daemon envelope completing a [`PageRequest`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageReply {
    /// Correlation id from the originating [`PageRequest`].
    pub id: u64,
    /// Capability-specific reply payload.
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    fn it_round_trips_the_page_envelope() {
        let request = PageRequest {
            id: 7,
            capability: "render-preview".into(),
            payload: serde_json::json!({"template": "<b>{x}</b>"}),
        };
        let json = serde_json::to_string(&request).unwrap();
        let back: PageRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, 7);
        assert_eq!(back.capability, "render-preview");
    }

    #[dialog_common::test]
    fn it_round_trips_a_render_request_with_conclusions() {
        let mut fields = std::collections::BTreeMap::new();
        fields.insert(
            "name".to_string(),
            ipld_core::ipld::Ipld::String("Alice".into()),
        );
        let request = RenderRequest {
            template: "<h1>{name}</h1>".into(),
            conclusions: vec![tonk_schema::conclusion::Conclusion {
                this: "did:key:zX".into(),
                fields,
            }],
        };
        let json = serde_json::to_string(&request).unwrap();
        let back: RenderRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.conclusions.len(), 1);
        assert_eq!(back.conclusions[0].this, "did:key:zX");
    }
}
