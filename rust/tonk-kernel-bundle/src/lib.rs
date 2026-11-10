//! A kernel, bundled for a host that compiles no WebAssembly, such as a
//! Cloudflare Worker.
//!
//! The polyfill compiles a component's core modules, and the modules it
//! generates as it links, from their bytes. A Worker compiles none: it takes
//! each one compiled ahead of time, looked up by the SHA-256 of its bytes (see
//! `wcmp_wasm_core_web::Precompiled`), and a Worker must be deployed with every
//! one of them. A run of the kernel where compiling is allowed records them.
//!
//! The Worker also needs the component itself, which the polyfill parses,
//! validates and links. Of a core module it reads the types, imports and
//! exports, and never runs the code: it runs the module compiled ahead of
//! time. So the component the Worker is deployed with holds, for each core
//! module, a [`skeleton`]: every function body is `unreachable` and every data
//! segment is empty. The skeleton validates as the module did and has the
//! same boundary, and the module compiled ahead of time is filed under the
//! key of the skeleton's bytes, which are what the polyfill hashes. A kernel's
//! core module is most of its size, so this halves what a Worker carries.

use sha2::{Digest, Sha256};
use wasmparser::{DataSectionReader, Parser, Payload};

/// A component's section that holds a core module, and one that holds a
/// nested component.
const CORE_MODULE: u8 = 1;
const COMPONENT: u8 = 4;
/// A core module's code section, and its data section.
const CODE: u8 = 10;
const DATA: u8 = 11;

/// The body of a function that does nothing but trap: no locals,
/// `unreachable`, `end`. It validates whatever the function's type.
const UNREACHABLE: [u8; 3] = [0x00, 0x00, 0x0b];

/// The key a module is looked up by: the SHA-256 of its bytes, in lowercase
/// hexadecimal.
pub fn key(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A core module whose bytes the component held, by the key of those bytes,
/// and the key of its skeleton, which the component now holds instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    pub module: String,
    pub skeleton: String,
}

/// `component` with each core module it holds, nested components' included,
/// in place of the module: and each module, by its key, renamed to its
/// skeleton's.
pub fn skeleton(component: &[u8]) -> Result<(Vec<u8>, Vec<Renamed>), String> {
    let mut renamed = Vec::new();
    let bytes = rewrite(component, &mut |id, body| match id {
        CORE_MODULE => {
            let skeleton = module(body)?;
            renamed.push(Renamed {
                module: key(body),
                skeleton: key(&skeleton),
            });
            Ok(skeleton)
        }
        COMPONENT => {
            let (nested, inner) = skeleton(body)?;
            renamed.extend(inner);
            Ok(nested)
        }
        _ => Ok(body.to_vec()),
    })?;
    Ok((bytes, renamed))
}

/// The core module `bytes` with each function body `unreachable` and each data
/// segment empty.
fn module(bytes: &[u8]) -> Result<Vec<u8>, String> {
    rewrite(bytes, &mut |id, body| match id {
        CODE => code(body),
        DATA => data(body),
        _ => Ok(body.to_vec()),
    })
}

/// A code section of as many functions as `body` holds, each `unreachable`.
fn code(body: &[u8]) -> Result<Vec<u8>, String> {
    let (count, _) = leb128(body, 0)?;
    let mut out = Vec::new();
    write_leb128(&mut out, count);
    for _ in 0..count {
        write_leb128(&mut out, UNREACHABLE.len());
        out.extend_from_slice(&UNREACHABLE);
    }
    Ok(out)
}

/// The data section `body` with each segment's bytes left out: its kind,
/// memory and offset as they were.
fn data(body: &[u8]) -> Result<Vec<u8>, String> {
    let malformed = |error: wasmparser::BinaryReaderError| error.to_string();
    let reader =
        DataSectionReader::new(wasmparser::BinaryReader::new(body, 0)).map_err(malformed)?;
    let mut out = Vec::new();
    write_leb128(&mut out, reader.count() as usize);
    for segment in reader {
        let segment = segment.map_err(malformed)?;
        // The segment's bytes end it, after their length: everything before
        // that length is the segment's kind, memory and offset.
        let data_start = segment.data.as_ptr() as usize - body.as_ptr() as usize;
        let header_end = data_start - leb128_len(segment.data.len());
        let header_start =
            usize::try_from(segment.range.start).map_err(|error| error.to_string())?;
        out.extend_from_slice(&body[header_start..header_end]);
        write_leb128(&mut out, 0);
    }
    Ok(out)
}

/// What becomes of a section, given its id and body.
type Section<'a> = dyn FnMut(u8, &[u8]) -> Result<Vec<u8>, String> + 'a;

