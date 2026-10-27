//! The Tonk kernel, natively on the polyfill: Tonk's worker inside a component,
//! on the storage and credentials of its host.
#![cfg(not(target_arch = "wasm32"))]

use futures::StreamExt as _;
use tonk_kernel_host::credentials::Standing;
use tonk_kernel_host::kernel::Kernel;
use tonk_kernel_host::layers::{self, Layers};
use tonk_kernel_host::sapling::did_key;
use wcmp::Engine;

fn engine() -> Engine {
    Engine::with_backend(wcmp_wasm_core_wasmtime::Wasmtime::new().unwrap()).unwrap()
}

/// Layers of a test's own, empty, in a directory named for `test`: a kernel
/// that starts again over them finds what the one before kept.
fn scratch(test: &str) -> Layers {
    let path = std::env::temp_dir().join(format!("tonk-kernel-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    Layers::under(path.to_string_lossy().into_owned())
}

fn space() -> String {
    did_key(&ed25519_dalek::SigningKey::from_bytes(&[3; 32]))
}

/// The standing every kernel here boots with: the same key, so a kernel that
/// starts again is the same worker, and no delegation to claim.
fn standing() -> Standing {
    Standing::new([7; 32], Vec::new())
}

/// Boot a kernel over the layers in `layers`, and answer who it is.
async fn identify(engine: &Engine, bytes: &[u8], layers: &Layers) -> String {
    let mut kernel = Kernel::load(engine, bytes, layers.clone()).await.unwrap();
    kernel.init(&space(), standing()).await.unwrap();
    let answer = kernel.request("GET", "/api/identify", b"").await.unwrap();
    assert_eq!(answer.status, 200, "{}", answer.text());
    answer.text()
}

/// The kernel instantiates against the world alone, boots Tonk's worker, and
/// answers its API.
#[tokio::test(flavor = "current_thread")]
async fn it_answers_with_tonk_s_worker() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;
    let mut kernel = Kernel::load(&engine, bytes, scratch("answers"))
        .await
        .unwrap();
    kernel.init(&space(), standing()).await.unwrap();

    for path in ["/api/identify", "/api/profile"] {
        let answer = kernel.request("GET", path, b"").await.unwrap();
        assert_eq!(answer.status, 200, "{path}: {}", answer.text());
        println!("{path} -> {}", answer.text());
    }
    println!("timings: {:?}", kernel.timings);
}

/// A kernel keeps no key of its own: it acts as its session, whose key its
/// host holds, so it keeps no custody, and a kernel that starts again over
/// what it kept is the same worker.
#[tokio::test(flavor = "current_thread")]
async fn it_keeps_no_key_of_its_own() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;
    let kept = scratch("no-keys");
    let first = identify(&engine, bytes, &kept).await;
    let credentials = kept
        .open("did:local:profile:dialog.credential")
        .await
        .unwrap();
    let custodies = layers::list(&credentials, "custody").await.unwrap();
    assert_eq!(
        custodies,
        Vec::<String>::new(),
        "the kernel keeps no custody"
    );

    let again = identify(&engine, bytes, &kept).await;
    assert_eq!(
        first, again,
        "a kernel that starts again is the same worker"
    );
}

/// A served kernel answers each request while another's answer is still
/// being written: a subscription stays open, and a request made after it is
/// answered all the same.
#[tokio::test(flavor = "current_thread")]
async fn it_answers_while_a_subscription_is_open() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;
    let mut kernel = Kernel::load(&engine, bytes, scratch("subscription"))
        .await
        .unwrap();
    kernel.init(&space(), standing()).await.unwrap();
    let (served, driver) = kernel.serve();

    let asked = async {
        let query = tonk_worker::helpers::named_concept_wire_query().to_string();
        let mut subscription = served
            .request(
                "POST",
                "/api/repository/profile:tonk/branch/main/query?level=2",
                vec![
                    ("accept".to_owned(), b"text/event-stream".to_vec()),
                    ("content-type".to_owned(), b"application/json".to_vec()),
                ],
                query.as_bytes(),
            )
            .await
            .unwrap();
        assert_eq!(subscription.status, 200);
        let first = subscription.body.next().await;
        assert!(first.is_some(), "the subscription answers its first frame");

        let identify = served
            .request("GET", "/api/identify", Vec::new(), b"")
            .await
            .unwrap();
        assert_eq!(identify.status, 200);
        let said: Vec<Vec<u8>> = identify.body.collect().await;
        let said = String::from_utf8(said.concat()).unwrap();
        assert!(said.contains("did:key:"), "{said}");
        // The subscription is still open: its body has not ended.
        drop(subscription);
    };
    let asked = tokio::time::timeout(std::time::Duration::from_secs(60), asked);
    tokio::select! {
        answered = asked => answered.expect("the kernel answered within a minute"),
        stopped = driver => panic!("the kernel stopped while asked: {stopped:?}"),
    }
}

/// A kernel's peer is its session: it acts with the key its host hands it.
#[tokio::test(flavor = "current_thread")]
async fn it_acts_as_its_session() {
    let engine = engine();
    let said = identify(
        &engine,
        tonk_components::TONK_KERNEL_BYTES,
        &scratch("session"),
    )
    .await;
    let session = did_key(&ed25519_dalek::SigningKey::from_bytes(&[7; 32]));
    assert!(said.contains(&session), "{said} names {session}");
}

