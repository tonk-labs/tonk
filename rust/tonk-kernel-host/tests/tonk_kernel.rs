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

/// The standing every space's kernel here boots with: the same key, so a
/// kernel that starts again is the same worker, and nothing delegated.
fn standing() -> Standing {
    Standing::handed([7; 32], None)
}

/// The standing every persona here boots with: the same key, provisional.
fn persona_standing() -> Standing {
    Standing::new([7; 32])
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

/// A kernel keeps everything in the layer of its stack, and no key of its
/// own there: it acts as its session, whose key its host holds. A kernel
/// that starts again over what it kept is the same worker.
#[tokio::test(flavor = "current_thread")]
async fn it_keeps_everything_in_its_stack_and_no_key() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;
    let root = std::env::temp_dir().join(format!("tonk-kernel-stack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let kept = Layers::under(root.to_string_lossy().into_owned());
    let first = identify(&engine, bytes, &kept).await;

    let stores: Vec<_> = walkdir(&root);
    assert_eq!(
        stores,
        vec![format!("current/{}", space())],
        "the host keeps the layer of the stack alone"
    );
    let stack = kept.open(&space()).await.unwrap();
    let custodies = layers::list(&stack, "did:local:profile:dialog.credential/custody")
        .await
        .unwrap();
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

/// Each store under `root`: a directory two levels down, as `Layers::under`
/// keeps a location.
fn walkdir(root: &std::path::Path) -> Vec<String> {
    let mut stores = Vec::new();
    for kind in std::fs::read_dir(root).unwrap().flatten() {
        for store in std::fs::read_dir(kind.path()).unwrap().flatten() {
            stores.push(format!(
                "{}/{}",
                kind.file_name().to_string_lossy(),
                store.file_name().to_string_lossy()
            ));
        }
    }
    stores.sort();
    stores
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

/// A kernel answers a page load of its site: with its page, where its routes
/// keep nothing, which renders the site's view; with an asset, opened as a
/// page; and with a page saying what it answered, for an address it cannot
/// answer with either. A request that is not a page load is answered as
/// before.
#[tokio::test(flavor = "current_thread")]
async fn it_answers_page_loads_with_its_page() {
    let engine = engine();
    let mut kernel = Kernel::load(
        &engine,
        tonk_components::TONK_KERNEL_BYTES,
        scratch("page-loads"),
    )
    .await
    .unwrap();
    kernel
        .init(
            &space(),
            Standing::handed([7; 32], Some(space_to_session().await)),
        )
        .await
        .unwrap();
    let (served, driver) = kernel.serve();
    let page = vec![("accept".to_owned(), b"text/html,*/*;q=0.8".to_vec())];

    let asked = async {
        let load = |path: &'static str, headers: Vec<(String, Vec<u8>)>| {
            let served = &served;
            async move {
                let answer = served.request("GET", path, headers, b"").await.unwrap();
                let status = answer.status;
                let kind = answer
                    .headers
                    .iter()
                    .find(|(name, _)| name == "content-type")
                    .map(|(_, value)| String::from_utf8_lossy(value).into_owned());
                let body: Vec<Vec<u8>> = answer.body.collect().await;
                (status, kind, String::from_utf8(body.concat()).unwrap())
            }
        };

        let (status, kind, body) = load("/notes", page.clone()).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(kind.as_deref(), Some("text/html; charset=utf-8"));
        assert!(
            body.contains("tonk-display"),
            "the page renders the view: {body}"
        );

        let stored = served
            .request(
                "PUT",
                "/",
                vec![("content-type".to_owned(), b"text/plain".to_vec())],
                b"an asset",
            )
            .await
            .unwrap();
        assert_eq!(stored.status, 201);
        let at = stored
            .headers
            .iter()
            .find(|(name, _)| name == "location")
            .map(|(_, value)| String::from_utf8_lossy(value).into_owned())
            .expect("the asset's address");
        let opened = served.request("GET", &at, page.clone(), b"").await.unwrap();
        assert_eq!(opened.status, 200, "an asset opened as a page is the asset");
        let body: Vec<Vec<u8>> = opened.body.collect().await;
        assert_eq!(body.concat(), b"an asset");

        let (status, kind, body) = load("/asset:3mJr7AoUXx2Wqd", page.clone()).await;
        assert_eq!(status, 400, "{body}");
        assert_eq!(kind.as_deref(), Some("text/html; charset=utf-8"));
        assert!(
            body.contains("This space answered 400."),
            "a page says what it answered: {body}"
        );

        let (status, _, body) = load("/notes", Vec::new()).await;
        assert_eq!(status, 404, "a request for content finds none: {body}");
    };
    let asked = tokio::time::timeout(std::time::Duration::from_secs(60), asked);
    tokio::select! {
        answered = asked => answered.expect("the kernel answered within a minute"),
        stopped = driver => panic!("the kernel stopped while asked: {stopped:?}"),
    }
}

/// A persona's provisional persona merges into the account the person signs
/// in to: the account delegates to the representative, which links this
/// device to it, and the persona records the account as its root, from what
/// its host says the passkey ceremony made of it. The persona acts for the
/// account from then on.
#[tokio::test(flavor = "current_thread")]
async fn it_merges_into_the_account_the_person_signs_in_to() {
    use futures::StreamExt as _;
    use serde_json::{Value, json};
    use tonk_kernel_host::credentials::Key;
    use zeroize::Zeroizing;

    let engine = engine();
    let session = did_key(&ed25519_dalek::SigningKey::from_bytes(&[7; 32]));
    let account = did_key(&ed25519_dalek::SigningKey::from_bytes(&[9; 32]));
    let mut kernel = Kernel::load(
        &engine,
        tonk_components::TONK_PERSONA_BYTES,
        scratch("merge"),
    )
    .await
    .unwrap();
    let mut messages = kernel.messages();
    kernel.init(&session, persona_standing()).await.unwrap();
    let (served, driver) = kernel.serve();

    // The host answers what the persona asks, as a browser's host would for
    // a log-in: no service answers it here.
    let answering = async {
        while let Some(message) = messages.next().await {
            let message: Value = serde_json::from_slice(&message).unwrap();
            let Some(id) = message["ask"].as_u64() else {
                continue;
            };
            let answer = if message.get("ceremony").is_some() {
                let ceremony = json!({
                    "kind": "log-in",
                    "credentialId": "0a0b",
                    "remote": "https://tonk.test/ucan/",
                });
                json!({ "answer": id, "status": 200, "body": ceremony.to_string() })
            } else if message.get("fetch").is_some() {
                json!({ "answer": id, "status": 404, "body64": "" })
            } else {
                json!({ "answer": id, "status": 404, "body": "" })
            };
            let _ = served.receive(answer.to_string().into_bytes()).await;
        }
    };
    let asked = async {
        let key = |seed: [u8; 32]| Key::Seed {
            seed: Zeroizing::new(seed),
            imported: false,
        };
        let (persona, link) = served.merge(key([7; 32]), key([9; 32])).await.unwrap();
        assert_eq!(persona, account, "the persona is the account's");
        let link = dialog_ucan_core::DelegationChain::try_from(link.as_slice()).unwrap();
        assert_eq!(link.issuer().to_string(), account);
        assert_eq!(
            link.audience().to_string(),
            session,
            "the account links the device"
        );

        let root = served
            .request("GET", "/api/identity/root", Vec::new(), b"")
            .await
            .unwrap();
        let body: Vec<Vec<u8>> = root.body.collect().await;
        let root: Value = serde_json::from_slice(&body.concat()).unwrap();
        assert_eq!(
            root["rootDid"], account,
            "the persona records the account: {root}"
        );
    };
    let asked = tokio::time::timeout(std::time::Duration::from_secs(60), asked);
    tokio::select! {
        answered = asked => answered.expect("the merge finished within a minute"),
        () = answering => panic!("the persona stopped sending"),
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

/// A persona creates a space in its profile: its key one the host makes,
/// whose seed the persona never knows.
#[tokio::test(flavor = "current_thread")]
async fn it_creates_a_space_with_a_key_the_host_makes() {
    let engine = engine();
    let (mut kernel, space) = persona_with_space(&engine, scratch("profile")).await;
    let keys = kernel.keys();
    assert!(
        keys.contains(&(space.clone(), false)),
        "the space's key is one the host made: {keys:?}"
    );
    assert!(
        keys.iter().all(|(_, imported)| !imported),
        "the persona imports no seed it knows: {keys:?}"
    );
}

/// A persona delegates another worker's session the space it asks for, and
/// the space's own worker takes the space up with what the persona
/// delegated.
#[tokio::test(flavor = "current_thread")]
async fn it_takes_up_a_space_its_persona_delegates() {
    use tonk_kernel_host::bindings::sapling::kernel::credential::Access;

    let engine = engine();
    let (mut persona, space) = persona_with_space(&engine, scratch("granting")).await;
    let worker = did_key(&ed25519_dalek::SigningKey::from_bytes(&[8; 32]));
    let access = Access {
        subject: Some(space.clone()),
        command: "/".to_owned(),
        arguments: Vec::new(),
        reason: "to serve the space".to_owned(),
    };
    let delegated = persona
        .delegate(&worker, access)
        .await
        .unwrap()
        .expect("the persona delegates the space it created");

    let mut kernel = Kernel::load(
        &engine,
        tonk_components::TONK_KERNEL_BYTES,
        scratch("granted"),
    )
    .await
    .unwrap();
    kernel
        .init(&space, Standing::handed([8; 32], Some(delegated)))
        .await
        .unwrap();
    let path = format!("/api/repository/{space}");
    let answer = kernel.request("GET", &path, b"").await.unwrap();
    assert_eq!(
        answer.status,
        200,
        "the worker holds its space: {}",
        answer.text()
    );
}

/// A persona, booted over `layers` with the session of the seed `[7; 32]`,
/// that has created a space, as its profile's page creates one; and the
/// space's DID.
async fn persona_with_space(engine: &Engine, layers: Layers) -> (Kernel, String) {
    let session = did_key(&ed25519_dalek::SigningKey::from_bytes(&[7; 32]));
    let mut kernel = Kernel::load(engine, tonk_components::TONK_PERSONA_BYTES, layers)
        .await
        .unwrap();
    kernel.init(&session, persona_standing()).await.unwrap();
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
    (kernel, space)
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
            Standing::handed([7; 32], Some(space_to_session().await)),
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
