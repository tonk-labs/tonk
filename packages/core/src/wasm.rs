use crate::bundle::{Bundle, BundlePath};
use crate::tonk_core::TonkCore;
use crate::vfs::NodeType;
use crate::StorageConfig;
use automerge::AutoSerde;
use js_sys::{Array, Function, Promise, Uint8Array};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::Mutex;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};

#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(s: &str);

    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_error {
    ($($t:tt)*) => (error(&format_args!($($t)*).to_string()))
}

#[wasm_bindgen(start)]
pub fn init() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

fn js_error(err: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&err.to_string())
}

#[wasm_bindgen]
pub struct WasmTonkCore {
    tonk: Arc<Mutex<TonkCore>>,
}

#[wasm_bindgen]
impl WasmTonkCore {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Promise {
        future_to_promise(async move {
            match TonkCore::new().await {
                Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                    tonk: Arc::new(Mutex::new(tonk)),
                })),
                Err(e) => {
                    console_error!("TonkCore creation failed: {}", e);
                    Err(js_error(e))
                }
            }
        })
    }

    #[wasm_bindgen(js_name = withPeerId)]
    pub fn with_peer_id(peer_id: String) -> Promise {
        future_to_promise(async move {
            let peer_id = samod::PeerId::from_string(peer_id);
            match TonkCore::with_peer_id(peer_id).await {
                Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                    tonk: Arc::new(Mutex::new(tonk)),
                })),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = getPeerId)]
    pub fn get_peer_id(&self) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            Ok(JsValue::from_str(&tonk.peer_id().to_string()))
        })
    }

    #[wasm_bindgen(js_name = connectWebsocket)]
    pub fn connect_websocket(&self, url: String) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            match tonk.connect_websocket(&url).await {
                Ok(_) => Ok(JsValue::undefined()),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(data: Uint8Array) -> Promise {
        future_to_promise(async move {
            let bytes = data.to_vec();
            match TonkCore::from_bytes(bytes).await {
                Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                    tonk: Arc::new(Mutex::new(tonk)),
                })),
                Err(e) => {
                    console_error!("Failed to load TonkCore from bytes: {}", e);
                    Err(js_error(e))
                }
            }
        })
    }

    #[wasm_bindgen(js_name = fromBundle)]
    pub fn from_bundle(bundle: &WasmBundle) -> Promise {
        // Get the bundle bytes from WasmBundle
        let bundle_to_bytes_promise = bundle.to_bytes();
        future_to_promise(async move {
            // Get the bytes from the bundle
            let bytes_result = JsFuture::from(bundle_to_bytes_promise).await;
            match bytes_result {
                Ok(bytes_value) => {
                    let bytes_array: Uint8Array = bytes_value.into();
                    let bytes = bytes_array.to_vec();
                    // Load TonkCore from the bytes
                    match TonkCore::from_bytes(bytes).await {
                        Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                            tonk: Arc::new(Mutex::new(tonk)),
                        })),
                        Err(e) => {
                            console_error!("Failed to load TonkCore from bundle: {}", e);
                            Err(js_error(e))
                        }
                    }
                }
                Err(e) => {
                    console_error!("Failed to get bundle bytes: {:?}", e);
                    Err(js_error("Failed to get bundle bytes"))
                }
            }
        })
    }

    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            match tonk.to_bytes().await {
                Ok(bytes) => {
                    let array = Uint8Array::new_with_length(bytes.len() as u32);
                    array.copy_from(&bytes);
                    Ok(JsValue::from(array))
                }
                Err(e) => {
                    console_error!("Failed to export TonkCore to bytes: {}", e);
                    Err(js_error(e))
                }
            }
        })
    }

    #[wasm_bindgen(js_name = createFile)]
    pub fn create_file(&self, path: String, content: JsValue) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            // Deserialize JsValue to serde_json::Value
            let content_value: serde_json::Value = serde_wasm_bindgen::from_value(content)
                .map_err(|e| js_error(format!("Invalid content value: {}", e)))?;

            match vfs.create_document(&path, content_value).await {
                Ok(_) => Ok(JsValue::TRUE),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = readFile)]
    pub fn read_file(&self, path: String) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.find_document(&path).await {
                Ok(Some(handle)) => {
                    let doc = handle.with_document(|doc| {
                        let auto_serde = AutoSerde::from(&*doc);
                        serde_json::to_value(&auto_serde)
                    });

                    match doc {
                        Ok(json_value) => serde_wasm_bindgen::to_value(&json_value)
                            .map_err(|e| js_error(format!("Failed to convert to JsValue: {}", e))),
                        Err(e) => Err(js_error(e)),
                    }
                }
                Ok(None) => Ok(JsValue::NULL),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = updateFile)]
    pub fn update_file(&self, path: String, content: JsValue) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            // Deserialize JsValue to serde_json::Value
            let content_value: serde_json::Value = serde_wasm_bindgen::from_value(content)
                .map_err(|e| js_error(format!("Invalid content value: {}", e)))?;

            match vfs.update_document(&path, content_value).await {
                Ok(updated) => Ok(JsValue::from_bool(updated)),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = deleteFile)]
    pub fn delete_file(&self, path: String) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.remove_document(&path).await {
                Ok(removed) => Ok(JsValue::from_bool(removed)),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = createDirectory)]
    pub fn create_directory(&self, path: String) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.create_directory(&path).await {
                Ok(_) => Ok(JsValue::TRUE),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = listDirectory)]
    pub fn list_directory(&self, path: String) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.list_directory(&path).await {
                Ok(nodes) => {
                    let array = Array::new();
                    for node in nodes.iter() {
                        let obj = js_sys::Object::new();
                        match node.node_type {
                            NodeType::Directory => {
                                js_sys::Reflect::set(&obj, &"type".into(), &"directory".into())
                                    .unwrap();
                            }
                            NodeType::Document => {
                                js_sys::Reflect::set(&obj, &"type".into(), &"document".into())
                                    .unwrap();
                            }
                        }

                        js_sys::Reflect::set(&obj, &"name".into(), &node.name.clone().into())
                            .unwrap();

                        array.push(&obj);
                    }
                    Ok(JsValue::from(array))
                }
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = exists)]
    pub fn exists(&self, path: String) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.exists(&path).await {
                Ok(exists) => Ok(JsValue::from_bool(exists)),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = getMetadata)]
    pub fn get_metadata(&self, path: String) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.metadata(&path).await {
                Ok(Some((node_type, timestamps))) => {
                    let metadata = WasmNodeMetadata {
                        node_type: match node_type {
                            NodeType::Directory => "directory".to_string(),
                            NodeType::Document => "document".to_string(),
                        },
                        created_at: timestamps.created.timestamp(),
                        modified_at: timestamps.modified.timestamp(),
                    };
                    Ok(serde_wasm_bindgen::to_value(&metadata).unwrap())
                }
                Ok(None) => Ok(JsValue::NULL),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = watchDocument)]
    pub fn watch_document(&self, path: String, callback: Function) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.watch_document(&path).await {
                Ok(Some(watcher)) => {
                    // Get the document ID before moving the watcher
                    let document_id = watcher.document_id().to_string();

                    // Create abort handle for the watcher task
                    let (abort_handle, abort_registration) =
                        futures::future::AbortHandle::new_pair();

                    // Create a channel for communication between the watcher and callback
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                    // Spawn a task to receive document updates and call the JS callback
                    spawn_local(async move {
                        while let Some(json_str) = rx.recv().await {
                            let js_value = JsValue::from_str(&json_str);
                            let _ = callback.call1(&JsValue::null(), &js_value);
                        }
                    });

                    // Spawn the watcher task
                    spawn_local(async move {
                        let abortable = futures::future::Abortable::new(
                            watcher.on_change(move |doc| {
                                // Convert document to JSON
                                let auto_serde = AutoSerde::from(&*doc);
                                if let Ok(json_str) = serde_json::to_string(&auto_serde) {
                                    // Send the JSON through the channel
                                    let _ = tx.send(json_str);
                                }
                            }),
                            abort_registration,
                        );
                        let _ = abortable.await;
                    });

                    Ok(JsValue::from(WasmDocumentWatcher {
                        document_id,
                        abort_handle: Arc::new(Mutex::new(Some(abort_handle))),
                    }))
                }
                Ok(None) => Err(js_error("Document not found at the specified path")),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = watchDirectory)]
    pub fn watch_directory(&self, path: String, callback: Function) -> Promise {
        let tonk = Arc::clone(&self.tonk);
        future_to_promise(async move {
            let tonk = tonk.lock().await;
            let vfs = tonk.vfs();

            match vfs.watch_directory(&path).await {
                Ok(Some(watcher)) => {
                    // Get the document ID before moving the watcher
                    let document_id = watcher.document_id().to_string();

                    // Create abort handle for the watcher task
                    let (abort_handle, abort_registration) =
                        futures::future::AbortHandle::new_pair();

                    // Create a channel for communication between the watcher and callback
                    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

                    // Move the callback into the spawned task

                    // Spawn a task to receive document updates and call the JS callback
                    spawn_local(async move {
                        while let Some(json_str) = rx.recv().await {
                            let js_value = JsValue::from_str(&json_str);
                            let _ = callback.call1(&JsValue::null(), &js_value);
                        }
                    });

                    // Spawn the watcher task
                    spawn_local(async move {
                        let abortable = futures::future::Abortable::new(
                            watcher.on_change(move |doc| {
                                // Convert document to JSON
                                let auto_serde = AutoSerde::from(&*doc);
                                if let Ok(json_str) = serde_json::to_string(&auto_serde) {
                                    // Send the JSON through the channel
                                    let _ = tx.send(json_str);
                                }
                            }),
                            abort_registration,
                        );
                        let _ = abortable.await;
                    });

                    Ok(JsValue::from(WasmDocumentWatcher {
                        document_id,
                        abort_handle: Arc::new(Mutex::new(Some(abort_handle))),
                    }))
                }
                Ok(None) => Err(js_error("Directory not found at the specified path")),
                Err(e) => Err(js_error(e)),
            }
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct WasmNodeMetadata {
    pub node_type: String,
    pub created_at: i64,
    pub modified_at: i64,
}

#[wasm_bindgen]
pub struct WasmBundle {
    bundle: Arc<Mutex<Bundle<Cursor<Vec<u8>>>>>,
}

#[wasm_bindgen]
impl WasmBundle {
    #[wasm_bindgen(js_name = fromBytes)]
    pub fn from_bytes(data: Uint8Array) -> std::result::Result<WasmBundle, JsValue> {
        let bytes = data.to_vec();
        match Bundle::from_bytes(bytes) {
            Ok(bundle) => Ok(WasmBundle {
                bundle: Arc::new(Mutex::new(bundle)),
            }),
            Err(e) => Err(js_error(e)),
        }
    }

    #[wasm_bindgen(js_name = getRootId)]
    pub fn get_root_id(&self) -> Promise {
        let bundle = Arc::clone(&self.bundle);
        future_to_promise(async move {
            let bundle = bundle.lock().await;
            match bundle.root_id() {
                Ok(root_id) => Ok(JsValue::from_str(&root_id)),
                Err(e) => Err(js_error(format!("Error retrieving bundle root ID: {}", e))),
            }
        })
    }

    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, key: String) -> Promise {
        let bundle = Arc::clone(&self.bundle);
        future_to_promise(async move {
            let mut bundle = bundle.lock().await;
            let path = BundlePath::from(&key);

            match bundle.get(&path) {
                Ok(Some(data)) => {
                    let array = Uint8Array::new_with_length(data.len() as u32);
                    array.copy_from(&data);
                    Ok(JsValue::from(array))
                }
                Ok(None) => Ok(JsValue::NULL),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = listKeys)]
    pub fn list_keys(&self) -> Promise {
        let bundle = Arc::clone(&self.bundle);
        future_to_promise(async move {
            let bundle = bundle.lock().await;
            let keys = bundle.list_keys();

            let array = Array::new();
            for key in keys {
                array.push(&JsValue::from_str(&key.to_string()));
            }
            Ok(JsValue::from(array))
        })
    }

    #[wasm_bindgen(js_name = getPrefix)]
    pub fn get_prefix(&self, prefix: String) -> Promise {
        let bundle = Arc::clone(&self.bundle);
        future_to_promise(async move {
            let mut bundle = bundle.lock().await;
            let prefix_path = BundlePath::from(&prefix);

            match bundle.prefix(&prefix_path) {
                Ok(entries) => {
                    let array = Array::new();
                    for (key, value) in entries {
                        let obj = js_sys::Object::new();
                        js_sys::Reflect::set(&obj, &"key".into(), &key.to_string().into()).unwrap();

                        let data_array = Uint8Array::new_with_length(value.len() as u32);
                        data_array.copy_from(&value);
                        js_sys::Reflect::set(&obj, &"value".into(), &data_array.into()).unwrap();

                        array.push(&obj);
                    }
                    Ok(JsValue::from(array))
                }
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = getManifest)]
    pub fn get_manifest(&self) -> Promise {
        let bundle = Arc::clone(&self.bundle);
        future_to_promise(async move {
            let bundle = bundle.lock().await;
            let manifest = bundle.manifest();
            match serde_wasm_bindgen::to_value(&manifest) {
                Ok(value) => Ok(value),
                Err(e) => Err(js_error(format!("Failed to serialize manifest: {}", e))),
            }
        })
    }

    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Promise {
        let bundle = Arc::clone(&self.bundle);
        future_to_promise(async move {
            let mut bundle = bundle.lock().await;
            match bundle.to_bytes() {
                Ok(bytes) => {
                    let array = Uint8Array::new_with_length(bytes.len() as u32);
                    array.copy_from(&bytes);
                    Ok(JsValue::from(array))
                }
                Err(e) => Err(js_error(e)),
            }
        })
    }
}

#[derive(Clone)]
pub struct WasmVfsEvent {
    pub event_type: String,
    pub path: String,
}

#[wasm_bindgen]
pub struct WasmDocumentWatcher {
    document_id: String,
    abort_handle: Arc<Mutex<Option<futures::future::AbortHandle>>>,
}

#[wasm_bindgen]
impl WasmDocumentWatcher {
    #[wasm_bindgen(js_name = stop)]
    pub fn stop(&self) -> Promise {
        let abort_handle = Arc::clone(&self.abort_handle);
        future_to_promise(async move {
            // Abort the watcher task
            if let Some(handle) = abort_handle.lock().await.take() {
                handle.abort();
            }

            Ok(JsValue::undefined())
        })
    }

    #[wasm_bindgen(js_name = documentId)]
    pub fn document_id(&self) -> String {
        self.document_id.clone()
    }
}

#[wasm_bindgen]
pub fn create_tonk() -> Promise {
    WasmTonkCore::new()
}

#[wasm_bindgen]
pub fn create_tonk_with_peer_id(peer_id: String) -> Promise {
    WasmTonkCore::with_peer_id(peer_id)
}

#[wasm_bindgen]
pub fn create_tonk_with_storage(use_indexed_db: bool) -> Promise {
    future_to_promise(async move {
        let storage_config = if use_indexed_db {
            StorageConfig::IndexedDB
        } else {
            StorageConfig::InMemory
        };

        match TonkCore::builder()
            .with_storage(storage_config)
            .build()
            .await
        {
            Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                tonk: Arc::new(Mutex::new(tonk)),
            })),
            Err(e) => {
                console_error!("TonkCore creation failed: {}", e);
                Err(js_error(e))
            }
        }
    })
}

#[wasm_bindgen]
pub fn create_tonk_with_config(peer_id: String, use_indexed_db: bool) -> Promise {
    future_to_promise(async move {
        let peer_id = samod::PeerId::from_string(peer_id);
        let storage_config = if use_indexed_db {
            StorageConfig::IndexedDB
        } else {
            StorageConfig::InMemory
        };

        match TonkCore::builder()
            .with_peer_id(peer_id)
            .with_storage(storage_config)
            .build()
            .await
        {
            Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                tonk: Arc::new(Mutex::new(tonk)),
            })),
            Err(e) => {
                console_error!("TonkCore creation failed: {}", e);
                Err(js_error(e))
            }
        }
    })
}

