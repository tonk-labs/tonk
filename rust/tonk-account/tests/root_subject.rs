use dialog_capability::Subject;
use dialog_credentials::{Credential, Ed25519Signer, Ed25519Verifier};
use dialog_effects::space::{Space, SpaceExt as _};
use dialog_operator::helpers::test_operator_with_profile;
use dialog_repository::Repository;
use dialog_ucan::UcanDelegation;
use dialog_ucan_core::subject::Subject as UcanSubject;
use dialog_ucan_core::{DelegationBuilder, DelegationChain};
use dialog_varsig::Principal as _;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test_configure;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test_configure!(run_in_browser);

#[dialog_common::test]
async fn it_commits_a_root_subject_revision_without_storing_the_root_key() -> anyhow::Result<()> {
    let (operator, profile) = test_operator_with_profile().await;
    let root = Ed25519Signer::generate().await?;
    let root_did = root.did();

    let delegation = DelegationBuilder::new()
        .issuer(root)
        .audience(&profile.did())
        .subject(UcanSubject::Any)
        .command(vec![])
        .try_build()
        .await?;
    profile
        .access()
        .save(UcanDelegation(DelegationChain::new(delegation)))
        .perform(&operator)
        .await?;

    let verifier: Ed25519Verifier = root_did.to_string().parse()?;
    let local = Subject::from(profile.did()).attenuate(Space::new(root_did.to_string()));
    let credential = local
        .create(Credential::from(verifier))
        .perform(&operator)
        .await?;
    let repository = Repository::from(credential);

    assert!(
        matches!(repository.credential(), Credential::Verifier(_)),
        "the mounted repository must retain only the root verifier"
    );

    let branch = repository
        .branch(tonk_account::MAIN_BRANCH)
        .open()
        .perform(&operator)
        .await?;
    let revision = branch.transaction().commit().perform(&operator).await?;

    // The head no longer carries the subject DID; its opaque branch
    // entity commits to (profile, subject, name) instead.
    assert_eq!(
        revision.branch,
        dialog_repository::branch_of(&root_did, &profile.did(), tonk_account::MAIN_BRANCH)
    );
    assert!(
        matches!(repository.credential(), Credential::Verifier(_)),
        "committing through root → device → operator must not add the root signer"
    );

    Ok(())
}
