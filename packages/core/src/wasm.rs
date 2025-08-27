use crate::bundle::{Bundle, BundlePath};
use crate::sync::SyncEngine;
use crate::vfs::{NodeType, VirtualFileSystem};
use automerge::AutoSerde;
use js_sys::{Array, Promise, Uint8Array};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::Mutex;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

    #[wasm_bindgen(js_namespace = console)]
    fn error(s: &str);
}

macro_rules! console_error {
    ($($t:tt)*) => (error(&format_args!($($t)*).to_string()))
}

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

fn js_error(err: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&err.to_string())
}

#[wasm_bindgen]
pub struct WasmSyncEngine {
    engine: Arc<Mutex<SyncEngine>>,
}

#[wasm_bindgen]
impl WasmSyncEngine {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> Promise {
        future_to_promise(async move {
            match SyncEngine::new().await {
                Ok(engine) => Ok(JsValue::from(WasmSyncEngine {
                    engine: Arc::new(Mutex::new(engine)),
                })),
                Err(e) => {
                    console_error!("SyncEngine creation failed: {}", e);
                    Err(js_error(e))
                }
            }
        })
    }

    #[wasm_bindgen(js_name = withPeerId)]
    pub fn with_peer_id(peer_id: String) -> Promise {
        future_to_promise(async move {
            let peer_id = samod::PeerId::from_string(peer_id);
            match SyncEngine::with_peer_id(peer_id).await {
                Ok(engine) => Ok(JsValue::from(WasmSyncEngine {
                    engine: Arc::new(Mutex::new(engine)),
                })),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = getPeerId)]
    pub fn get_peer_id(&self) -> Promise {
        let engine = self.engine.clone();
        future_to_promise(async move {
            let engine = engine.lock().await;
            Ok(JsValue::from_str(&engine.peer_id().to_string()))
        })
    }

    #[wasm_bindgen(js_name = connectWebsocket)]
    pub fn connect_websocket(&self, url: String) -> Promise {
        let engine = self.engine.clone();
        future_to_promise(async move {
            let engine = engine.lock().await;
            match engine.connect_websocket(&url).await {
                Ok(_) => Ok(JsValue::undefined()),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = getVfs)]
    pub fn get_vfs(&self) -> Promise {
        let engine = self.engine.clone();
        future_to_promise(async move {
            let engine = engine.lock().await;
            let vfs = engine.vfs();
            Ok(JsValue::from(WasmVfs {
                vfs: Arc::new(Mutex::new(vfs)),
            }))
        })
    }
}

#[wasm_bindgen]
pub struct WasmVfs {
    vfs: Arc<Mutex<Arc<VirtualFileSystem>>>,
}

#[derive(Serialize, Deserialize)]
pub struct WasmNodeMetadata {
    pub node_type: String,
    pub created_at: i64,
    pub modified_at: i64,
}

#[wasm_bindgen]
impl WasmVfs {
    #[wasm_bindgen(js_name = createFile)]
    pub fn create_file(&self, path: String, content: JsValue) -> Promise {
        let vfs = self.vfs.clone();
        future_to_promise(async move {
            let vfs = vfs.lock().await;
            let content_str = content.as_string().unwrap_or_default();

            match vfs.create_document(&path, content_str).await {
                Ok(_handle) => Ok(JsValue::TRUE),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = readFile)]
    pub fn read_file(&self, path: String) -> Promise {
        let vfs = self.vfs.clone();
        future_to_promise(async move {
            let vfs = vfs.lock().await;

            match vfs.find_document(&path).await {
                Ok(Some(handle)) => {
                    handle.with_document(|doc| {
                        let auto_serde = AutoSerde::from(&*doc);
                        serde_json::to_string_pretty(&auto_serde)
                            .unwrap_or_else(|e| format!("Error serializing: {e}"))
                    });
                    Ok(JsValue::from_str(""))
                }
                Ok(None) => Ok(JsValue::NULL),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = deleteFile)]
    pub fn delete_file(&self, path: String) -> Promise {
        let vfs = self.vfs.clone();
        future_to_promise(async move {
            let vfs = vfs.lock().await;

            match vfs.remove_document(&path).await {
                Ok(removed) => Ok(JsValue::from_bool(removed)),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = createDirectory)]
    pub fn create_directory(&self, path: String) -> Promise {
        let vfs = self.vfs.clone();
        future_to_promise(async move {
            let vfs = vfs.lock().await;

            match vfs.create_directory(&path).await {
                Ok(_) => Ok(JsValue::TRUE),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = listDirectory)]
    pub fn list_directory(&self, path: String) -> Promise {
        let vfs = self.vfs.clone();
        future_to_promise(async move {
            let vfs = vfs.lock().await;

            match vfs.list_directory(&path).await {
                Ok(nodes) => {
                    let array = Array::new();
                    for node in nodes {
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

                        js_sys::Reflect::set(&obj, &"name".into(), &node.name.into()).unwrap();

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
        let vfs = self.vfs.clone();
        future_to_promise(async move {
            let vfs = vfs.lock().await;

            match vfs.exists(&path).await {
                Ok(exists) => Ok(JsValue::from_bool(exists)),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = getMetadata)]
    pub fn get_metadata(&self, path: String) -> Promise {
        let vfs = self.vfs.clone();
        future_to_promise(async move {
            let vfs = vfs.lock().await;

            match vfs.get_metadata(&path).await {
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

    #[wasm_bindgen(js_name = get)]
    pub fn get(&self, key: String) -> Promise {
        let bundle = self.bundle.clone();
        future_to_promise(async move {
            let mut bundle = bundle.lock().await;
            let path = BundlePath::from_str(&key);

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

    #[wasm_bindgen(js_name = put)]
    pub fn put(&self, key: String, value: Uint8Array) -> Promise {
        let bundle = self.bundle.clone();
        future_to_promise(async move {
            let mut bundle = bundle.lock().await;
            let path = BundlePath::from_str(&key);
            let data = value.to_vec();

            match bundle.put(&path, data) {
                Ok(_) => Ok(JsValue::TRUE),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = delete)]
    pub fn delete(&self, key: String) -> Promise {
        let bundle = self.bundle.clone();
        future_to_promise(async move {
            let mut bundle = bundle.lock().await;
            let path = BundlePath::from_str(&key);

            match bundle.delete(&path) {
                Ok(_) => Ok(JsValue::TRUE),
                Err(e) => Err(js_error(e)),
            }
        })
    }

    #[wasm_bindgen(js_name = listKeys)]
    pub fn list_keys(&self) -> Promise {
        let bundle = self.bundle.clone();
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
        let bundle = self.bundle.clone();
        future_to_promise(async move {
            let mut bundle = bundle.lock().await;
            let prefix_path = BundlePath::from_str(&prefix);

            match bundle.get_prefix(&prefix_path) {
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

    #[wasm_bindgen(js_name = toBytes)]
    pub fn to_bytes(&self) -> Promise {
        let bundle = self.bundle.clone();
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
pub fn create_sync_engine() -> Promise {
    WasmSyncEngine::new()
}

#[wasm_bindgen]
pub fn create_sync_engine_with_peer_id(peer_id: String) -> Promise {
    WasmSyncEngine::with_peer_id(peer_id)
}

#[wasm_bindgen]
pub fn create_bundle() -> std::result::Result<WasmBundle, JsValue> {
    match Bundle::create_empty() {
        Ok(bundle) => Ok(WasmBundle {
            bundle: Arc::new(Mutex::new(bundle)),
        }),
        Err(e) => Err(js_error(e)),
    }
}

#[wasm_bindgen]
pub fn create_bundle_from_bytes(data: Uint8Array) -> std::result::Result<WasmBundle, JsValue> {
    WasmBundle::from_bytes(data)
}
