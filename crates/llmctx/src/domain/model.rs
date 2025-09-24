//! Domain models for selections, bundles, and exports.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionItem {
    pub path: std::path::PathBuf,
    pub range: Option<(usize, usize)>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextBundle {
    pub items: Vec<SelectionItem>,
    pub model: Option<String>,
}
