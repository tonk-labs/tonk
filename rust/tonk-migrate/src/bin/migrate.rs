//! The bundle the app's page loads, only when an earlier release left
//! something in its origin, to read what that release kept.

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use dialog_effects::storage::Directory;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use js_sys::{Array, Object, Reflect, Uint8Array};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tonk_migrate::{REGISTRY_PROFILE, local_spaces};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use wasm_bindgen::prelude::*;

/// Every space the earlier release kept on this device alone, each as
/// `{ subject, name, content, revision }`: the CARv1 of what its `main`
/// reaches and the revision's JSON, as bytes. That release kept its profiles
/// in the origin's profile directory, and their spaces in its current one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(js_name = localSpaces)]
pub async fn local_spaces_kept() -> Result<Array, JsValue> {
    let spaces = local_spaces(Directory::Profile, Directory::Current, REGISTRY_PROFILE)
        .await
        .map_err(|error| JsValue::from_str(&error.to_string()))?;
    let found = Array::new();
    for space in spaces {
        let entry = Object::new();
        Reflect::set(&entry, &"subject".into(), &space.subject.to_string().into())?;
        Reflect::set(&entry, &"name".into(), &space.name.into())?;
        Reflect::set(
            &entry,
            &"content".into(),
            &Uint8Array::from(space.content.as_slice()),
        )?;
        Reflect::set(
            &entry,
            &"revision".into(),
            &Uint8Array::from(space.revision.as_slice()),
        )?;
        found.push(&entry);
    }
    Ok(found)
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
#[wasm_bindgen(main)]
fn main() {}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn main() {}