/// The module or component `bytes` with each section as `each` makes it.
fn rewrite(bytes: &[u8], each: &mut Section<'_>) -> Result<Vec<u8>, String> {
    if bytes.len() < 8 {
        return Err("no WebAssembly preamble".into());
    }
    // The preamble: the magic number, the version and the layer.
    let mut out = bytes[..8].to_vec();
    let mut at = 8;
    while at < bytes.len() {
        let id = bytes[at];
        let (size, start) = leb128(bytes, at + 1)?;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or("a section runs past the end")?;
        let body = each(id, &bytes[start..end])?;
        out.push(id);
        write_leb128(&mut out, body.len());
        out.extend_from_slice(&body);
        at = end;
    }
    Ok(out)
}

/// The unsigned LEB128 number at `at` in `bytes`, and where what follows it
/// starts.
fn leb128(bytes: &[u8], mut at: usize) -> Result<(usize, usize), String> {
    let (mut value, mut shift) = (0usize, 0);
    loop {
        let byte = *bytes.get(at).ok_or("a number runs past the end")?;
        at += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, at));
        }
        shift += 7;
        if shift > 28 {
            return Err("a number is longer than 32 bits".into());
        }
    }
}

/// How many bytes `value` takes as unsigned LEB128.
fn leb128_len(value: usize) -> usize {
    let mut out = Vec::new();
    write_leb128(&mut out, value);
    out.len()
}

fn write_leb128(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Every core module `component` holds, nested components' included, by its
/// key. What a host is deployed with besides the skeleton is these, and the
/// modules the polyfill generates.
pub fn modules(component: &[u8]) -> Result<Vec<String>, String> {
    let mut keys = Vec::new();
    for payload in Parser::new(0).parse_all(component) {
        if let Payload::ModuleSection {
            unchecked_range, ..
        } = payload.map_err(|error| error.to_string())?
        {
            let start =
                usize::try_from(unchecked_range.start).map_err(|error| error.to_string())?;
            let end = usize::try_from(unchecked_range.end).map_err(|error| error.to_string())?;
            keys.push(key(&component[start..end]));
        }
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A component holding one core module with a function that returns 7,
    /// and a data segment.
    fn component() -> Vec<u8> {
        wat::parse_str(
            r#"
            (component
              (core module $m
                (memory (export "memory") 1)
                (data (i32.const 8) "hello")
                (func (export "seven") (result i32) i32.const 7))
              (core instance $i (instantiate $m)))
            "#,
        )
        .expect("the component parses")
    }

    #[test]
    fn it_keeps_a_component_that_validates_as_it_did() {
        let (skeleton, _) = skeleton(&component()).expect("the component rewrites");
        let mut validator = wasmparser::Validator::new();
        validator
            .validate_all(&skeleton)
            .expect("the skeleton validates");
    }

    #[test]
    fn it_leaves_out_the_code_and_the_data_of_each_core_module() {
        let original = component();
        let (skeleton, _) = skeleton(&original).expect("the component rewrites");
        assert!(skeleton.len() < original.len());
        let text = words(&skeleton).join(" ");
        assert!(text.contains("unreachable"), "{text}");
        assert!(!text.contains("hello"), "{text}");
        assert!(text.contains("(export \"seven\""), "{text}");
    }

    #[test]
    fn it_renames_each_core_module_to_its_skeleton() {
        let original = component();
        let (skeleton, renamed) = skeleton(&original).expect("the component rewrites");
        assert_eq!(renamed.len(), 1);
        assert_eq!(modules(&original).unwrap(), vec![renamed[0].module.clone()]);
        assert_eq!(
            modules(&skeleton).unwrap(),
            vec![renamed[0].skeleton.clone()]
        );
    }

    #[test]
    fn it_keys_a_module_by_the_sha256_of_its_bytes() {
        assert_eq!(
            key(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// Each export, instruction and data segment `bytes` holds, as words
    /// for a test to read.
    fn words(bytes: &[u8]) -> Vec<String> {
        let mut words = Vec::new();
        for payload in Parser::new(0).parse_all(bytes) {
            match payload.expect("the component reads") {
                Payload::ExportSection(exports) => {
                    for export in exports {
                        words.push(format!("(export \"{}\"", export.expect("an export").name));
                    }
                }
                Payload::CodeSectionEntry(body) => {
                    for operator in body.get_operators_reader().expect("a body") {
                        words.push(format!("{:?}", operator.expect("an operator")).to_lowercase());
                    }
                }
                Payload::DataSection(segments) => {
                    for segment in segments {
                        words
                            .push(String::from_utf8_lossy(segment.expect("a segment").data).into());
                    }
                }
                _ => {}
            }
        }
        words
    }
}
