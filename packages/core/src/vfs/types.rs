use chrono::{DateTime, Utc};
use samod::DocumentId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeType {
    #[serde(rename = "document")]
    Document,
    #[serde(rename = "directory")]
    Directory,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Timestamps {
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
}

impl Serialize for Timestamps {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Timestamps", 2)?;
        state.serialize_field("created", &self.created.timestamp_millis())?;
        state.serialize_field("modified", &self.modified.timestamp_millis())?;
        state.end()
    }
}

impl Timestamps {
    pub fn now() -> Self {
        let now = Utc::now();
        Self {
            created: now,
            modified: now,
        }
    }

    pub fn update_modified(&mut self) {
        self.modified = Utc::now();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefNode {
    pub pointer: DocumentId,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub timestamps: Timestamps,
    pub name: String,
}

impl RefNode {
    pub fn new_document(name: String, pointer: DocumentId) -> Self {
        Self {
            pointer,
            node_type: NodeType::Document,
            timestamps: Timestamps::now(),
            name,
        }
    }

    pub fn new_directory(name: String, pointer: DocumentId) -> Self {
        Self {
            pointer,
            node_type: NodeType::Directory,
            timestamps: Timestamps::now(),
            name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirNode {
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub name: String,
    pub timestamps: Timestamps,
    pub children: Vec<RefNode>,
}

impl DirNode {
    pub fn new(name: String) -> Self {
        Self {
            node_type: NodeType::Directory,
            name,
            timestamps: Timestamps::now(),
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, child: RefNode) {
        if !self.children.iter().any(|c| c.name == child.name) {
            self.children.push(child);
            self.timestamps.update_modified();
        }
    }

    pub fn remove_child(&mut self, name: &str) -> Option<RefNode> {
        if let Some(pos) = self.children.iter().position(|c| c.name == name) {
            self.timestamps.update_modified();
            Some(self.children.remove(pos))
        } else {
            None
        }
    }

    pub fn find_child(&self, name: &str) -> Option<&RefNode> {
        self.children.iter().find(|c| c.name == name)
    }

    pub fn find_child_mut(&mut self, name: &str) -> Option<&mut RefNode> {
        self.children.iter_mut().find(|c| c.name == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocNode<T> {
    #[serde(rename = "type")]
    pub node_type: NodeType,
    pub name: String,
    pub timestamps: Timestamps,
    pub content: T,
    pub bytes: Option<Vec<u8>>,
}

impl<T> DocNode<T> {
    pub fn new(name: String, content: T, bytes: Option<Vec<u8>>) -> Self {
        Self {
            node_type: NodeType::Document,
            name,
            timestamps: Timestamps::now(),
            content,
            bytes,
        }
    }

    pub fn update_content(&mut self, content: T) {
        self.content = content;
        self.timestamps.update_modified();
    }
}
