//! The boot cell of a space, as both its host and its kernel read it.
//!
//! The boot cell is `boot/loader`. Its content is a DAG-CBOR map,
//! `{ "v": 2, "kernel": <id> }`, where the id is the content identifier of
//! the kernel's wasm (CIDv1, raw codec, BLAKE3, base32). The wasm is the
//! space's blob under that BLAKE3 hash. A space that holds both runs that
//! kernel, and needs nothing from anywhere else to start it.

/// The namespace of the boot cell.
pub const NAMESPACE: &str = "boot";
/// The name of the boot cell in its namespace.
pub const CELL: &str = "loader";
/// The host version a kernel pinned this way needs.
const VERSION: u64 = 2;

/// The bytes that open the content identifier of a kernel, before its
/// digest: CIDv1, the raw codec (0x55), BLAKE3 (0x1e), 32 bytes.
const PREFIX: [u8; 4] = [0x01, 0x55, 0x1e, 0x20];

/// The content identifier of a kernel's wasm.
pub fn kernel_id(kernel: &[u8]) -> String {
    let mut bytes = PREFIX.to_vec();
    bytes.extend_from_slice(blake3::hash(kernel).as_bytes());
    format!("b{}", base32(&bytes))
}

/// The BLAKE3 digest a kernel's content identifier names.
pub fn digest(id: &str) -> Result<[u8; 32], String> {
    let bytes = id
        .strip_prefix('b')
        .and_then(unbase32)
        .ok_or_else(|| format!("{id} is not a base32 content identifier"))?;
    match bytes.split_at_checked(4) {
        Some((prefix, digest)) if prefix == PREFIX => digest
            .try_into()
            .map_err(|_| format!("{id} does not name a 32-byte BLAKE3 digest")),
        _ => Err(format!("{id} does not name a kernel by its BLAKE3 hash")),
    }
}

/// The boot cell's content for the kernel `id`, in DAG-CBOR's key order (by
/// length, then bytes), so a given pin is the same bytes on every device.
pub fn cell_content(id: &str) -> Vec<u8> {
    let mut bytes = vec![0xa2];
    text(&mut bytes, "v");
    bytes.push(VERSION as u8);
    text(&mut bytes, "kernel");
    text(&mut bytes, id);
    bytes
}

/// The kernel a boot cell's content pins.
pub fn read_cell(content: &[u8]) -> Result<String, String> {
    let mut reader = Reader(content);
    let entries = reader.head(5)?;
    let (mut version, mut kernel) = (None, None);
    for _ in 0..entries {
        match reader.text()?.as_str() {
            "v" => version = Some(reader.head(0)?),
            "kernel" => kernel = Some(reader.text()?),
            other => return Err(format!("the boot cell has an unknown key {other}")),
        }
    }
    match (version, kernel) {
        (Some(VERSION), Some(kernel)) => Ok(kernel),
        (Some(version), _) if version != VERSION => {
            Err(format!("the boot cell is for host version {version}"))
        }
        _ => Err("the boot cell names no kernel".to_owned()),
    }
}

fn text(bytes: &mut Vec<u8>, value: &str) {
    let length = value.len();
    match length {
        0..24 => bytes.push(0x60 | length as u8),
        24..256 => bytes.extend([0x78, length as u8]),
        _ => {
            bytes.push(0x79);
            bytes.extend((length as u16).to_be_bytes());
        }
    }
    bytes.extend_from_slice(value.as_bytes());
}

/// Reads the few CBOR items a boot cell holds.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, String> {
        let (&first, rest) = self.0.split_first().ok_or("the boot cell ends early")?;
        self.0 = rest;
        Ok(first)
    }

    /// The argument of an item of major type `major`.
    fn head(&mut self, major: u8) -> Result<u64, String> {
        let first = self.byte()?;
        if first >> 5 != major {
            return Err(format!("the boot cell holds an unexpected item {first:#x}"));
        }
        match first & 0x1f {
            small @ 0..24 => Ok(small as u64),
            24 => Ok(self.byte()? as u64),
            25 => Ok(u16::from_be_bytes([self.byte()?, self.byte()?]) as u64),
            other => Err(format!("the boot cell holds a length of the form {other}")),
        }
    }

    fn text(&mut self) -> Result<String, String> {
        let length = self.head(3)? as usize;
        let (value, rest) = self
            .0
            .split_at_checked(length)
            .ok_or("the boot cell ends early")?;
        self.0 = rest;
        String::from_utf8(value.to_vec()).map_err(|error| error.to_string())
    }
}

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32, lowercase, unpadded: the multibase `b`.
fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for &byte in bytes {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for character in text.bytes() {
        let value = ALPHABET.iter().position(|&letter| letter == character)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_names_a_kernel_by_a_blake3_cid() {
        let id = kernel_id(b"kernel");
        assert!(id.starts_with("bafkr4i"), "{id}");
        assert_eq!(digest(&id).unwrap(), *blake3::hash(b"kernel").as_bytes());
    }

    #[test]
    fn it_reads_the_kernel_a_boot_cell_pins() {
        let id = kernel_id(b"kernel");
        assert_eq!(read_cell(&cell_content(&id)).unwrap(), id);
        assert_eq!(cell_content(&id)[..4], [0xa2, 0x61, b'v', 0x02]);
    }
}
