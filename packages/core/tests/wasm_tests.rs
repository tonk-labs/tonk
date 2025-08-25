#![cfg(target_arch = "wasm32")]

use tonk_core::wasm::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn test_create_sync_engine() {
    let engine = create_sync_engine().await;
    assert!(engine.is_ok());
}

#[wasm_bindgen_test]
async fn test_create_sync_engine_with_peer_id() {
    let peer_id = "test-peer-id-123";
    let engine = create_sync_engine_with_peer_id(peer_id.to_string()).await;
    assert!(engine.is_ok());
}

// Note: More comprehensive tests would require mocking the async runtime
// and file system operations in WASM environment