/// A kernel that serves its session's own space is a person's profile: its
/// peer is its session, and it creates a space with a key the host makes,
/// whose seed it never knows.
#[tokio::test(flavor = "current_thread")]
async fn it_is_a_person_s_profile_on_its_session_s_own_space() {
    let engine = engine();
    let session = did_key(&ed25519_dalek::SigningKey::from_bytes(&[7; 32]));
    let mut kernel = Kernel::load(
        &engine,
        tonk_components::TONK_KERNEL_BYTES,
        scratch("profile"),
    )
    .await
    .unwrap();
    kernel.init(&session, standing()).await.unwrap();
    for path in ["/api/identify", "/api/profiles", "/api/profile"] {
        let answer = kernel.request("GET", path, b"").await.unwrap();
        assert_eq!(answer.status, 200, "{path}: {}", answer.text());
        println!("{path} -> {}", answer.text());
    }
    let said = kernel.request("GET", "/api/identify", b"").await.unwrap();
    assert!(
        said.text().contains(&session),
        "{} names {session}",
        said.text()
    );

    let create = serde_json::json!({ "claims": [{ "op": "assert", "application": {
        "predicate": { "kind": "transient", "concept": { "with": {
            "name": { "the": "xyz.tonk.command.create-space/name", "as": "Text" }
        } } },
        "parameters": { "this": "cmd:create", "name": "Untitled" }
    } }] });
    let path = format!("/api/repository/{session}/branch/main/transact");
    let answer = kernel
        .request("POST", &path, create.to_string().as_bytes())
        .await
        .unwrap();
    assert_eq!(answer.status, 200, "{}", answer.text());

    let listed = kernel.request("GET", "/api/profile", b"").await.unwrap();
    let listed: serde_json::Value = serde_json::from_str(&listed.text()).unwrap();
    let space = listed["space"][0]["subject"]
        .as_str()
        .expect("the profile lists the space it created")
        .to_owned();
    let keys = kernel.keys();
    assert!(
        keys.contains(&(space.clone(), false)),
        "the space's key is one the host made: {keys:?}"
    );
    assert!(
        keys.iter().all(|(_, imported)| !imported),
        "the kernel imports no seed it knows: {keys:?}"
    );
}

/// The delegation of the space to the session's key, signed by the space,
/// with the account it is for in its meta, as the person's profile signs it.
async fn space_to_session() -> Vec<u8> {
    use dialog_ucan_core::subject::Subject;
    use dialog_ucan_core::{DelegationBuilder, DelegationChain};
    use ipld_core::ipld::Ipld;

    let space = dialog_credentials::Ed25519Signer::import(&[3; 32])
        .await
        .unwrap();
    let session: dialog_varsig::Did = did_key(&ed25519_dalek::SigningKey::from_bytes(&[7; 32]))
        .parse()
        .unwrap();
    let account = did_key(&ed25519_dalek::SigningKey::from_bytes(&[9; 32]));
    let delegation = DelegationBuilder::new()
        .issuer(dialog_credentials::Signer::from(space))
        .audience(&session)
        .subject(Subject::Specific(space_did()))
        .command(vec!["use".to_owned()])
        .meta([("acts.for".to_owned(), Ipld::String(account))].into())
        .try_build()
        .await
        .unwrap();
    DelegationChain::new(delegation).to_bytes().unwrap()
}

fn space_did() -> dialog_varsig::Did {
    space().parse().unwrap()
}

/// What the kernel says of the space.
async fn holds(kernel: &mut Kernel) -> tonk_kernel_host::kernel::Answer {
    let path = format!("/api/repository/{}", space());
    kernel.request("GET", &path, b"").await.unwrap()
}

/// The first boot takes the space up with the delegation its session claims.
/// A boot after it is handed none, and holds the space all the same: the
/// kernel kept what it took up.
#[tokio::test(flavor = "current_thread")]
async fn it_takes_up_its_space_from_its_session() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;

    let kept = scratch("take-up");
    let mut kernel = Kernel::load(&engine, bytes, kept.clone()).await.unwrap();
    kernel
        .init(
            &space(),
            Standing::new([7; 32], vec![space_to_session().await]),
        )
        .await
        .unwrap();
    let answer = holds(&mut kernel).await;
    assert_eq!(
        answer.status,
        200,
        "the kernel holds its space: {}",
        answer.text()
    );

    let mut again = Kernel::load(&engine, bytes, kept).await.unwrap();
    again.init(&space(), standing()).await.unwrap();
    let answer = holds(&mut again).await;
    assert_eq!(
        answer.status,
        200,
        "a kernel handed no delegation holds what it took up: {}",
        answer.text()
    );
}

/// A kernel handed no delegation, and holding none, does not hold its space.
#[tokio::test(flavor = "current_thread")]
async fn it_holds_no_space_it_was_not_delegated() {
    let engine = engine();
    let mut kernel = Kernel::load(
        &engine,
        tonk_components::TONK_KERNEL_BYTES,
        scratch("not-delegated"),
    )
    .await
    .unwrap();
    kernel.init(&space(), standing()).await.unwrap();
    assert_ne!(holds(&mut kernel).await.status, 200);
}

/// The build names the kernel as a space's boot cell pins it.
#[test]
fn it_names_the_kernel_the_way_a_space_pins_it() {
    assert_eq!(
        tonk_kernel_host::boot::kernel_id(tonk_components::TONK_KERNEL_BYTES),
        tonk_components::TONK_KERNEL_ID
    );
}
