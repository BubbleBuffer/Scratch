//! Contents views live in ordinary blocks. Legacy saved views remain intact
//! while a page's first open adds their block counterparts.

use crate::types::{Block, ViewDef, ViewKind};

/// Helpers for the JSON payload of a `view` block.
pub struct ViewBlock;

fn list_def() -> ViewDef {
    ViewDef {
        id: String::new(),
        name: "List".into(),
        kind: ViewKind::List,
        filter: None,
        sorts: Vec::new(),
        group_by: None,
        date_prop: None,
        visible_props: Vec::new(),
        card_cover: None,
        config: String::new(),
    }
}

impl ViewBlock {
    /// Reads a view without trusting its JSON identity. Missing fields use
    /// List defaults; invalid JSON or invalid field types fall back to List.
    /// The block itself is always the view's identity.
    pub fn def(block: &Block) -> ViewDef {
        let mut fallback = list_def();
        fallback.id = block.id.clone();
        let Ok(serde_json::Value::Object(mut data)) = serde_json::from_str(&block.text) else {
            return fallback;
        };
        data.remove("id");
        let mut defaults = serde_json::to_value(&fallback)
            .expect("ViewDef serializes")
            .as_object()
            .expect("ViewDef is an object")
            .clone();
        defaults.extend(data);
        serde_json::from_value(serde_json::Value::Object(defaults)).unwrap_or(fallback)
    }
}

/// Serializes a view's settings for a contents block. Identity belongs to
/// the block, so it is omitted from the stored payload.
pub fn view_block_text(def: &ViewDef) -> String {
    let mut value = serde_json::to_value(def).expect("ViewDef serializes");
    let object = value.as_object_mut().expect("ViewDef is an object");
    object.remove("id");
    object.insert("source".into(), "contents".into());
    value.to_string()
}
