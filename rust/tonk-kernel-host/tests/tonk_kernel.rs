//! The Tonk kernel, natively on the polyfill: Tonk's worker inside a component,
//! on the storage and credentials of its host.
#![cfg(not(target_arch = "wasm32"))]

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

/// The build names the kernel as a space's boot cell pins it.
#[test]
fn it_names_the_kernel_the_way_a_space_pins_it() {
    assert_eq!(
        tonk_kernel_host::boot::kernel_id(tonk_components::TONK_KERNEL_BYTES),
        tonk_components::TONK_KERNEL_ID
    );
}
