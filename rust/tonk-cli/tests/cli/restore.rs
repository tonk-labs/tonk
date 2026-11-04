use anyhow::Result;
use dialog_artifacts::{Changes, Pick, Update, Value};
use dialog_repository::Revision;
use tonk_cli::{
    branches, log,
    restore::{self, RestoreError},
    site::TonkSite,
};

use crate::common::{ATTRIBUTE_DECL, CONCEPT_DECL, TestSite};

async fn fixture() -> Result<TestSite> {
    let test = TestSite::new().await?;
    test.eval_inline(ATTRIBUTE_DECL).await?;
    test.eval_inline(CONCEPT_DECL).await?;
    test.eval_inline("task!: &laundry\n  title: \"Pick up the laundry\"\n  done: false\n")
        .await?;
    Ok(test)
}

async fn head(site: &TonkSite) -> Result<Revision> {
    Ok(site.branch().await?.handle().revision().unwrap())
}

#[dialog_common::test]
async fn it_checks_out_the_exact_tree_preserves_main_and_writes_on_a_new_origin() -> Result<()> {
    let test = fixture().await?;
    test.eval_inline("task!: &other\n  title: \"Keep this task\"\n  done: false\n")
        .await?;
    let target = head(&test.site).await?;
    let target_log = log::revisions(&test.site, 100).await?;
    test.eval_inline("task!:\n  this: laundry\n  title: \"Wednesday laundry\"\n  done: true\n")
        .await?;
    test.eval_inline("task!:\n  this: other\n  ..: _\n").await?;
    test.eval_inline("task!: &later\n  title: \"Added later\"\n  done: true\n")
        .await?;
    test.eval_inline(crate::common::NOTE_ATTRIBUTE_DECL).await?;
    test.eval_inline(crate::common::NOTE_CONCEPT_DECL).await?;
    let before = head(&test.site).await?;
    let before_log = log::revisions(&test.site, 100).await?;

    let plan = restore::plan(&test.site, &target.version().to_string()).await?;
    assert_eq!(
        head(&test.site).await?,
        before,
        "preview leaves the active head alone"
    );
    assert_eq!(branches::list(&test.site).await?.len(), 1);
    restore::apply(&test.site, &plan).await?;
    assert_eq!(
        head(&test.site).await?,
        target,
        "checkout uses the exact signed tree, including schemas and history"
    );
    assert_eq!(branches::active_name(&test.site).await?, plan.branch);
    assert_eq!(
        log::revisions(&test.site, 100).await?.len(),
        target_log.len(),
        "checkout mints no content revision"
    );
    let main = test.site.named_branch("main").await?;
    assert_eq!(main.handle().revision(), Some(before.clone()));
    assert_eq!(
        main.handle().log(&test.site.operator, 100).await?.len(),
        before_log.len()
    );

    test.eval_inline("task!:\n  this: laundry\n  title: \"Collect it tomorrow\"\n")
        .await?;
    let edited = head(&test.site).await?;
    assert_ne!(
        edited.origin(),
        target.origin(),
        "future writes have a fresh branch origin"
    );
    assert_eq!(
        log::revisions(&test.site, 1).await?[0].parents,
        vec![target.version().to_string()]
    );
    assert_eq!(main.handle().revision(), Some(before.clone()));

    let reopened = TonkSite::open_with(&test.site.root, test.config.clone()).await?;
    assert_eq!(branches::active_name(&reopened).await?, plan.branch);
    assert_eq!(
        head(&reopened).await?,
        edited,
        "selection persists across processes"
    );
    branches::switch(&reopened, "main").await?;
    assert_eq!(head(&reopened).await?, before);
    branches::switch(&reopened, &plan.branch).await?;
    assert_eq!(head(&reopened).await?, edited);
    Ok(())
}

#[dialog_common::test]
async fn it_accepts_a_tree_hash_and_can_return_to_a_later_saved_revision() -> Result<()> {
    let test = fixture().await?;
    let target = head(&test.site).await?;
    test.eval_inline("task!:\n  this: laundry\n  done: true\n")
        .await?;
    let later = head(&test.site).await?;
    let plan = restore::plan(&test.site, &target.tree.to_string()).await?;
    restore::apply(&test.site, &plan).await?;
    assert_eq!(head(&test.site).await?, target);
    let plan = restore::plan(&test.site, &later.version().to_string()).await?;
    restore::apply(&test.site, &plan).await?;
    assert_eq!(head(&test.site).await?, later);
    Ok(())
}

