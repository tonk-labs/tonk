//! The files the standard library includes, bundled into the binary.
//!
//! The service worker seeds the library from its served `/library/`
//! directory, and an `!include` there is fetched from beside it. A host
//! without that directory — the CLI, or the worker running natively —
//! seeds from a copy compiled into the binary, so the documents have no
//! place of their own to be relative to. They are given [`ROOT`]
//! ("the library bundled with this binary") instead, and [`Bundled`]
//! serves what they include from files `build.rs` found by parsing them
//! and embedded here.

use tonk_notation::{Load, Url};

/// The directory bundled library documents are located in. A document
/// is parsed at `ROOT` joined with its file name, e.g.
/// `tonk-library:///core.yaml`, and its includes resolve against that.
pub const ROOT: &str = "tonk-library:///";

/// The files library documents include, by path relative to [`ROOT`].
const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/bundled.rs"));

/// The location of the bundled library document named `file`.
pub fn location(file: &str) -> Url {
    Url::parse(ROOT)
        .and_then(|root| root.join(file))
        .expect("ROOT is a directory URL")
}

/// Serves the files bundled library documents include, and nothing
/// else: a reference that resolves outside [`ROOT`], or to a file no
/// library document includes, is refused.
#[derive(Debug, Clone, Copy, Default)]
pub struct Bundled;

impl Bundled {
    /// The bundled content at `uri`.
    pub fn get(&self, uri: &Url) -> Option<&'static [u8]> {
        find(FILES, uri)
    }
}

impl Load for Bundled {
    async fn load(&self, uri: &Url) -> Result<Vec<u8>, String> {
        self.get(uri)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| format!("`{uri}` is not a file the standard library includes"))
    }
}

fn find(files: &[(&str, &'static [u8])], uri: &Url) -> Option<&'static [u8]> {
    let relative = uri.as_str().strip_prefix(ROOT)?;
    files
        .iter()
        .find(|(path, _)| *path == relative)
        .map(|(_, bytes)| *bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

    const FIXTURE: &[(&str, &[u8])] = &[("table/connected.js", b"(self) => {}")];

    /// An include written in a library document resolves against the
    /// document's location to the path `build.rs` keyed the file by.
    #[dialog_common::test]
    fn it_finds_a_bundled_file_by_the_reference_that_names_it() {
        let uri = location("table.yaml").join("./table/connected.js").unwrap();
        assert_eq!(find(FIXTURE, &uri), Some(&b"(self) => {}"[..]));
    }

    #[dialog_common::test]
    fn it_serves_nothing_it_did_not_bundle() {
        let base = location("table.yaml");
        for reference in ["./missing.js", "/etc/passwd", "file:///table/connected.js"] {
            let uri = base.join(reference).unwrap();
            assert_eq!(find(FIXTURE, &uri), None, "{reference}");
        }
    }

    /// Every file the library includes is bundled, so every one of them
    /// loads. A library that includes nothing bundles nothing.
    #[dialog_common::test]
    async fn it_loads_every_bundled_file() {
        for (path, bytes) in FILES {
            let uri = Url::parse(ROOT).unwrap().join(path).unwrap();
            assert_eq!(Bundled.load(&uri).await.as_deref(), Ok(*bytes), "{path}");
        }
    }
}
