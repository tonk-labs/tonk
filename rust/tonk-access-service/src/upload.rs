//! Backpressured upload validation. The last byte is withheld until EOF and
//! every signed constraint validates, so storage never sees a complete bad body.
use crate::operation::Operation;
use futures_util::Stream;
use sha2_0_10::{Digest, Sha256};
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pending,
    Complete,
    Invalid(String),
}
pub type Progress = Arc<Mutex<Verdict>>;
pub struct Validated<S> {
    inner: S,
    sha: Sha256,
    blake: blake3::Hasher,
    expected: Operation,
    declared: u64,
    received: u64,
    held: Option<u8>,
    done: bool,
    progress: Progress,
}
impl<S> Validated<S> {
    pub fn new(inner: S, expected: Operation, declared: u64) -> (Self, Progress) {
        let progress = Arc::new(Mutex::new(Verdict::Pending));
        (
            Self {
                inner,
                sha: Sha256::new(),
                blake: blake3::Hasher::new(),
                expected,
                declared,
                received: 0,
                held: None,
                done: false,
                progress: progress.clone(),
            },
            progress,
        )
    }
    fn validate(&self) -> Result<(), String> {
        if self.received != self.declared || self.expected.size.is_some_and(|n| n != self.received)
        {
            return Err("upload length does not match the signed or declared size".into());
        }
        if self
            .expected
            .sha256
            .as_ref()
            .is_some_and(|n| n.as_slice() != &self.sha.clone().finalize()[..])
            || self
                .expected
                .digest
                .as_ref()
                .is_some_and(|n| n.as_bytes() != self.blake.finalize().as_bytes())
        {
            return Err("upload digest does not match the signed invocation".into());
        }
        Ok(())
    }
    fn fail(&mut self, detail: String) -> Poll<Option<Result<Vec<u8>, String>>> {
        self.done = true;
        self.held = None;
        *self.progress.lock().expect("upload progress") = Verdict::Invalid(detail.clone());
        Poll::Ready(Some(Err(detail)))
    }
}
/// The compatibility path historically collects unknown-length archive/cell
/// payloads. Validate this buffer before any storage mutation.
pub fn validate_buffer(expected: &Operation, bytes: &[u8]) -> Result<(), String> {
    let (mut validator, _) = Validated::new((), expected.clone(), bytes.len() as u64);
    validator.received = bytes.len() as u64;
    validator.sha.update(bytes);
    validator.blake.update(bytes);
    validator.validate()
}
pub fn validate_empty(expected: &Operation) -> Result<(), String> {
    let (validator, _) = Validated::new((), expected.clone(), 0);
    validator.validate()
}
impl<S: Stream<Item = Result<Vec<u8>, String>> + Unpin> Stream for Validated<S> {
    type Item = Result<Vec<u8>, String>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.done {
            return Poll::Ready(None);
        }
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Err(error))) => return self.fail(error),
                Poll::Ready(Some(Ok(mut chunk))) => {
                    if chunk.is_empty() {
                        continue;
                    }
                    let Some(received) = self.received.checked_add(chunk.len() as u64) else {
                        return self.fail("upload length overflow".into());
                    };
                    self.received = received;
                    if received > self.declared || self.expected.size.is_some_and(|n| received > n)
                    {
                        return self.fail("upload exceeds its signed or declared size".into());
                    }
                    self.sha.update(&chunk);
                    self.blake.update(&chunk);
                    let last = chunk.pop().expect("nonempty chunk");
                    if let Some(previous) = self.held.replace(last) {
                        chunk.insert(0, previous);
                    }
                    if !chunk.is_empty() {
                        return Poll::Ready(Some(Ok(chunk)));
                    }
                }
                Poll::Ready(None) => {
                    if let Err(error) = self.validate() {
                        return self.fail(error);
                    }
                    self.done = true;
                    *self.progress.lock().expect("upload progress") = Verdict::Complete;
                    return Poll::Ready(self.held.take().map(|last| Ok(vec![last])));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{Method, Precondition};
    use futures_util::{StreamExt, stream};
    fn operation(bytes: &[u8]) -> Operation {
        Operation {
            method: Method::Put,
            key: "test".into(),
            sha256: Some(Sha256::digest(bytes).to_vec()),
            precondition: Precondition::None,
            digest: Some(dialog_common::Blake3Hash::hash(bytes)),
            size: Some(bytes.len() as u64),
            range: None,
            immutable: false,
        }
    }
    async fn consume(
        op: Operation,
        declared: u64,
        chunks: Vec<Result<Vec<u8>, String>>,
    ) -> (Vec<u8>, Verdict, bool) {
        let (mut stream, progress) = Validated::new(stream::iter(chunks), op, declared);
        let mut bytes = Vec::new();
        let mut failed = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => bytes.extend(chunk),
                Err(_) => failed = true,
            }
        }
        let verdict = progress.lock().unwrap().clone();
        (bytes, verdict, failed)
    }
    #[dialog_common::test]
    async fn releases_exact_body_only_after_all_constraints_pass() {
        for width in [1, 2, 7, 64] {
            let body = b"checked streaming content";
            let chunks = body.chunks(width).map(|c| Ok(c.to_vec())).collect();
            let (got, v, failed) = consume(operation(body), body.len() as u64, chunks).await;
            assert_eq!(got, body);
            assert_eq!(v, Verdict::Complete);
            assert!(!failed);
        }
    }
    #[dialog_common::test]
    async fn wrong_sha_or_blake_cannot_complete_the_storage_body() {
        let body = b"checked streaming content";
        for blake in [false, true] {
            let mut op = operation(body);
            if blake {
                op.digest = Some(dialog_common::Blake3Hash::hash(b"wrong"));
            } else {
                op.sha256 = Some(vec![0; 32]);
            }
            let (got, v, failed) = consume(op, body.len() as u64, vec![Ok(body.to_vec())]).await;
            assert_eq!(got.len(), body.len() - 1);
            assert!(matches!(v, Verdict::Invalid(_)));
            assert!(failed);
        }
    }
    #[dialog_common::test]
    async fn truncated_extra_and_failed_sources_never_complete() {
        let body = b"abcd";
        for chunks in [
            vec![Ok(b"abc".to_vec())],
            vec![Ok(b"abcd".to_vec()), Ok(b"x".to_vec())],
            vec![Ok(b"abcd".to_vec()), Err("disconnected".into())],
        ] {
            let (got, v, failed) = consume(operation(body), 4, chunks).await;
            assert!(got.len() < 4);
            assert!(matches!(v, Verdict::Invalid(_)));
            assert!(failed);
        }
    }
    #[dialog_common::test]
    fn empty_upload_must_match_both_digests_and_signed_size() {
        assert!(validate_empty(&operation(b"")).is_ok());
        let mut op = operation(b"");
        op.digest = Some(dialog_common::Blake3Hash::hash(b"x"));
        assert!(validate_empty(&op).is_err());
        let mut op = operation(b"");
        op.sha256 = Some(vec![0; 32]);
        assert!(validate_empty(&op).is_err());
        let mut op = operation(b"");
        op.size = Some(1);
        assert!(validate_empty(&op).is_err());
    }
    #[dialog_common::test]
    async fn only_one_byte_is_held_while_waiting_for_eof() {
        let body = b"four";
        let input = stream::iter([Ok(body.to_vec())]).chain(stream::pending());
        let (mut stream, progress) = Validated::new(input, operation(body), 4);
        assert_eq!(stream.next().await.unwrap().unwrap(), b"fou");
        assert_eq!(stream.held, Some(b'r'));
        assert_eq!(*progress.lock().unwrap(), Verdict::Pending);
    }
}
