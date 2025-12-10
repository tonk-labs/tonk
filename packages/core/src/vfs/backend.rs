use crate::error::{Result, VfsError};
use crate::vfs::types::*;
use automerge::{transaction::Transactable, ObjType, ReadDoc, ScalarValue, Value};
use bytes::Bytes;
use samod::{DocHandle, DocumentId};

/// Helper functions for working with Automerge documents in the VFS
pub struct AutomergeHelpers;

impl AutomergeHelpers {
    /// Initialize a document as a directory node
    pub fn init_as_directory(handle: &DocHandle, name: &str) -> Result<()> {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "type", "directory")?;
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
                .unwrap_or_else(|| "directory".to_string());

            if node_type != "directory" {
                return Err(VfsError::NodeTypeMismatch {
                    expected: "directory".to_string(),
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
            tx.put(automerge::ROOT, "type", "document")?;
            tx.put(automerge::ROOT, "name", name)?;

            let now = chrono::Utc::now().timestamp_millis();
            let timestamps_obj =
                tx.put_object(automerge::ROOT, "timestamps", automerge::ObjType::Map)?;
            tx.put(timestamps_obj.clone(), "created", now)?;
            tx.put(timestamps_obj, "modified", now)?;

            // Convert content to JSON value and store as native Automerge objects
            let json_value =
                serde_json::to_value(&content).map_err(VfsError::SerializationError)?;

            // Create content object based on JSON type
            match &json_value {
                serde_json::Value::Object(map) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    for (k, v) in map {
                        Self::put_json_value(&mut tx, content_obj.clone(), k, v)?;
                    }
                }
                serde_json::Value::Array(arr) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::List)?;
                    for (i, item) in arr.iter().enumerate() {
                        Self::insert_json_value(&mut tx, content_obj.clone(), i, item)?;
                    }
                }
                // For primitive values, wrap in an object with a "value" key
                _ => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    Self::put_json_value(&mut tx, content_obj, "value", &json_value)?;
                }
            }

            tx.commit();
            Ok(())
        })
    }

    /// Initialize a document as a document node with bytes
    pub fn init_as_document_with_bytes<T>(
        handle: &DocHandle,
        name: &str,
        content: T,
        bytes: Bytes,
    ) -> Result<()>
    where
        T: serde::Serialize,
    {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "type", "document")?;
            tx.put(automerge::ROOT, "name", name)?;

            let now = chrono::Utc::now().timestamp_millis();
            let timestamps_obj =
                tx.put_object(automerge::ROOT, "timestamps", automerge::ObjType::Map)?;
            tx.put(timestamps_obj.clone(), "created", now)?;
            tx.put(timestamps_obj, "modified", now)?;

            // Convert content to JSON value and store as native Automerge objects
            let json_value =
                serde_json::to_value(&content).map_err(VfsError::SerializationError)?;

            // Create content object based on JSON type
            match &json_value {
                serde_json::Value::Object(map) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    for (k, v) in map {
                        Self::put_json_value(&mut tx, content_obj.clone(), k, v)?;
                    }
                }
                serde_json::Value::Array(arr) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::List)?;
                    for (i, item) in arr.iter().enumerate() {
                        Self::insert_json_value(&mut tx, content_obj.clone(), i, item)?;
                    }
                }
                // For primitive values, wrap in an object with a "value" key
                _ => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    Self::put_json_value(&mut tx, content_obj, "value", &json_value)?;
                }
            }

            // Store bytes value separately
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
            automerge::Value::Scalar(scalar) => match scalar.as_ref() {
                automerge::ScalarValue::Bytes(bytes) => Some(bytes.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    // ============================================================
    // JSON <-> Native Automerge Conversion Helpers
    // ============================================================

    /// Write a JSON value to an Automerge object at the given key
    fn put_json_value(
        tx: &mut automerge::transaction::Transaction<'_>,
        obj_id: automerge::ObjId,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<()> {
        match value {
            serde_json::Value::Null => {
                tx.put(obj_id, key, ())?;
            }
            serde_json::Value::Bool(b) => {
                tx.put(obj_id, key, *b)?;
            }
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    tx.put(obj_id, key, i)?;
                } else if let Some(f) = n.as_f64() {
                    tx.put(obj_id, key, f)?;
                }
            }
            serde_json::Value::String(s) => {
                tx.put(obj_id, key, s.as_str())?;
            }
            serde_json::Value::Array(arr) => {
                let list_obj = tx.put_object(obj_id, key, ObjType::List)?;
                for (i, item) in arr.iter().enumerate() {
                    Self::insert_json_value(tx, list_obj.clone(), i, item)?;
                }
            }
            serde_json::Value::Object(map) => {
                let map_obj = tx.put_object(obj_id, key, ObjType::Map)?;
                for (k, v) in map {
                    Self::put_json_value(tx, map_obj.clone(), k, v)?;
                }
            }
        }
        Ok(())
    }

    /// Insert a JSON value into an Automerge list at the given index
    fn insert_json_value(
        tx: &mut automerge::transaction::Transaction<'_>,
        obj_id: automerge::ObjId,
        index: usize,
        value: &serde_json::Value,
    ) -> Result<()> {
        match value {
            serde_json::Value::Null => {
                tx.insert(obj_id, index, ())?;
            }
            serde_json::Value::Bool(b) => {
                tx.insert(obj_id, index, *b)?;
            }
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    tx.insert(obj_id, index, i)?;
                } else if let Some(f) = n.as_f64() {
                    tx.insert(obj_id, index, f)?;
                }
            }
            serde_json::Value::String(s) => {
                tx.insert(obj_id, index, s.as_str())?;
            }
            serde_json::Value::Array(arr) => {
                let list_obj = tx.insert_object(obj_id, index, ObjType::List)?;
                for (i, item) in arr.iter().enumerate() {
                    Self::insert_json_value(tx, list_obj.clone(), i, item)?;
                }
            }
            serde_json::Value::Object(map) => {
                let map_obj = tx.insert_object(obj_id, index, ObjType::Map)?;
                for (k, v) in map {
                    Self::put_json_value(tx, map_obj.clone(), k, v)?;
                }
            }
        }
        Ok(())
    }

    /// Read an Automerge value and convert it to a JSON value
    fn read_automerge_value(
        doc: &automerge::Automerge,
        obj_id: automerge::ObjId,
    ) -> Result<serde_json::Value> {
        use automerge::ReadDoc;

        let obj_type = doc.object_type(&obj_id);

        match obj_type {
            Ok(ObjType::Map) | Ok(ObjType::Table) => {
                let mut map = serde_json::Map::new();
                for key in doc.keys(&obj_id) {
                    if let Ok(Some((value, inner_obj_id))) = doc.get(&obj_id, &key) {
                        let json_val = Self::value_to_json(doc, &value, inner_obj_id)?;
                        map.insert(key.to_string(), json_val);
                    }
                }
                Ok(serde_json::Value::Object(map))
            }
            Ok(ObjType::List) => {
                let mut arr = Vec::new();
                let len = doc.length(&obj_id);
                for i in 0..len {
                    if let Ok(Some((value, inner_obj_id))) = doc.get(&obj_id, i) {
                        let json_val = Self::value_to_json(doc, &value, inner_obj_id)?;
                        arr.push(json_val);
                    }
                }
                Ok(serde_json::Value::Array(arr))
            }
            Ok(ObjType::Text) => {
                let text = doc.text(&obj_id).map_err(VfsError::AutomergeError)?;
                Ok(serde_json::Value::String(text))
            }
            Err(_) => Ok(serde_json::Value::Null),
        }
    }

    /// Convert an Automerge Value to a JSON value
    fn value_to_json(
        doc: &automerge::Automerge,
        value: &Value,
        obj_id: automerge::ObjId,
    ) -> Result<serde_json::Value> {
        match value {
            Value::Object(ObjType::Map)
            | Value::Object(ObjType::List)
            | Value::Object(ObjType::Text)
            | Value::Object(ObjType::Table) => Self::read_automerge_value(doc, obj_id),
            Value::Scalar(s) => {
                match s.as_ref() {
                    ScalarValue::Null => Ok(serde_json::Value::Null),
                    ScalarValue::Boolean(b) => Ok(serde_json::Value::Bool(*b)),
                    ScalarValue::Int(i) => Ok(serde_json::Value::Number((*i).into())),
                    ScalarValue::Uint(u) => Ok(serde_json::Value::Number((*u).into())),
                    ScalarValue::F64(f) => Ok(serde_json::Number::from_f64(*f)
                        .map(serde_json::Value::Number)
                        .unwrap_or(serde_json::Value::Null)),
                    ScalarValue::Str(s) => Ok(serde_json::Value::String(s.to_string())),
                    ScalarValue::Bytes(b) => {
                        // Encode bytes as base64 string
                        use base64::Engine;
                        let encoded = base64::engine::general_purpose::STANDARD.encode(b);
                        Ok(serde_json::Value::String(encoded))
                    }
                    ScalarValue::Counter(c) => Ok(serde_json::Value::Number(i64::from(c).into())),
                    ScalarValue::Timestamp(t) => Ok(serde_json::Value::Number((*t).into())),
                    ScalarValue::Unknown { .. } => Ok(serde_json::Value::Null),
                }
            }
        }
    }

    /// Navigate to a specific path within a document, returning the object ID
    fn navigate_to_path(doc: &automerge::Automerge, path: &[String]) -> Result<automerge::ObjId> {
        let mut current = automerge::ROOT;

        for key in path {
            match doc.get(current.clone(), key.as_str()) {
                Ok(Some((Value::Object(_), obj_id))) => current = obj_id,
                Ok(Some(_)) => {
                    return Err(VfsError::Other(anyhow::anyhow!(
                        "Path element '{}' is not an object",
                        key
                    )))
                }
                Ok(None) => {
                    return Err(VfsError::Other(anyhow::anyhow!(
                        "Path element '{}' not found",
                        key
                    )))
                }
                Err(e) => return Err(VfsError::AutomergeError(e)),
            }
        }

        Ok(current)
    }

    /// Navigate to parent of a path, returning (parent_obj_id, final_key)
    fn navigate_to_parent(
        doc: &automerge::Automerge,
        path: &[String],
    ) -> Result<(automerge::ObjId, String)> {
        if path.is_empty() {
            return Err(VfsError::Other(anyhow::anyhow!("Path cannot be empty")));
        }

        let parent_path = &path[..path.len() - 1];
        let final_key = path.last().unwrap().clone();

        let parent_obj = if parent_path.is_empty() {
            automerge::ROOT
        } else {
            Self::navigate_to_path(doc, parent_path)?
        };

        Ok((parent_obj, final_key))
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
            .unwrap_or_else(|| "document".to_string());

        let node_type = match node_type_str.as_str() {
            "directory" => NodeType::Directory,
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
            .unwrap_or_else(|| "document".to_string());

        let node_type = match node_type_str.as_str() {
            "directory" => NodeType::Directory,
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
            NodeType::Directory => "directory",
            NodeType::Document => "document",
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
                .unwrap_or_else(|| "document".to_string());

            if node_type != "document" {
                return Err(VfsError::NodeTypeMismatch {
                    expected: "document".to_string(),
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
            let content_result = doc
                .get(automerge::ROOT, "content")
                .map_err(VfsError::AutomergeError)?;

            let content: T = match content_result {
                Some((Value::Object(_), content_obj_id)) => {
                    // Native storage: read as Automerge object and convert to JSON
                    let json_value = Self::read_automerge_value(doc, content_obj_id)?;
                    serde_json::from_value(json_value).map_err(VfsError::SerializationError)?
                }
                Some((value, _)) => {
                    // Legacy storage: content is a JSON string
                    let content_str = Self::extract_string_value(&value)
                        .ok_or_else(|| VfsError::InvalidDocumentStructure)?;
                    serde_json::from_str(&content_str).map_err(VfsError::SerializationError)?
                }
                None => {
                    return Err(VfsError::InvalidDocumentStructure);
                }
            };

            Ok(DocNode {
                node_type: NodeType::Document,
                name,
                timestamps,
                content,
                bytes: None,
            })
        })
    }

    /// Read a document node from an Automerge document with bytes
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
                .unwrap_or_else(|| "document".to_string());

            if node_type != "document" {
                return Err(VfsError::NodeTypeMismatch {
                    expected: "document".to_string(),
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
            let content_result = doc
                .get(automerge::ROOT, "content")
                .map_err(VfsError::AutomergeError)?;

            let content: T = match content_result {
                Some((Value::Object(_), content_obj_id)) => {
                    // Native storage: read as Automerge object and convert to JSON
                    let json_value = Self::read_automerge_value(doc, content_obj_id)?;
                    serde_json::from_value(json_value).map_err(VfsError::SerializationError)?
                }
                Some((value, _)) => {
                    // Legacy storage: content is a JSON string
                    let content_str = Self::extract_string_value(&value)
                        .ok_or_else(|| VfsError::InvalidDocumentStructure)?;
                    serde_json::from_str(&content_str).map_err(VfsError::SerializationError)?
                }
                None => {
                    return Err(VfsError::InvalidDocumentStructure);
                }
            };

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
                bytes: Some(content_bytes),
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

            // Convert content to JSON value and store as native Automerge objects
            let json_value =
                serde_json::to_value(&content).map_err(VfsError::SerializationError)?;

            // Delete old content if it exists
            let _ = tx.delete(automerge::ROOT, "content");

            // Create new content object based on JSON type
            match &json_value {
                serde_json::Value::Object(map) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    for (k, v) in map {
                        Self::put_json_value(&mut tx, content_obj.clone(), k, v)?;
                    }
                }
                serde_json::Value::Array(arr) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::List)?;
                    for (i, item) in arr.iter().enumerate() {
                        Self::insert_json_value(&mut tx, content_obj.clone(), i, item)?;
                    }
                }
                _ => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    Self::put_json_value(&mut tx, content_obj, "value", &json_value)?;
                }
            }

            // Update modified timestamp
            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;

            tx.commit();
            Ok(())
        })
    }

    /// Update content and binary data of an existing document
    pub fn update_document_content_with_bytes<T>(
        handle: &DocHandle,
        content: T,
        bytes: Bytes,
    ) -> Result<()>
    where
        T: serde::Serialize,
    {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();

            // Convert content to JSON value and store as native Automerge objects
            let json_value =
                serde_json::to_value(&content).map_err(VfsError::SerializationError)?;

            // Delete old content if it exists
            let _ = tx.delete(automerge::ROOT, "content");

            // Create new content object based on JSON type
            match &json_value {
                serde_json::Value::Object(map) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    for (k, v) in map {
                        Self::put_json_value(&mut tx, content_obj.clone(), k, v)?;
                    }
                }
                serde_json::Value::Array(arr) => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::List)?;
                    for (i, item) in arr.iter().enumerate() {
                        Self::insert_json_value(&mut tx, content_obj.clone(), i, item)?;
                    }
                }
                _ => {
                    let content_obj = tx.put_object(automerge::ROOT, "content", ObjType::Map)?;
                    Self::put_json_value(&mut tx, content_obj, "value", &json_value)?;
                }
            }

            // Update binary data
            let bytes_scalar = ScalarValue::Bytes(bytes.to_vec());
            tx.put(automerge::ROOT, "bytes", bytes_scalar)?;

            // Update modified timestamp
            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;

            tx.commit();
            Ok(())
        })
    }

    /// Patch a document at a specific path (incremental update)
    /// path: ["content", "x"] -> updates doc.content.x
    pub fn patch_document(
        handle: &DocHandle,
        path: &[String],
        value: serde_json::Value,
    ) -> Result<()> {
        if path.is_empty() {
            return Err(VfsError::Other(anyhow::anyhow!("Path cannot be empty")));
        }

        handle.with_document(|doc| {
            // Navigate to parent BEFORE creating transaction (borrow checker)
            let (parent_obj, final_key) = Self::navigate_to_parent(doc, path)?;

            // Now create the transaction
            let mut tx = doc.transaction();

            // Update the value at the path
            Self::put_json_value(&mut tx, parent_obj, &final_key, &value)?;

            // Update modified timestamp
            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;

            tx.commit();
            Ok(())
        })
    }

    /// Splice text at a specific path within a document
    /// Uses Automerge's Text CRDT for character-level collaborative editing
    pub fn splice_text(
        handle: &DocHandle,
        path: &[String],
        index: usize,
        delete_count: isize,
        insert: &str,
    ) -> Result<()> {
        if path.is_empty() {
            return Err(VfsError::Other(anyhow::anyhow!("Path cannot be empty")));
        }

        handle.with_document(|doc| {
            // Navigate to parent BEFORE creating transaction (borrow checker)
            let (parent_obj, final_key) = Self::navigate_to_parent(doc, path)?;

            // Now create the transaction
            let mut tx = doc.transaction();

            // Check if text object exists at this path
            let text_obj = match tx.get(parent_obj.clone(), final_key.as_str()) {
                Ok(Some((Value::Object(ObjType::Text), obj_id))) => obj_id,
                Ok(Some((Value::Scalar(scalar), _))) => {
                    // Extract existing string content if it's a string scalar
                    let existing_content = match scalar.as_ref() {
                        ScalarValue::Str(s) => s.to_string(),
                        _ => String::new(),
                    };
                    // Create a new Text object and initialize with existing content
                    let text_obj = tx.put_object(parent_obj, final_key.as_str(), ObjType::Text)?;
                    if !existing_content.is_empty() {
                        tx.splice_text(text_obj.clone(), 0, 0, &existing_content)
                            .map_err(VfsError::AutomergeError)?;
                    }
                    text_obj
                }
                Ok(None) => {
                    // Create a new empty Text object
                    tx.put_object(parent_obj, final_key.as_str(), ObjType::Text)?
                }
                Ok(Some((Value::Object(_), _))) => {
                    return Err(VfsError::Other(anyhow::anyhow!(
                        "Path '{}' is an object, not text",
                        final_key
                    )));
                }
                Err(e) => return Err(VfsError::AutomergeError(e)),
            };

            // Perform the splice operation
            tx.splice_text(text_obj, index, delete_count, insert)
                .map_err(VfsError::AutomergeError)?;

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

    /// Update the name field of a document or directory
    pub fn update_document_name(handle: &DocHandle, new_name: &str) -> Result<()> {
        handle.with_document(|doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "name", new_name)?;
            Self::update_modified_timestamp(&mut tx, automerge::ROOT)?;
            tx.commit();
            Ok(())
        })
    }
}
