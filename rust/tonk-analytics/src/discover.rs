//! Discover attribution and space-entry boundaries. Raw references stay local.

use serde_json::{Value, json};

/// Successful creation, optionally attributed to a validated catalog template.
/// The complete catalog URL plus fragment identifies a template across releases.
pub fn created_properties(space: &str, template: Option<&str>) -> Value {
    let mut properties =
        crate::launch::space_conversion_properties(crate::launch::SpaceConversion::Created, space);
    if let Some(template) = template {
        properties["template_id"] = crate::anonymize(template).into();
    }
    properties
}

/// Page-local navigation state; no storage or cross-tab coordination is added.
#[derive(Default)]
pub struct SpaceEntries {
    current: Option<String>,
}

impl SpaceEntries {
    /// Emit once when entering a different space, including initial deep links.
    /// Leaving for the Hub resets the boundary. Reloads count as fresh entries;
    /// use distinct profile/space/day tuples for return metrics, not event totals.
    pub fn navigate(&mut self, path: &str) -> Option<Value> {
        let next = path
            .strip_prefix("/space/")
            .and_then(|tail| tail.split('/').next())
            .filter(|key| !key.is_empty())
            .map(crate::anonymize);
        if self.current == next {
            return None;
        }
        self.current = next;
        self.current.as_ref().map(|space| {
            json!({
                "schema_version": 1,
                "space_id": space,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_only_templates_without_sending_raw_references() {
        let reference = "https://example.com/catalog.json#notebook";
        let props = created_properties("private-space", Some(reference));
        assert_eq!(props["template_id"], crate::anonymize(reference));
        assert_eq!(props["space_id"], crate::anonymize("private-space"));
        assert_eq!(props["conversion"], "created");
        assert!(!props.to_string().contains("example.com"));
        assert!(!props.to_string().contains("private-space"));
        assert!(
            created_properties("space", None)
                .get("template_id")
                .is_none()
        );
        assert_ne!(
            props["template_id"],
            created_properties("space", Some("https://example.com/catalog.json#other"))["template_id"]
        );
    }

    #[test]
    fn entries_exclude_internal_navigation_but_include_returns() {
        let mut entries = SpaceEntries::default();
        assert!(entries.navigate("/").is_none());
        assert_eq!(
            entries.navigate("/space/a/view/start").unwrap()["space_id"],
            crate::anonymize("a")
        );
        assert!(entries.navigate("/space/a/view/next").is_none());
        assert!(entries.navigate("/space/a").is_none());
        assert!(entries.navigate("/space/b").is_some());
        assert!(entries.navigate("/space/a").is_some());
        assert!(entries.navigate("/").is_none());
        assert!(entries.navigate("/space/a").is_some());
        assert!(entries.navigate("/space/").is_none());
        assert!(entries.navigate("/seed/https://example.com").is_none());
        assert!(SpaceEntries::default().navigate("/space/a").is_some());
    }
}
