//! Bundle the files the standard library includes.
//!
//! Hosts without the served `/library/` directory — the CLI, and the
//! worker when it runs natively — seed library documents from copies
//! compiled into the binary, so whatever those documents `!include` has
//! to be compiled in too: the binary runs where the source tree does not
//! exist (a shipped CLI, a test archive). Every library document is
//! parsed with the notation's own parser and each include is resolved
//! exactly as seeding resolves it, so the set embedded here is the set
//! seeding asks for. Only included files are embedded; the library's own
//! documents (and the megabytes of media beside them) are not.
//!
//! The output is `bundled.rs`: `(path, bytes)` pairs keyed by the path
//! relative to the library directory, read by `src/lib.rs`.

use std::path::{Path, PathBuf};

use tonk_notation::{Field, FieldValue, Url, parse_at};

/// The library directory, relative to this crate.
const LIBRARY: &str = "../tonk-core/assets/library";

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let directory = manifest
        .join(LIBRARY)
        .canonicalize()
        .expect("the library directory exists");
    // A new or removed document changes what is bundled.
    println!("cargo:rerun-if-changed={}", directory.display());
    let root = Url::from_directory_path(&directory).expect("an absolute path");

    let mut documents: Vec<PathBuf> = std::fs::read_dir(&directory)
        .expect("the library directory is readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "yaml")
        })
        .collect();
    documents.sort();

    let mut bundled: Vec<(String, PathBuf)> = Vec::new();
    for document in documents {
        println!("cargo:rerun-if-changed={}", document.display());
        let text = std::fs::read_to_string(&document).expect("a library document is readable");
        let location = Url::from_file_path(&document).expect("an absolute path");
        let name = document.file_name().unwrap().to_string_lossy().into_owned();
        let Some(syntax) = parse_at(location.clone(), &text).syntax else {
            continue;
        };
        let mut references = Vec::new();
        for expression in &syntax.expressions {
            collect(&expression.application().fields, &mut references);
        }
        for reference in references {
            let resolved = location.join(&reference).unwrap_or_else(|e| {
                panic!("{name} includes {reference:?}, which does not resolve: {e}")
            });
            let relative = resolved
                .as_str()
                .strip_prefix(root.as_str())
                .unwrap_or_else(|| {
                    panic!("{name} includes {reference:?}, which is outside the library directory")
                })
                .to_owned();
            let path = resolved.to_file_path().expect("a file URL");
            assert!(
                path.is_file(),
                "{name} includes {reference:?}, but {} does not exist",
                path.display()
            );
            if !bundled.iter().any(|(known, _)| *known == relative) {
                println!("cargo:rerun-if-changed={}", path.display());
                bundled.push((relative, path));
            }
        }
    }

    let mut entries = String::new();
    for (relative, path) in bundled {
        entries.push_str(&format!(
            "    ({relative:?}, include_bytes!({:?})),\n",
            path.display().to_string()
        ));
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    write(&out.join("bundled.rs"), &format!("&[\n{entries}]\n"));
}

/// Every `!include` reference under `fields`, in document order.
fn collect(fields: &[Field], out: &mut Vec<String>) {
    for field in fields {
        match &field.value {
            FieldValue::Include(include) => out.push(include.reference.clone()),
            FieldValue::Nested(nested) => collect(nested, out),
            FieldValue::Premises(premises) => {
                for premise in premises {
                    collect(&premise.bindings, out);
                }
            }
            _ => {}
        }
    }
}

fn write(path: &Path, contents: &str) {
    // Only rewrite on change, so an unchanged library does not dirty
    // the crate.
    if std::fs::read_to_string(path).ok().as_deref() != Some(contents) {
        std::fs::write(path, contents).expect("OUT_DIR is writable");
    }
}