#[wasm_bindgen]
pub fn create_bundle_from_bytes(data: Uint8Array) -> std::result::Result<WasmBundle, JsValue> {
    WasmBundle::from_bytes(data)
}

#[wasm_bindgen]
pub fn create_tonk_from_bundle(bundle: &WasmBundle) -> Promise {
    WasmTonkCore::from_bundle(bundle)
}

#[wasm_bindgen]
pub fn create_tonk_from_bundle_with_storage(bundle: &WasmBundle, use_indexed_db: bool) -> Promise {
    let bundle_to_bytes_promise = bundle.to_bytes();
    future_to_promise(async move {
        let bytes_result = JsFuture::from(bundle_to_bytes_promise).await;
        match bytes_result {
            Ok(bytes_value) => {
                let bytes_array: Uint8Array = bytes_value.into();
                let bytes = bytes_array.to_vec();

                let storage_config = if use_indexed_db {
                    StorageConfig::IndexedDB
                } else {
                    StorageConfig::InMemory
                };

                match TonkCore::builder()
                    .with_storage(storage_config)
                    .from_bytes(bytes)
                    .await
                {
                    Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                        tonk: Arc::new(Mutex::new(tonk)),
                    })),
                    Err(e) => {
                        console_error!("Failed to load TonkCore from bundle with storage: {}", e);
                        Err(js_error(e))
                    }
                }
            }
            Err(e) => {
                console_error!("Failed to get bundle bytes: {:?}", e);
                Err(js_error("Failed to get bundle bytes"))
            }
        }
    })
}

#[wasm_bindgen]
pub fn create_tonk_from_bytes(data: Uint8Array) -> Promise {
    WasmTonkCore::from_bytes(data)
}

#[wasm_bindgen]
pub fn create_tonk_from_bytes_with_storage(data: Uint8Array, use_indexed_db: bool) -> Promise {
    future_to_promise(async move {
        let bytes = data.to_vec();

        let storage_config = if use_indexed_db {
            StorageConfig::IndexedDB
        } else {
            StorageConfig::InMemory
        };

        match TonkCore::builder()
            .with_storage(storage_config)
            .from_bytes(bytes)
            .await
        {
            Ok(tonk) => Ok(JsValue::from(WasmTonkCore {
                tonk: Arc::new(Mutex::new(tonk)),
            })),
            Err(e) => {
                console_error!("Failed to load TonkCore from bytes with storage: {}", e);
                Err(js_error(e))
            }
        }
    })
}