#[dialog_common::test]
async fn it_leaves_the_current_revision_on_its_existing_branch() -> Result<()> {
    let test = fixture().await?;
    let target = head(&test.site).await?;
    let plan = restore::plan(&test.site, &target.version().to_string()).await?;
    assert!(!plan.switches());
    restore::apply(&test.site, &plan).await?;
    assert_eq!(branches::active_name(&test.site).await?, "main");
    assert_eq!(branches::list(&test.site).await?.len(), 1);
    assert_eq!(head(&test.site).await?, target);
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_missing_snapshots_and_unknown_branches_without_switching() -> Result<()> {
    let test = fixture().await?;
    let before = head(&test.site).await?;
    let missing = format!("9999@{}", before.origin());
    assert!(matches!(
        restore::plan(&test.site, &missing).await,
        Err(RestoreError::Unavailable(_))
    ));
    assert!(matches!(
        restore::plan(&test.site, "not-a-revision").await,
        Err(RestoreError::Invalid(_))
    ));
    assert!(
        branches::switch(&test.site, "does-not-exist")
            .await
            .is_err()
    );
    assert_eq!(head(&test.site).await?, before);
    assert_eq!(branches::active_name(&test.site).await?, "main");
    assert_eq!(branches::list(&test.site).await?.len(), 1);
    Ok(())
}

#[dialog_common::test]
async fn it_refuses_a_plan_after_another_writer_advances_the_source() -> Result<()> {
    let test = fixture().await?;
    let target = head(&test.site).await?;
    test.eval_inline("task!:\n  this: laundry\n  done: true\n")
        .await?;
    let plan = restore::plan(&test.site, &target.version().to_string()).await?;
    let other = TonkSite::open_with(&test.site.root, test.config.clone()).await?;
    tonk_cli::eval::run_against_site(
        &other,
        tonk_cli::eval::Source::Inline(
            "task!:\n  this: laundry\n  title: \"Another writer\"\n".into(),
        ),
        tonk_cli::eval::Options::default(),
    )
    .await?;
    assert!(matches!(
        restore::apply(&test.site, &plan).await,
        Err(RestoreError::Changed)
    ));
    assert_eq!(branches::active_name(&test.site).await?, "main");
    assert_eq!(branches::list(&test.site).await?.len(), 1);
    Ok(())
}

#[dialog_common::test]
async fn it_restores_many_valued_claims_without_reconstructing_their_history() -> Result<()> {
    let test = fixture().await?;
    let session = test.site.branch().await?;
    let mut changes = Changes::new();
    for value in ["one", "two"] {
        changes.associate(
            "test.tag/value".parse()?,
            "id:tags".parse()?,
            Value::String(value.into()),
            Pick::All,
        );
    }
    let target = session
        .handle()
        .transaction()
        .integrate(changes)
        .commit()
        .publish()
        .perform(&test.site.operator)
        .await?;
    let mut remove = Changes::new();
    remove.dissociate(
        "test.tag/value".parse()?,
        "id:tags".parse()?,
        Value::String("one".into()),
    );
    session
        .handle()
        .transaction()
        .integrate(remove)
        .commit()
        .publish()
        .perform(&test.site.operator)
        .await?;
    let plan = restore::plan(&test.site, &target.version().to_string()).await?;
    restore::apply(&test.site, &plan).await?;
    assert_eq!(head(&test.site).await?, target);
    Ok(())
}

#[dialog_common::test]
async fn it_does_not_inherit_the_old_branch_upstream() -> Result<()> {
    let test = fixture().await?;
    let target = head(&test.site).await?;
    test.eval_inline("task!:\n  this: laundry\n  done: true\n")
        .await?;
    let main = test.site.branch().await?;
    let upstream = test.site.named_branch("upstream").await?;
    upstream
        .handle()
        .reset(target.clone())
        .perform(&test.site.operator)
        .await?;
    main.handle()
        .set_upstream(upstream.handle())
        .perform(&test.site.operator)
        .await?;
    let plan = restore::plan(&test.site, &target.version().to_string()).await?;
    restore::apply(&test.site, &plan).await?;
    assert!(tonk_account::peer::upstream(test.site.branch().await?.handle()).is_none());
    assert!(tonk_account::peer::upstream(main.handle()).is_some());
    assert!(matches!(
        tonk_cli::sync::pull(&test.site).await,
        Err(tonk_cli::sync::SyncError::UpstreamNotConfigured { .. })
    ));
    assert_eq!(head(&test.site).await?, target);
    Ok(())
}

#[dialog_common::test]
async fn it_does_not_invent_a_snapshot_for_an_older_log_entry() -> Result<()> {
    let test = fixture().await?;
    let session = test.site.branch().await?;
    // Simulate commits made before the CLI retained complete heads. Their
    // records remain in the log, but no signed historical tree was saved.
    let target = session
        .handle()
        .transaction()
        .commit()
        .allow_empty()
        .publish()
        .perform(test.site.operator.inner())
        .await?;
    let before = session
        .handle()
        .transaction()
        .commit()
        .allow_empty()
        .publish()
        .perform(test.site.operator.inner())
        .await?;
    assert!(
        log::revisions(&test.site, 100)
            .await?
            .iter()
            .any(|row| row.version == target.version().to_string())
    );
    assert!(matches!(
        restore::plan(&test.site, &target.version().to_string()).await,
        Err(RestoreError::Unavailable(_))
    ));
    assert_eq!(head(&test.site).await?, before);
    Ok(())
}
