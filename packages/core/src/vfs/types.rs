use chrono::{DateTime, Utc};
use samod::DocumentId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeType {
    #[serde(rename = "doc")]
    Document,
    #[serde(rename = "dir")]
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timestamps {
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
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
}

impl<T> DocNode<T> {
    pub fn new(name: String, content: T) -> Self {
        Self {
            node_type: NodeType::Document,
            name,
            timestamps: Timestamps::now(),
            content,
        }
    }

    pub fn update_content(&mut self, content: T) {
        self.content = content;
        self.timestamps.update_modified();
    }
}
