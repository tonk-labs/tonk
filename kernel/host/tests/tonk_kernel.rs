//! The Tonk kernel, natively on the polyfill.
#![cfg(not(target_arch = "wasm32"))]

use kernel_host::kernel::Kernel;
use wcmp::Engine;

const TONK_KERNEL: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../target/wasm32-wasip3/release/tonk_kernel.wasm"
);

#[tokio::test(flavor = "current_thread")]
async fn it_boots_the_tonk_kernel() {
    let engine = Engine::with_backend(wcmp_wasm_core_wasmtime::Wasmtime::new().unwrap()).unwrap();
    let mut kernel = Kernel::load(&engine, &std::fs::read(TONK_KERNEL).unwrap())
        .await
        .unwrap();
    let space = kernel_host::sapling::did_key(&ed25519_dalek::SigningKey::from_bytes(&[3; 32]));
    let persona = kernel_host::sapling::did_key(&ed25519_dalek::SigningKey::from_bytes(&[2; 32]));
    println!("init -> {:?}", kernel.init(&persona, &space).await);
    for (method, path, body) in [
        ("GET", "/api/health", String::new()),
        (
            "POST",
            "/api/transact",
            r#"[{"the":"xyz.tonk.note/title","of":"note:1","is":"hello"}]"#.to_owned(),
        ),
        (
            "POST",
            "/api/select",
            r#"{"the":"xyz.tonk.note/title"}"#.to_owned(),
        ),
    ] {
        let answer = kernel.request(method, path, body.as_bytes()).await;
        println!(
            "{method} {path} -> {:?}",
            answer.map(|answer| (answer.status, answer.text()))
        );
    }
    println!("timings: {:?}", kernel.timings);
}

/// A second kernel, over what the first one wrote, reads every fact the
/// first committed: what a worker restart must do.
#[tokio::test(flavor = "current_thread")]
async fn it_reads_what_an_earlier_kernel_committed() {
    let engine = Engine::with_backend(wcmp_wasm_core_wasmtime::Wasmtime::new().unwrap()).unwrap();
    let bytes = std::fs::read(TONK_KERNEL).unwrap();
    let space = kernel_host::sapling::did_key(&ed25519_dalek::SigningKey::from_bytes(&[3; 32]));
    let persona = kernel_host::sapling::did_key(&ed25519_dalek::SigningKey::from_bytes(&[2; 32]));
    let mut kept = Vec::new();
    for round in 0..3 {
        let mut kernel = Kernel::load_restoring(&engine, &bytes, kept.clone())
            .await
            .unwrap();
        kernel.init(&persona, &space).await.unwrap();
        let claim =
            format!(r#"[{{"the":"xyz.tonk.note/title","of":"note:{round}","is":"hello"}}]"#);
        let committed = kernel
            .request("POST", "/api/transact", claim.as_bytes())
            .await
            .unwrap();
        let read = kernel
            .request("POST", "/api/select", br#"{"the":"xyz.tonk.note/title"}"#)
            .await
            .unwrap();
        let drained = kernel.drain();
        let kinds: Vec<_> = drained
            .iter()
            .map(|change| match change {
                kernel_host::sapling::Change::Block { .. } => "block",
                kernel_host::sapling::Change::Cell {
                    namespace, cell, ..
                } => {
                    if namespace.is_empty() {
                        "cell"
                    } else {
                        Box::leak(format!("cell {namespace}/{cell}").into_boxed_str())
                    }
                }
                kernel_host::sapling::Change::Blob { .. } => "blob",
            })
            .collect();
        println!(
            "round {round}: restored {} -> commit {} -> {} facts; wrote {kinds:?}",
            kept.len(),
            committed.status,
            read.text().matches("note:").count()
        );
        kept.extend(drained);
    }
}
