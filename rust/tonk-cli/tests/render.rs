//! Behavioural tests for `tonk render`: the full model -> view ->
//! entity resolution and HTML rendering against a seeded `.tonk`
//! site. The route-parser unit tests live in `src/render.rs`; these
//! exercise the resolve / render / recurse / fallback paths that need
//! real branch data.

mod common;

use anyhow::Result;
use tonk_cli::render::{self, RenderRoute};

use crate::common::TestSite;

// Site init seeds the standard library (the `tonk:view` concept and
// the `tonk:_` default dictionary), so these tests rely on the
// built-in view concept being present rather than seeding it by hand.

const PERSON_CONCEPT: &str = r#"attribute!: &person-name
  description: "person name"
  the: xyz.tonk.person/name
  as: text
  cardinality: one

concept!: &person
  description: "a person"
  with:
    name: person-name
"#;

/// A site (with the built-in standard library) plus the `person`
/// concept and a `person-card` detail view. Tests add instances /
/// extra views as needed.
async fn seeded() -> Result<TestSite> {
    let test = TestSite::new().await?;
    test.eval_inline(PERSON_CONCEPT).await?;
    test.eval_inline(
        r#"view!:
  this: person
  show:
    ui: "<article><h2>{name}</h2></article>"
"#,
    )
    .await?;
    Ok(test)
}

#[dialog_common::test]
async fn it_seeds_the_standard_library_on_init() -> Result<()> {
    // A fresh site resolves the standard library's `view` concept
    // without any manual seeding — proof that site init lowered
    // core.yaml. Asked by name: `list_concepts` is the author-facing
    // listing and deliberately omits everything init seeded.
    let test = TestSite::new().await?;
    assert!(
        tonk_cli::schema::find_concept(&test.site, "view")
            .await?
            .is_some(),
        "the standard library's `view` concept should be seeded at init"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_renders_one_entity_through_its_view() -> Result<()> {
    let test = seeded().await?;
    test.eval_inline("person!: &alice\n  name: Alice\n").await?;

    let route = RenderRoute::parse("alice@person")?;
    let html = render::render(&test.site, &route).await?;

    assert!(html.contains("<h2>Alice</h2>"), "rendered name: {html}");
    assert!(html.starts_with("<article"), "view chrome: {html}");
    // The repeat row carries a `with=` stamp keyed to the entity.
    assert!(html.contains("with=\"did:key:"), "repeat stamp: {html}");
    Ok(())
}

#[dialog_common::test]
async fn it_injects_dom_host_fields_for_nested_resolution() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(PERSON_CONCEPT).await?;
    // A view that reads {dom.host/model} into an attribute.
    test.eval_inline(
        r#"view!:
  this: person
  show:
    ui: "<article data-model=\"{dom.host/model}\"><h2>{name}</h2></article>"
"#,
    )
    .await?;
    test.eval_inline("person!: &bob\n  name: Bob\n").await?;

    let route = RenderRoute::parse("bob@person")?;
    let html = render::render(&test.site, &route).await?;
    // {dom.host/model} resolves to the route's model name.
    assert!(
        html.contains("data-model=\"person\""),
        "dom.host/model resolved: {html}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_renders_a_directory_of_every_instance() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(PERSON_CONCEPT).await?;
    // A person-specific `directory` facet. Overrides the stdlib's
    // `tonk:_` default carousel so the test asserts this exact template.
    test.eval_inline(
        r#"view!:
  this: person
  show:
    directory: "<ul><li data-id=\"{this}\">{name}</li></ul>"
"#,
    )
    .await?;
    test.eval_inline("person!: &ann\n  name: Ann\n").await?;
    test.eval_inline("person!: &bo\n  name: Bo\n").await?;

    // No entity -> directory mode: one <li> per instance.
    let route = RenderRoute::parse("person")?;
    let html = render::render(&test.site, &route).await?;
    assert!(html.contains("Ann"), "first instance: {html}");
    assert!(html.contains("Bo"), "second instance: {html}");
    assert_eq!(
        html.matches("<li").count(),
        2,
        "one row per instance: {html}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_errors_when_no_view_exists_for_the_model() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(PERSON_CONCEPT).await?;
    test.eval_inline("person!: &alice\n  name: Alice\n").await?;
    // No view declared for `person` and no `tonk:_` default seeded.
    let route = RenderRoute::parse("alice@person")?;
    let err = render::render(&test.site, &route).await.unwrap_err();
    assert!(
        err.to_string().contains("no view found"),
        "expected no-view error, got: {err}"
    );
    Ok(())
}

// The `tonk:_` wildcard-model sentinel: a view keyed to it renders
// any model that has no specific view. `tonk:_` is a valid entity
// URI, so it satisfies the `model` field's `as: entity` type under
// dialog's strict Entity/Text typing (the old `_:_` text sentinel
// did not — see BUG-16).
#[dialog_common::test]
async fn it_falls_back_to_the_default_model_view() -> Result<()> {
    let test = TestSite::new().await?;
    test.eval_inline(PERSON_CONCEPT).await?;
    // A view keyed to the `tonk:_` default model rather than `person`.
    test.eval_inline(
        r#"view!:
  this: tonk:_
  show:
    ui: "<div class=\"default\">{name}</div>"
"#,
    )
    .await?;
    test.eval_inline("person!: &alice\n  name: Alice\n").await?;

    let route = RenderRoute::parse("alice@person")?;
    let html = render::render(&test.site, &route).await?;
    assert!(
        html.contains("class=\"default\"") && html.contains("Alice"),
        "default-model view rendered: {html}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_renders_empty_chrome_for_a_missing_entity() -> Result<()> {
    let test = seeded().await?;
    // No instance asserted; the entity name doesn't resolve.
    let route = RenderRoute::parse("ghost@person")?;
    let err = render::render(&test.site, &route).await.unwrap_err();
    assert!(
        err.to_string().contains("no entity named"),
        "expected name-resolution error, got: {err}"
    );
    Ok(())
}

#[dialog_common::test]
async fn it_errors_on_an_unknown_model() -> Result<()> {
    let test = seeded().await?;
    let route = RenderRoute::parse("nope")?;
    let err = render::render(&test.site, &route).await.unwrap_err();
    assert!(
        err.to_string().contains("no concept matched"),
        "expected unknown-model error, got: {err}"
    );
    Ok(())
}
