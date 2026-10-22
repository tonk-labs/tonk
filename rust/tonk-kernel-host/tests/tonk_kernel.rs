//! The Tonk kernel, natively on the polyfill: Tonk's worker inside a component,
//! on the storage and credentials of its host.
#![cfg(not(target_arch = "wasm32"))]

use futures::StreamExt as _;
use tonk_kernel_host::kernel::Kernel;
use tonk_kernel_host::sapling::{Change, did_key};
use wcmp::Engine;

fn engine() -> Engine {
    Engine::with_backend(wcmp_wasm_core_wasmtime::Wasmtime::new().unwrap()).unwrap()
}

fn space() -> String {
    did_key(&ed25519_dalek::SigningKey::from_bytes(&[3; 32]))
}

/// Boot a kernel over what an earlier one kept, and answer who it is.
async fn identify(engine: &Engine, bytes: &[u8], kept: Vec<Change>) -> (String, Vec<Change>) {
    let mut kernel = Kernel::load_restoring(engine, bytes, [7; 32], kept)
        .await
        .unwrap();
    kernel.init(&space(), &space()).await.unwrap();
    let answer = kernel.request("GET", "/api/identify", b"").await.unwrap();
    assert_eq!(answer.status, 200, "{}", answer.text());
    (answer.text(), kernel.drain())
}

/// The kernel instantiates against the world alone, boots Tonk's worker, and
/// answers its API.
#[tokio::test(flavor = "current_thread")]
async fn it_answers_with_tonk_s_worker() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;
    let mut kernel = Kernel::load(&engine, bytes, [7; 32]).await.unwrap();
    kernel.init(&space(), &space()).await.unwrap();

    for path in ["/api/identify", "/api/profile"] {
        let answer = kernel.request("GET", path, b"").await.unwrap();
        assert_eq!(answer.status, 200, "{path}: {}", answer.text());
        println!("{path} -> {}", answer.text());
    }
    println!("timings: {:?}", kernel.timings);
}

/// The worker's keys are made and kept by the host: the kernel's writes carry
/// each key's seed to the host's store, and a kernel that starts again over
/// them is the same profile.
#[tokio::test(flavor = "current_thread")]
async fn it_keeps_its_keys_in_the_host() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;

    let (first, kept) = identify(&engine, bytes, Vec::new()).await;
    let keys = kept
        .iter()
        .filter(|change| matches!(change, Change::Key { seed: Some(_), .. }))
        .count();
    assert!(keys >= 1, "the host keeps the worker's keys: {kept:?}");

    let (again, _) = identify(&engine, bytes, kept).await;
    assert_eq!(
        first, again,
        "a kernel that starts again is the same profile"
    );
}

/// A served kernel answers each request while another's answer is still
/// being written: a subscription stays open, and a request made after it is
/// answered all the same.
#[tokio::test(flavor = "current_thread")]
async fn it_answers_while_a_subscription_is_open() {
    let engine = engine();
    let bytes = tonk_components::TONK_KERNEL_BYTES;
    let mut kernel = Kernel::load(&engine, bytes, [7; 32]).await.unwrap();
    kernel.init(&space(), &space()).await.unwrap();
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

/// The build names the kernel as a space's boot cell pins it.
#[test]
fn it_names_the_kernel_the_way_a_space_pins_it() {
    assert_eq!(
        tonk_kernel_host::boot::kernel_id(tonk_components::TONK_KERNEL_BYTES),
        tonk_components::TONK_KERNEL_ID
    );
}
