//! Behavioural tests for the concept schema-read API
//! (`tonk_cli::schema::find_concept`): looking up a single named
//! concept's fields, types, and cardinalities off the branch.

mod common;

use anyhow::Result;

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};

mod when_reading_a_concepts_schema {
    use super::*;

    #[dialog_common::test]
    async fn it_returns_fields_types_and_cardinality_for_a_named_concept() -> Result<()> {
        let test = TestSite::new().await?;
        test.eval_inline(ATTRIBUTE_DECL).await?; // seeds task-title / task-done
        test.eval_inline(CONCEPT_DECL).await?; // seeds the `task` concept
        let info = tonk_cli::schema::find_concept(&test.site, "task")
            .await?
            .expect("task concept should be found");
        assert_eq!(info.name, "task");
        let fields: Vec<&str> = info.descriptor.with().iter().map(|(f, _)| f).collect();
        assert!(
            fields.contains(&"title"),
            "task should have a title field, got {fields:?}"
        );
        Ok(())
    }

    #[dialog_common::test]
    async fn it_returns_none_for_an_unknown_concept() -> Result<()> {
        let test = TestSite::new().await?;
        assert!(
            tonk_cli::schema::find_concept(&test.site, "nope")
                .await?
                .is_none()
        );
        Ok(())
    }
}

mod when_rendering_an_overview {
    use tonk_cli::schema::{ConceptSummary, FieldSummary, render_overview};

    fn concept(name: &str, fields: usize) -> ConceptSummary {
        ConceptSummary {
            name: name.into(),
            entity: "id:hidden-schema-identity".into(),
            description: None,
            fields: (0..fields).map(|i| format!("field{i}")).collect(),
            field_specs: (0..fields)
                .map(|i| FieldSummary {
                    name: format!("field{i}"),
                    value_type: "text".into(),
                    cardinality: if i == 0 { "many" } else { "one" }.into(),
                    required: i != 0,
                    description: "Detailed field prose belongs in named show".into(),
                })
                .collect(),
        }
    }

    #[test]
    fn overview_preserves_field_semantics_without_runtime_schema_or_raw_details() {
        let concepts = [concept("task", 2), concept("tonk/repository", 2)];
        let text = render_overview("demo", &concepts, false);
        assert!(
            text.contains("task  field0: text[]?, field1: text"),
            "{text}"
        );
        assert!(!text.contains("tonk/repository"), "{text}");
        assert!(!text.contains("hidden-schema-identity"), "{text}");
        assert!(!text.contains("Detailed field prose"), "{text}");
        assert!(text.contains("Runtime concepts omitted"), "{text}");
        let all = render_overview("demo", &concepts, true);
        assert!(all.contains("tonk/repository  field0: text[]?"), "{all}");
    }

    #[test]
    fn large_overviews_bound_concepts_fields_and_multiline_unicode_descriptions() {
        let mut concepts: Vec<_> = (0..21)
            .map(|i| concept(&format!("model{i:02}"), 10))
            .collect();
        concepts[0].description = Some(format!("line\nbreak {}", "é".repeat(100)));
        let text = render_overview("demo", &concepts, false);
        assert!(
            text.contains("1 more concepts; use tonk show --all"),
            "{text}"
        );
        assert!(!text.contains("model20"), "{text}");
        assert!(text.contains("2 more fields (tonk show model00)"), "{text}");
        assert!(!text.contains("field8:"), "{text}");
        assert!(text.contains("line break"), "{text}");
        assert!(!text.contains(&"é".repeat(81)), "{text}");
        assert!(text.lines().count() < 40, "{text}");
        let all = render_overview("demo", &concepts, true);
        assert!(all.contains("model20"), "{all}");
        assert!(all.contains("field9: text"), "{all}");
    }

    #[test]
    fn a_space_with_only_runtime_concepts_explains_how_to_start() {
        let text = render_overview("empty", &[concept("tonk/repository", 2)], false);
        assert!(text.contains("Concepts (0)"), "{text}");
        assert!(
            text.contains("No application concepts. Define one"),
            "{text}"
        );
    }
}

#[dialog_common::test]
async fn stored_descriptions_are_recovered_without_hiding_undescribed_concepts() -> Result<()> {
    let test = TestSite::new().await?;
    tonk_cli::data_ops::concept_add(
        &test.site,
        "described",
        &["title:text:one".into()],
        Some("Work that needs doing"),
        Default::default(),
    )
    .await?;
    test.eval_inline("concept!: &plain\n  with:\n    title: { description: Title, the: test.plain/title, as: text }\n")
        .await?;
    let concepts = tonk_cli::schema::list_concepts(&test.site).await?;
    assert_eq!(
        concepts
            .iter()
            .find(|c| c.name == "described")
            .unwrap()
            .description
            .as_deref(),
        Some("Work that needs doing")
    );
    assert!(
        concepts
            .iter()
            .any(|c| c.name == "plain" && c.description.is_none())
    );
    let text = tonk_cli::schema::render_overview("test", &concepts, false);
    assert!(text.contains("Work that needs doing"), "{text}");
    Ok(())
}
