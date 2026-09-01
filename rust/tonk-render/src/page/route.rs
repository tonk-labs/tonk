//! The parsed render route — the `{entity}@{model}!{view}` shorthand
//! shared by the SW/display routes and `tonk render`.

use crate::page::RenderError;

/// A parsed render route.
///
/// Grammar (mirrors the SW/display route shorthand):
/// - `/{model}` — directory: every instance of the model.
/// - `/{entity}@{model}` — a single entity of the model.
/// - `/{entity}@{model}!{view}` — a single entity, explicit facet.
/// - `/{model}!{view}` — directory through an explicit facet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderRoute {
    /// The model concept name or URI.
    pub model: String,
    /// The target entity (bookmark name or URI). `None` => directory
    /// mode (render every instance).
    pub entity: Option<String>,
    /// The `show` facet to render (`label`, `title`, …). `None` =>
    /// the mode default: `ui` (entity set) or `directory`.
    pub view: Option<String>,
}

impl RenderRoute {
    /// Parse a route string. Leading `/` is optional. The `entity`
    /// is the part before `@`; the `view` is the part after `!`.
    pub fn parse(input: &str) -> Result<Self, RenderError> {
        let invalid = |msg: String| RenderError::Descriptor(msg);
        let s = input.strip_prefix('/').unwrap_or(input);
        if s.is_empty() {
            return Err(invalid("empty render route".into()));
        }
        // Split off the view (`!view`) first, from the end.
        let (head, view) = match s.split_once('!') {
            Some((h, v)) if !v.is_empty() => (h, Some(v.to_string())),
            Some((_, _)) => return Err(invalid("route has a trailing `!` with no view".into())),
            None => (s, None),
        };
        // Then split entity@model.
        let (entity, model) = match head.split_once('@') {
            Some((e, m)) if !e.is_empty() && !m.is_empty() => (Some(e.to_string()), m.to_string()),
            Some(_) => return Err(invalid(format!("route `{input}` has an empty side of `@`"))),
            None => (None, head.to_string()),
        };
        Ok(RenderRoute {
            model,
            entity,
            view,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_parses_a_bare_model_route() {
        let r = RenderRoute::parse("/person").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity, None);
        assert_eq!(r.view, None);
    }

    #[dialog_common::test]
    fn it_parses_entity_at_model() {
        let r = RenderRoute::parse("alice@person").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity.as_deref(), Some("alice"));
        assert_eq!(r.view, None);
    }

    #[dialog_common::test]
    fn it_parses_entity_at_model_bang_view() {
        let r = RenderRoute::parse("/alice@person!card").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity.as_deref(), Some("alice"));
        assert_eq!(r.view.as_deref(), Some("card"));
    }

    #[dialog_common::test]
    fn it_parses_directory_with_view() {
        let r = RenderRoute::parse("person!directory").unwrap();
        assert_eq!(r.model, "person");
        assert_eq!(r.entity, None);
        assert_eq!(r.view.as_deref(), Some("directory"));
    }

    #[dialog_common::test]
    fn it_keeps_a_did_key_entity_uri() {
        let r = RenderRoute::parse("did:key:zABC@person").unwrap();
        assert_eq!(r.entity.as_deref(), Some("did:key:zABC"));
        assert_eq!(r.model, "person");
    }

    #[dialog_common::test]
    fn it_rejects_empty_and_malformed_routes() {
        assert!(RenderRoute::parse("").is_err());
        assert!(RenderRoute::parse("/").is_err());
        assert!(RenderRoute::parse("@person").is_err());
        assert!(RenderRoute::parse("alice@").is_err());
        assert!(RenderRoute::parse("person!").is_err());
    }
}
