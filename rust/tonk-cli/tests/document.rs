//! Automerge documents from the CLI: the same commands and queries the
//! web uses, with the same result.

mod common;

use anyhow::Result;
use dialog_artifacts::Entity;
use tonk_document::engine::{Content, Edit, Format, Stamp};
use tonk_document::session;

const DOCUMENT_LIBRARY: &str = include_str!("../../tonk-core/assets/library/document.yaml");

fn text(snapshot: &session::Snapshot) -> String {
    match &snapshot.content {
        Content::Text(text) => text.clone(),
        Content::Table(_) => panic!("expected a text document"),
    }
}

#[dialog_common::test]
async fn it_runs_a_document_command_asserted_from_the_cli() -> Result<()> {
    let test = common::TestSite::new().await?;
    test.eval_inline(DOCUMENT_LIBRARY).await?;

    // An element created the document and typed into it.
    let doc: Entity = "id:prose/doc".parse().unwrap();
    {
        let session = test.site.branch().await?;
        session::write(
            session.handle(),
            &doc,
            Some(Format::Text),
            None,
            &[Edit::SetText {
                text: "hello world".into(),
            }],
            &Stamp::default(),
            &test.site.operator,
        )
        .await?;
    }

    // An agent edits it the way it edits anything: by asserting a
    // command. The CLI has no command registry; its write path
    // dispatches the document commands itself after the commit.
    let outcome = test
        .eval_inline(
            "document/replace!:\n  document: id:prose/doc\n  find: \"world\"\n  with: \"there\"\n",
        )
        .await?;
    assert!(outcome.committed);

    let session = test.site.branch().await?;
    let after = session::read(session.handle(), &doc, None, &test.site.operator).await?;
    assert_eq!(
        text(&after),
        "hello there",
        "the command edited the document"
    );

    // A refused command changes nothing.
    test.eval_inline(
        "document/replace!:\n  document: id:prose/doc\n  find: \"absent\"\n  with: \"x\"\n",
    )
    .await?;
    let still = session::read(session.handle(), &doc, None, &test.site.operator).await?;
    assert_eq!(text(&still), "hello there");
    Ok(())
}

#[dialog_common::test]
async fn it_reads_document_text_and_history_with_queries() -> Result<()> {
    let test = common::TestSite::new().await?;
    test.eval_inline(DOCUMENT_LIBRARY).await?;
    let doc: Entity = "id:prose/doc".parse().unwrap();
    let first = {
        let session = test.site.branch().await?;
        let first = session::write(
            session.handle(),
            &doc,
            Some(Format::Text),
            None,
            &[Edit::SetText { text: "one".into() }],
            &Stamp::default(),
            &test.site.operator,
        )
        .await?;
        session::write(
            session.handle(),
            &doc,
            None,
            None,
            &[Edit::SetText {
                text: "one two".into(),
            }],
            &Stamp::default(),
            &test.site.operator,
        )
        .await?;
        first
    };

    // The mirror: a plain concept query reads the document's text.
    let query = test
        .eval_inline("document/content:\n  this: ?doc\n  text: ?text\n")
        .await?;
    let rendered = format!("{:?}", query.response.matches_after);
    assert!(
        rendered.contains("one two"),
        "the mirror is queryable: {rendered}"
    );

    // History: the same formulas the worker's /query route resolves.
    let versions = tonk_cli::document::query_formula(
        &test.site,
        "document/versions",
        &[("document".into(), "id:prose/doc".into())],
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let versions: serde_json::Value = serde_json::from_str(&versions)?;
    assert_eq!(
        versions.as_array().map(Vec::len),
        Some(2),
        "one version per save"
    );

    let old = tonk_cli::document::query_formula(
        &test.site,
        "document/content",
        &[
            ("document".into(), "id:prose/doc".into()),
            ("heads".into(), first.snapshot.heads.join(" ")),
        ],
    )
    .await
    .map_err(anyhow::Error::msg)?;
    assert!(old.contains("\"one\""), "content at old heads: {old}");
    Ok(())
}
