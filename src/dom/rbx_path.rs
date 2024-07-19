use std::{fmt, path::PathBuf};

#[derive(Clone)]
pub struct DotPath {
    root: String,
    components: Vec<String>,
}

impl Default for DotPath {
    fn default() -> Self {
        DotPath {
            root: String::from("script"),
            components: Vec::new(),
        }
    }
}

impl DotPath {
    // TODO: implement parsing of path, with support for functions and other identifiers
    // fn new(path: &str) -> Self {
    //     DotPath {
    //         components: path.split('.').map(String::from).collect(),
    //         ..Default::default()
    //     }
    // }

    /// Creates a path that starts from depth and leads to the ancestor at depth 0
    ///
    /// # Example
    ///
    /// ```rust
    /// use pluginproxy_transpiler::dom::rbx_path::DotPath;
    ///
    /// DotPath::new_ancestor_path(2);
    /// assert_eq!(DotPath::new_ancestor_path(2).to_string(), "script.Parent.Parent");
    /// assert_eq!(DotPath::new_ancestor_path(0).to_string(), "script");
    /// ```
    pub fn new_ancestor_path(depth: usize) -> Self {
        DotPath {
            components: (0..depth).map(|_| String::from("Parent")).collect(),
            ..Default::default()
        }
    }

    pub fn depth(&self) -> usize {
        self.components.len()
    }

    pub fn push(&mut self, component: &str) {
        self.components.push(component.to_string());
    }

    pub fn pop(&mut self) -> Option<String> {
        self.components.pop()
    }

    fn join(&self, separator: &str) -> String {
        format!(
            "{}{}",
            self.root,
            if !self.components.is_empty() {
                format!("{separator}{}", self.components.join(separator))
            } else {
                String::new()
            }
        )
    }

    /// Path string in the format of script/Parent/Parent
    pub fn path_string(&self) -> String {
        format!("{}/", self.join("/"))
    }

    pub fn to_path(&self) -> PathBuf {
        PathBuf::from(self.path_string())
    }
}

impl fmt::Display for DotPath {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.join("."))
    }
}
