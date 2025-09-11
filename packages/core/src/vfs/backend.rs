use crate::error::{Result, VfsError};
use crate::vfs::types::*;
use automerge::{transaction::Transactable, ObjType, ReadDoc, Value, ScalarValue};
use samod::{DocHandle, DocumentId};
use bytes::Bytes;

/// Helper functions for working with Automerge documents in the VFS
pub struct AutomergeHelpers;

impl AutomergeHelpers {
    /// Initialize a document as a directory node
    pub fn init_as_directory(handle: &DocHandle, name: &str) -> Result<()> {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "type", "dir")?;
            tx.put(automerge::ROOT, "name", name)?;

            let now = chrono::Utc::now().timestamp_millis();
            let timestamps_obj =
                tx.put_object(automerge::ROOT, "timestamps", automerge::ObjType::Map)?;
            tx.put(timestamps_obj.clone(), "created", now)?;
            tx.put(timestamps_obj, "modified", now)?;

            tx.put_object(automerge::ROOT, "children", automerge::ObjType::List)?;

            tx.commit();
            Ok(())
        })
    }

    /// Read a directory node from an Automerge document
    pub fn read_directory(handle: &DocHandle) -> Result<DirNode> {
        handle.with_document(|doc| {
            // Check if it's a directory
            let node_type = doc
                .get(automerge::ROOT, "type")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .unwrap_or_else(|| "dir".to_string());

            if node_type != "dir" {
                return Err(VfsError::NodeTypeMismatch {
                    expected: "dir".to_string(),
                    actual: node_type,
                });
            }

            // Get name
            let name = doc
                .get(automerge::ROOT, "name")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .unwrap_or_else(|| "/".to_string());

            // Get timestamps
            let timestamps = Self::read_timestamps(doc, automerge::ROOT)?;

            // Get children
            let mut children = Vec::new();
            if let Ok(Some((Value::Object(ObjType::List), children_obj_id))) =
                doc.get(automerge::ROOT, "children")
            {
                let len = doc.length(children_obj_id.clone());
                for i in 0..len {
                    if let Ok(Some((Value::Object(ObjType::Map), child_obj_id))) =
                        doc.get(children_obj_id.clone(), i)
                    {
                        if let Ok(ref_node) = Self::read_ref_node(doc, child_obj_id) {
                            children.push(ref_node);
                        }
                    }
                }
            }

            Ok(DirNode {
                node_type: NodeType::Directory,
                name,
                timestamps,
                children,
            })
        })
    }

    /// Add a child reference to a directory
    pub fn add_child_to_directory(handle: &DocHandle, child_ref: &RefNode) -> Result<()> {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();

            // Get or create children array
            let children_obj_id = match tx.get(automerge::ROOT, "children") {
                Ok(Some((Value::Object(ObjType::List), obj_id))) => obj_id,
                _ => {
                    // Children array doesn't exist, create it
                    tx.put_object(automerge::ROOT, "children", automerge::ObjType::List)?
                }
            };

            // Check if child already exists
            let len = tx.length(children_obj_id.clone());
            for i in 0..len {
                if let Ok(Some((Value::Object(ObjType::Map), child_obj_id))) =
                    tx.get(children_obj_id.clone(), i)
                {
                    if let Ok(Some((existing_name, _))) = tx.get(child_obj_id.clone(), "name") {
                        if Self::extract_string_value(&existing_name).as_deref()
                            == Some(&child_ref.name)
                        {
                            // Child already exists, update it
                            Self::write_ref_node(&mut tx, child_obj_id, child_ref)?;
                            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;

                            tx.commit();
                            return Ok(());
                        }
                    }
                }
            }

            // Child doesn't exist, add it
            let child_obj = tx.insert_object(children_obj_id, len, automerge::ObjType::Map)?;
            Self::write_ref_node(&mut tx, child_obj, child_ref)?;
            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;

            tx.commit();
            Ok(())
        })
    }

    /// Remove a child reference from a directory
    pub fn remove_child_from_directory(
        handle: &DocHandle,
        child_name: &str,
    ) -> Result<Option<RefNode>> {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            let mut found_child = None;

            if let Ok(Some((Value::Object(ObjType::List), children_obj_id))) =
                tx.get(automerge::ROOT, "children")
            {
                let len = tx.length(children_obj_id.clone());
                for i in 0..len {
                    if let Ok(Some((Value::Object(ObjType::Map), child_obj_id))) =
                        tx.get(children_obj_id.clone(), i)
                    {
                        if let Ok(Some((existing_name, _))) = tx.get(child_obj_id.clone(), "name") {
                            if Self::extract_string_value(&existing_name).as_deref()
                                == Some(child_name)
                            {
                                // Found the child, read it before deleting
                                found_child = Self::read_ref_node_from_tx(&tx, child_obj_id).ok();
                                tx.delete(children_obj_id, i)?;
                                Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;
                                break;
                            }
                        }
                    }
                }
            }

            tx.commit();
            Ok(found_child)
        })
    }

    /// Initialize a document as a document node
    pub fn init_as_document<T>(handle: &DocHandle, name: &str, content: T) -> Result<()>
    where
        T: serde::Serialize,
    {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "type", "doc")?;
            tx.put(automerge::ROOT, "name", name)?;

            let now = chrono::Utc::now().timestamp_millis();
            let timestamps_obj =
                tx.put_object(automerge::ROOT, "timestamps", automerge::ObjType::Map)?;
            tx.put(timestamps_obj.clone(), "created", now)?;
            tx.put(timestamps_obj, "modified", now)?;

            // Serialize content as JSON string for now
            let content_json =
                serde_json::to_string(&content).map_err(VfsError::SerializationError)?;
            tx.put(automerge::ROOT, "content", content_json)?;

            tx.commit();
            Ok(())
        })
    }

    /// Initialise a document as a document node (retaining the byte type of the second arg)
    pub fn init_as_document_with_bytes<T>(handle: &DocHandle, name: &str, content: T, bytes: Bytes) -> Result<()>
    where
        T: serde::Serialize,
    {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "type", "doc")?;
            tx.put(automerge::ROOT, "name", name)?;

            let now = chrono::Utc::now().timestamp_millis();
            let timestamps_obj =
                tx.put_object(automerge::ROOT, "timestamps", automerge::ObjType::Map)?;
            tx.put(timestamps_obj.clone(), "created", now)?;
            tx.put(timestamps_obj, "modified", now)?;

            // The design is quesitonable here, one option could be to store both content and bytes under one doc attribute
            // but i've decided to just store seperately for simplicity

            // Serialize content as JSON string
            let content_json =
                serde_json::to_string(&content).map_err(VfsError::SerializationError)?;
            tx.put(automerge::ROOT, "content", content_json)?;
            
            // Store bytes value seperately
            let bytes_scalar = ScalarValue::Bytes(bytes.to_vec());
            tx.put(automerge::ROOT, "bytes", bytes_scalar)?;

            tx.commit();
            Ok(())
        })
    }

    // Helper functions
    pub fn extract_string_value(value: &Value) -> Option<String> {
        match value {
            Value::Scalar(s) => {
                let s_str = s.to_string();
                // Remove quotes if present
                if s_str.starts_with('"') && s_str.ends_with('"') && s_str.len() > 1 {
                    Some(s_str[1..s_str.len() - 1].to_string())
                } else {
                    Some(s_str)
                }
            }
            _ => None,
        }
    }

    pub fn extract_bytes_value(value: &automerge::Value) -> Option<Vec<u8>> {
        match value {
            automerge::Value::Scalar(scalar) => {
                match scalar.as_ref() {
                    automerge::ScalarValue::Bytes(bytes) => Some(bytes.clone()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn read_timestamps(doc: &automerge::Automerge, obj_id: automerge::ObjId) -> Result<Timestamps> {
        let default_time = chrono::Utc::now();

        let created = doc
            .get(obj_id.clone(), "timestamps")
            .ok()
            .flatten()
            .and_then(|(value, ts_obj_id)| {
                if let Value::Object(_) = value {
                    doc.get(ts_obj_id, "created")
                        .ok()
                        .flatten()
                        .and_then(|(ts_val, _)| match ts_val {
                            Value::Scalar(s) => s.to_string().parse::<i64>().ok(),
                            _ => None,
                        })
                        .and_then(chrono::DateTime::from_timestamp_millis)
                } else {
                    None
                }
            })
            .unwrap_or(default_time);

        let modified = doc
            .get(obj_id, "timestamps")
            .ok()
            .flatten()
            .and_then(|(value, ts_obj_id)| {
                if let Value::Object(_) = value {
                    doc.get(ts_obj_id, "modified")
                        .ok()
                        .flatten()
                        .and_then(|(ts_val, _)| match ts_val {
                            Value::Scalar(s) => s.to_string().parse::<i64>().ok(),
                            _ => None,
                        })
                        .and_then(chrono::DateTime::from_timestamp_millis)
                } else {
                    None
                }
            })
            .unwrap_or(default_time);

        Ok(Timestamps { created, modified })
    }

    fn read_timestamps_from_tx(
        tx: &automerge::transaction::Transaction<'_>,
        obj_id: automerge::ObjId,
    ) -> Result<Timestamps> {
        let default_time = chrono::Utc::now();

        let created = tx
            .get(obj_id.clone(), "timestamps")
            .ok()
            .flatten()
            .and_then(|(value, ts_obj_id)| {
                if let Value::Object(_) = value {
                    tx.get(ts_obj_id, "created")
                        .ok()
                        .flatten()
                        .and_then(|(ts_val, _)| match ts_val {
                            Value::Scalar(s) => s.to_string().parse::<i64>().ok(),
                            _ => None,
                        })
                        .and_then(chrono::DateTime::from_timestamp_millis)
                } else {
                    None
                }
            })
            .unwrap_or(default_time);

        let modified = tx
            .get(obj_id, "timestamps")
            .ok()
            .flatten()
            .and_then(|(value, ts_obj_id)| {
                if let Value::Object(_) = value {
                    tx.get(ts_obj_id, "modified")
                        .ok()
                        .flatten()
                        .and_then(|(ts_val, _)| match ts_val {
                            Value::Scalar(s) => s.to_string().parse::<i64>().ok(),
                            _ => None,
                        })
                        .and_then(chrono::DateTime::from_timestamp_millis)
                } else {
                    None
                }
            })
            .unwrap_or(default_time);

        Ok(Timestamps { created, modified })
    }

    fn read_ref_node(doc: &automerge::Automerge, obj_id: automerge::ObjId) -> Result<RefNode> {
        let name = doc
            .get(obj_id.clone(), "name")
            .ok()
            .flatten()
            .and_then(|(value, _)| Self::extract_string_value(&value))
            .unwrap_or_default();

        let node_type_str = doc
            .get(obj_id.clone(), "type")
            .ok()
            .flatten()
            .and_then(|(value, _)| Self::extract_string_value(&value))
            .unwrap_or_else(|| "doc".to_string());

        let node_type = match node_type_str.as_str() {
            "dir" => NodeType::Directory,
            _ => NodeType::Document,
        };

        let pointer_str = doc
            .get(obj_id.clone(), "pointer")
            .ok()
            .flatten()
            .and_then(|(value, _)| Self::extract_string_value(&value))
            .ok_or_else(|| VfsError::InvalidDocumentStructure)?;

        let pointer = pointer_str
            .parse::<DocumentId>()
            .map_err(|_| VfsError::InvalidDocumentStructure)?;

        let timestamps = Self::read_timestamps(doc, obj_id)?;

        Ok(RefNode {
            pointer,
            node_type,
            timestamps,
            name,
        })
    }

    fn read_ref_node_from_tx(
        tx: &automerge::transaction::Transaction<'_>,
        obj_id: automerge::ObjId,
    ) -> Result<RefNode> {
        let name = tx
            .get(obj_id.clone(), "name")
            .ok()
            .flatten()
            .and_then(|(value, _)| Self::extract_string_value(&value))
            .unwrap_or_default();

        let node_type_str = tx
            .get(obj_id.clone(), "type")
            .ok()
            .flatten()
            .and_then(|(value, _)| Self::extract_string_value(&value))
            .unwrap_or_else(|| "doc".to_string());

        let node_type = match node_type_str.as_str() {
            "dir" => NodeType::Directory,
            _ => NodeType::Document,
        };

        let pointer_str = tx
            .get(obj_id.clone(), "pointer")
            .ok()
            .flatten()
            .and_then(|(value, _)| Self::extract_string_value(&value))
            .ok_or_else(|| VfsError::InvalidDocumentStructure)?;

        let pointer = pointer_str
            .parse::<DocumentId>()
            .map_err(|_| VfsError::InvalidDocumentStructure)?;

        let timestamps = Self::read_timestamps_from_tx(tx, obj_id)?;

        Ok(RefNode {
            pointer,
            node_type,
            timestamps,
            name,
        })
    }

    fn write_ref_node(
        tx: &mut automerge::transaction::Transaction<'_>,
        obj_id: automerge::ObjId,
        ref_node: &RefNode,
    ) -> Result<()> {
        tx.put(obj_id.clone(), "name", ref_node.name.clone())?;
        let type_str = match ref_node.node_type {
            NodeType::Directory => "dir",
            NodeType::Document => "doc",
        };
        tx.put(obj_id.clone(), "type", type_str)?;
        tx.put(obj_id.clone(), "pointer", ref_node.pointer.to_string())?;

        let timestamps_obj = tx.put_object(obj_id, "timestamps", automerge::ObjType::Map)?;
        tx.put(
            timestamps_obj.clone(),
            "created",
            ref_node.timestamps.created.timestamp_millis(),
        )?;
        tx.put(
            timestamps_obj,
            "modified",
            ref_node.timestamps.modified.timestamp_millis(),
        )?;

        Ok(())
    }

    fn update_modified_timestamp(
        tx: &mut automerge::transaction::Transaction<'_>,
        obj_id: automerge::ObjId,
    ) -> Result<()> {
        if let Ok(Some((Value::Object(_), ts_obj_id))) = tx.get(obj_id, "timestamps") {
            tx.put(ts_obj_id, "modified", chrono::Utc::now().timestamp_millis())?;
        }
        Ok(())
    }

    /// Read a document node from an Automerge document
    pub fn read_document<T>(handle: &DocHandle) -> Result<DocNode<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        handle.with_document(|doc| {
            // Check if it's a document
            let node_type = doc
                .get(automerge::ROOT, "type")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .unwrap_or_else(|| "doc".to_string());

            if node_type != "doc" {
                return Err(VfsError::NodeTypeMismatch {
                    expected: "doc".to_string(),
                    actual: node_type,
                });
            }

            // Get name
            let name = doc
                .get(automerge::ROOT, "name")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .unwrap_or_default();

            // Get timestamps
            let timestamps = Self::read_timestamps(doc, automerge::ROOT)?;

            // Get content
            let content_str = doc
                .get(automerge::ROOT, "content")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .ok_or_else(|| VfsError::InvalidDocumentStructure)?;

            let content =
                serde_json::from_str(&content_str).map_err(VfsError::SerializationError)?;

            Ok(DocNode {
                node_type: NodeType::Document,
                name,
                timestamps,
                content,
                content_bytes: None,
            })
        })
    }

    /// Read a document node from an Automerge document (specifically getting bytes)
    /// this is never actually used! doc reading goes through samod
    pub fn read_bytes_document<T>(handle: &DocHandle) -> Result<DocNode<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        handle.with_document(|doc| {
            // Check if it's a document
            let node_type = doc
                .get(automerge::ROOT, "type")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .unwrap_or_else(|| "doc".to_string());

            if node_type != "doc" {
                return Err(VfsError::NodeTypeMismatch {
                    expected: "doc".to_string(),
                    actual: node_type,
                });
            }

            // Get name
            let name = doc
                .get(automerge::ROOT, "name")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .unwrap_or_default();

            // Get timestamps
            let timestamps = Self::read_timestamps(doc, automerge::ROOT)?;

            // Get content
            let content_str = doc
                .get(automerge::ROOT, "content")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_string_value(&value))
                .ok_or_else(|| VfsError::InvalidDocumentStructure)?;

            let content =
                serde_json::from_str(&content_str).map_err(VfsError::SerializationError)?;

            let content_bytes = doc
                .get(automerge::ROOT, "bytes")
                .map_err(VfsError::AutomergeError)?
                .and_then(|(value, _)| Self::extract_bytes_value(&value))
                .ok_or_else(|| VfsError::InvalidDocumentStructure)?;

            Ok(DocNode {
                node_type: NodeType::Document,
                name,
                timestamps,
                content,
                content_bytes,
            })
        })
    }

    /// Update content of an existing document
    pub fn update_document_content<T>(handle: &DocHandle, content: T) -> Result<()>
    where
        T: serde::Serialize,
    {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();

            // Update content
            let content_json =
                serde_json::to_string(&content).map_err(VfsError::SerializationError)?;
            tx.put(automerge::ROOT, "content", content_json)?;

            // Update modified timestamp
            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;

            tx.commit();
            Ok(())
        })
    }

    /// Update content and binary data of an existing document
    pub fn update_document_content_with_bytes<T>(handle: &DocHandle, content: T, bytes: Bytes) -> Result<()>
    where
        T: serde::Serialize,
    {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();

            // Update content
            let content_json =
                serde_json::to_string(&content).map_err(VfsError::SerializationError)?;
            tx.put(automerge::ROOT, "content", content_json)?;

            // Update binary data
            let bytes_scalar = ScalarValue::Bytes(bytes.to_vec());
            tx.put(automerge::ROOT, "bytes", bytes_scalar)?;

            // Update modified timestamp
            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;

            tx.commit();
            Ok(())
        })
    }

    /// Update the timestamp of a RefNode in a directory
    pub fn update_child_ref_timestamp(handle: &DocHandle, child_name: &str) -> Result<bool> {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            let mut found = false;

            if let Ok(Some((Value::Object(ObjType::List), children_obj_id))) =
                tx.get(automerge::ROOT, "children")
            {
                let len = tx.length(children_obj_id.clone());
                for i in 0..len {
                    if let Ok(Some((Value::Object(ObjType::Map), child_obj_id))) =
                        tx.get(children_obj_id.clone(), i)
                    {
                        if let Ok(Some((existing_name, _))) = tx.get(child_obj_id.clone(), "name") {
                            if Self::extract_string_value(&existing_name).as_deref()
                                == Some(child_name)
                            {
                                // Found the child, update its timestamp
                                if let Ok(Some((Value::Object(_), ts_obj_id))) =
                                    tx.get(child_obj_id, "timestamps")
                                {
                                    let now = chrono::Utc::now().timestamp_millis();
                                    tx.put(ts_obj_id, "modified", now)?;
                                    found = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            if found {
                // Also update the directory's own modified timestamp
                Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;
            }

            tx.commit();
            Ok(found)
        })
    }
}
