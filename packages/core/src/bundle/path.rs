use std::fmt;

/// A type-safe wrapper for bundle paths that ensures consistent path handling.
/// 
/// BundlePath provides a safe interface for working with file paths in ZIP bundles,
/// automatically handling path normalization and component parsing.
/// 
/// # Examples
/// 
/// ```
/// # use tonk_core::bundle::path::BundlePath;
/// let path = BundlePath::from_str("/documents/readme.txt");
/// assert_eq!(path.filename(), Some("readme.txt"));
/// assert_eq!(path.parent().unwrap().to_string(), "documents");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BundlePath(Vec<String>);

impl BundlePath {
    /// Create a new bundle path from components.
    /// 
    /// Empty components are automatically filtered out to ensure path consistency.
    /// 
    /// # Arguments
    /// * `components` - Vector of path components
    /// 
    /// # Examples
    /// ```
    /// # use tonk_core::bundle::path::BundlePath;
    /// let path = BundlePath::new(vec!["docs".to_string(), "file.txt".to_string()]);
    /// assert_eq!(path.to_string(), "docs/file.txt");
    /// ```
    pub fn new(components: Vec<String>) -> Self {
        Self(components.into_iter().filter(|s| !s.is_empty()).collect())
    }

    /// Create a bundle path from a slash-separated string
    pub fn from_str(path: &str) -> Self {
        if path.is_empty() || path == "/" {
            return Self::root();
        }
        
        let components: Vec<String> = path
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        
        Self(components)
    }

    /// Create a root path (empty components)
    pub fn root() -> Self {
        Self(Vec::new())
    }

    /// Get the components as a slice
    pub fn components(&self) -> &[String] {
        &self.0
    }

    /// Convert to a slash-separated string
    pub fn to_string(&self) -> String {
        if self.0.is_empty() {
            String::new()
        } else {
            self.0.join("/")
        }
    }

    /// Check if this is the root path
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// Get the number of path components
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Check if the path is empty (same as is_root)
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get the last component (filename)
    pub fn filename(&self) -> Option<&str> {
        self.0.last().map(|s| s.as_str())
    }

    /// Get the parent path
    pub fn parent(&self) -> Option<BundlePath> {
        if self.0.is_empty() {
            None
        } else if self.0.len() == 1 {
            Some(Self::root())
        } else {
            Some(Self(self.0[..self.0.len() - 1].to_vec()))
        }
    }

    /// Check if this path starts with another path
    pub fn starts_with(&self, prefix: &BundlePath) -> bool {
        if prefix.0.len() > self.0.len() {
            return false;
        }
        
        self.0[..prefix.0.len()] == prefix.0
    }

    /// Create a child path by appending a component
    pub fn child(&self, component: &str) -> BundlePath {
        let mut components = self.0.clone();
        components.push(component.to_string());
        Self(components)
    }
}

impl fmt::Display for BundlePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl From<Vec<String>> for BundlePath {
    fn from(components: Vec<String>) -> Self {
        Self::new(components)
    }
}

impl From<&[String]> for BundlePath {
    fn from(components: &[String]) -> Self {
        Self::new(components.to_vec())
    }
}

impl From<&str> for BundlePath {
    fn from(path: &str) -> Self {
        Self::from_str(path)
    }
}

impl Into<Vec<String>> for BundlePath {
    fn into(self) -> Vec<String> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundle_path_creation() {
        let path = BundlePath::from_str("/foo/bar/baz");
        assert_eq!(path.components(), &["foo", "bar", "baz"]);
        assert_eq!(path.to_string(), "foo/bar/baz");
    }

    #[test]
    fn test_root_path() {
        let root1 = BundlePath::root();
        let root2 = BundlePath::from_str("/");
        let root3 = BundlePath::from_str("");
        
        assert!(root1.is_root());
        assert!(root2.is_root());
        assert!(root3.is_root());
        assert_eq!(root1, root2);
        assert_eq!(root2, root3);
    }

    #[test]
    fn test_path_operations() {
        let path = BundlePath::from_str("/docs/readme.txt");
        
        assert_eq!(path.filename(), Some("readme.txt"));
        assert_eq!(path.parent(), Some(BundlePath::from_str("/docs")));
        assert_eq!(path.len(), 2);
        
        let child = path.child("backup");
        assert_eq!(child.to_string(), "docs/readme.txt/backup");
    }

    #[test]
    fn test_path_prefix() {
        let path = BundlePath::from_str("/docs/readme.txt");
        let prefix = BundlePath::from_str("/docs");
        let non_prefix = BundlePath::from_str("/src");
        
        assert!(path.starts_with(&prefix));
        assert!(!path.starts_with(&non_prefix));
        assert!(path.starts_with(&BundlePath::root()));
    }
}