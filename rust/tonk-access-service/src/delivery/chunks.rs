//! Transparent bounded payload representation below D1's per-row size limit.
//! The parent stores a digest and length; ordered children and parent must be
//! committed in one transaction. Existing inline payloads remain readable.
use super::{MAX_APPROVAL_BYTES, StoreError};

pub const CHUNK_BYTES: usize = 512 * 1024;
const PREFIX: &str = "@chunks:";
#[derive(Clone, Copy)]
pub enum Kind {
    Initial,
    Addition,
}
impl Kind {
    pub fn insert(self) -> &'static str {
        match self {
            Self::Initial => {
                "INSERT INTO connection_delivery_chunk(request_hash,ordinal,content)
            SELECT ?1,?2,?3 FROM connection_delivery WHERE request_hash=?1 AND approval_hex=?4
            ON CONFLICT(request_hash,ordinal) DO NOTHING"
            }
            Self::Addition => {
                "INSERT INTO connection_addition_chunk(delivery_id,ordinal,content)
            SELECT ?1,?2,?3 FROM connection_addition WHERE delivery_id=?1 AND addition_hex=?4
            ON CONFLICT(delivery_id,ordinal) DO NOTHING"
            }
        }
    }
    pub fn select(self) -> &'static str {
        match self {
            Self::Initial => {
                "SELECT ordinal,content FROM connection_delivery_chunk WHERE request_hash=?1 ORDER BY ordinal LIMIT 17"
            }
            Self::Addition => {
                "SELECT ordinal,content FROM connection_addition_chunk WHERE delivery_id=?1 ORDER BY ordinal LIMIT 17"
            }
        }
    }
}
#[derive(Debug, serde::Deserialize)]
pub struct Chunk {
    pub ordinal: usize,
    pub content: String,
}
pub fn reference(payload: &str) -> Result<String, StoreError> {
    if payload.len() > MAX_APPROVAL_BYTES * 2 || !payload.is_ascii() {
        return Err(invalid());
    }
    Ok(format!(
        "{PREFIX}{}:{}",
        blake3::hash(payload.as_bytes()).to_hex(),
        payload.len()
    ))
}
pub fn is_chunked(value: &str) -> bool {
    value.starts_with(PREFIX)
}
pub fn pieces(payload: &str) -> impl Iterator<Item = &str> {
    // `reference` checked ASCII and the complete bound before callers use this.
    payload
        .as_bytes()
        .chunks(CHUNK_BYTES)
        .map(|c| std::str::from_utf8(c).expect("ASCII payload"))
}
pub fn assemble(marker: &str, chunks: Vec<Chunk>) -> Result<String, StoreError> {
    let (digest, length) = marker
        .strip_prefix(PREFIX)
        .and_then(|r| r.split_once(':'))
        .ok_or_else(invalid)?;
    let length: usize = length.parse().map_err(|_| invalid())?;
    if length > MAX_APPROVAL_BYTES * 2 || chunks.len() != length.div_ceil(CHUNK_BYTES) {
        return Err(invalid());
    }
    let mut result = String::with_capacity(length);
    for (position, chunk) in chunks.into_iter().enumerate() {
        let expected = (length - position * CHUNK_BYTES).min(CHUNK_BYTES);
        if chunk.ordinal != position || chunk.content.len() != expected || !chunk.content.is_ascii()
        {
            return Err(invalid());
        }
        result.push_str(&chunk.content);
    }
    if blake3::hash(result.as_bytes()).to_hex().as_str() != digest {
        return Err(invalid());
    }
    Ok(result)
}
fn invalid() -> StoreError {
    StoreError::Internal("incomplete or invalid delivery payload".into())
}
