mod common;

#[dialog_common::test]
async fn review_prose_query_does_not_match_workbooks() -> anyhow::Result<()> {
    let test = common::TestSite::new().await?;
    test.eval_inline(include_str!("../../tonk-core/assets/library/document.yaml"))
        .await?;
    test.eval_inline(include_str!("../../tonk-core/assets/library/prose.yaml"))
        .await?;
    test.eval_inline(include_str!("../../tonk-core/assets/library/table.yaml"))
        .await?;
    let query = test
        .eval_inline("prose:\n  this: ?doc\n  format: ?format\n")
        .await?;
    let result = format!("{:?}", query.response.matches_after);
    assert!(
        !result.contains("automerge/table@1"),
        "prose unexpectedly matches a workbook: {result}"
    );
    test.eval_inline("prose!:\n  this: id:review/empty-prose\n  format: automerge/text@1\n")
        .await?;
    let session = test.site.branch().await?;
    let entity = "id:review/empty-prose".parse()?;
    let opened =
        tonk_document::session::read(session.handle(), &entity, None, &test.site.operator).await?;
    assert_eq!(
        opened.content,
        tonk_document::Content::Text(String::new()),
        "prose assertions must still create documents"
    );
    Ok(())
}
