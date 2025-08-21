use futures::stream::StreamExt;
use samod::DocHandle;

/// A watcher for document changes in the VFS
pub struct DocumentWatcher {
    handle: DocHandle,
}

impl DocumentWatcher {
    /// Create a new document watcher
    pub fn new(handle: DocHandle) -> Self {
        Self { handle }
    }

    /// Get the document handle
    pub fn handle(&self) -> &DocHandle {
        &self.handle
    }

    /// Get the document ID being watched
    pub fn document_id(&self) -> samod::DocumentId {
        self.handle.document_id().clone()
    }

    /// Watch for changes and call the callback for each change
    /// This function runs until the changes stream is closed
    pub async fn on_change<F>(self, mut callback: F)
    where
        F: FnMut(&mut automerge::Automerge) + Send,
    {
        let mut changes = self.handle.changes();
        while (changes.next().await).is_some() {
            // When a change occurs, call the callback with the current document state
            self.handle.with_document(|doc| callback(doc));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncEngine;
    use automerge::{AutomergeError, ROOT, ReadDoc, transaction::Transactable};
    use std::sync::{Arc, Mutex};
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_document_watcher_creation() {
        let engine = SyncEngine::new().await.unwrap();
        let doc = automerge::Automerge::new();
        let handle = engine.create_document(doc).await.unwrap();

        let watcher = DocumentWatcher::new(handle.clone());
        assert_eq!(watcher.document_id(), handle.document_id().clone());
    }

    #[tokio::test]
    async fn test_on_change_callback() {
        let engine = SyncEngine::new().await.unwrap();
        let doc = automerge::Automerge::new();
        let handle = engine.create_document(doc).await.unwrap();

        let watcher = DocumentWatcher::new(handle.clone());
        let received_values = Arc::new(Mutex::new(Vec::new()));

        // Spawn a task to listen for changes
        let listener_task = tokio::spawn({
            let received = received_values.clone();
            async move {
                watcher
                    .on_change(move |doc| {
                        // Get the value from the document
                        if let Ok(Some((automerge::Value::Scalar(v), _))) =
                            doc.get(ROOT, "test_key")
                        {
                            received.lock().unwrap().push(v.to_string());
                        }
                    })
                    .await;
            }
        });

        // Give the listener time to start
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Make a change to the document
        handle.with_document(|doc| {
            doc.transact::<_, _, AutomergeError>(|tx| {
                tx.put(ROOT, "test_key", "test_value")?;
                Ok(())
            })
            .unwrap();
        });

        // Wait a bit for the change to propagate
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Check that we received the change
        let values = received_values.lock().unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0], "\"test_value\"");

        // Clean up
        listener_task.abort();
    }

    #[tokio::test]
    async fn test_multiple_changes() {
        let engine = SyncEngine::new().await.unwrap();
        let doc = automerge::Automerge::new();
        let handle = engine.create_document(doc).await.unwrap();

        let watcher = DocumentWatcher::new(handle.clone());
        let change_count = Arc::new(Mutex::new(0));

        // Spawn the listener
        let listener_task = tokio::spawn({
            let count = change_count.clone();
            async move {
                watcher
                    .on_change(move |_doc| {
                        *count.lock().unwrap() += 1;
                    })
                    .await;
            }
        });

        // Give the listener time to start
        tokio::time::sleep(Duration::from_millis(10)).await;

        // Make multiple changes
        for i in 0..3 {
            handle.with_document(|doc| {
                doc.transact::<_, _, AutomergeError>(|tx| {
                    tx.put(ROOT, format!("key_{i}"), format!("value_{i}"))?;
                    Ok(())
                })
                .unwrap();
            });
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Wait for changes to be processed
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Verify we received all changes
        assert_eq!(*change_count.lock().unwrap(), 3);

        // Clean up
        listener_task.abort();
    }
}
