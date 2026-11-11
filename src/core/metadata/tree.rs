//! Bounded, read-only presentation of embedded JSON. Built on the inspector's
//! worker, never by the paint loop. Values are owned once, rather than storing
//! a serialized copy of every ancestor in a large ComfyUI graph.
use crate::core::metadata::Metadata;
use serde_json::Value;

pub(crate) const VALUE_CHUNK_BYTES: usize = 4096;
const JSON_NODE_LIMIT: usize = 16_384;
const JSON_DEPTH_LIMIT: usize = 32;
const JSON_STRING_LAYERS: usize = 4;

#[derive(Debug)]
enum Kind {
    Text(String),
    Scalar(Value),
    Object,
    Array,
}

#[derive(Debug)]
pub(crate) struct Node {
    pub key: String,
    kind: Kind,
    pub children: Vec<usize>,
    // A field's original JSON, or a JSON document encoded inside a JSON string.
    // Keeping this also preserves the string type when copying its parent.
    original: Option<String>,
}

impl Node {
    fn value_text(&self) -> std::borrow::Cow<'_, str> {
        match &self.kind {
            Kind::Text(text) => text.as_str().into(),
            Kind::Scalar(value) => value.to_string().into(),
            Kind::Object if self.children.is_empty() => "{}".into(),
            Kind::Array if self.children.is_empty() => "[]".into(),
            Kind::Object => format!("{{…}} ({})", self.children.len()).into(),
            Kind::Array => format!("[…] ({})", self.children.len()).into(),
        }
    }

    // `query` is already lowercased. Containers match by key only; their
    // values are matched through their children.
    fn matches(&self, query: &str) -> bool {
        let contains = |text: &str| text.to_lowercase().contains(query);
        contains(&self.key)
            || match &self.kind {
                Kind::Text(text) => contains(text),
                Kind::Scalar(value) => contains(&value.to_string()),
                Kind::Object | Kind::Array => false,
            }
    }

    fn chunks(&self) -> usize {
        match &self.kind {
            Kind::Text(text) => text.len().div_ceil(VALUE_CHUNK_BYTES).max(1),
            _ => 1,
        }
    }

    pub fn label(&self, chunk: usize, nested: bool) -> String {
        // Continuations flow directly beneath the preceding text instead of
        // repeating the metadata key at each internal layout boundary.
        if chunk > 0 {
            return value_chunk(&self.value_text(), chunk).to_owned();
        }
        let key = value_chunk(&self.key, 0);
        let suffix = if key.len() < self.key.len() {
            "…"
        } else {
            ""
        };
        let value = self.value_text();
        let separator = if nested { ": " } else { "  ·  " };
        if value.is_empty() {
            format!("{key}{suffix}{separator}\"\"")
        } else if value.len() <= 80 && !value.contains('\n') {
            format!("{key}{suffix}{separator}{value}")
        } else {
            format!("{key}{suffix}\n{}", value_chunk(&value, chunk))
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Row {
    pub node: usize,
    pub chunk: usize,
    pub depth: usize,
}

#[derive(Default)]
pub(crate) struct Tree {
    pub nodes: Vec<Node>,
    roots: Vec<usize>,
    json_nodes: usize,
    pub limited: bool,
}

impl Tree {
    pub fn new(report: &Metadata) -> Self {
        let mut tree = Self::default();
        let mut order: Vec<_> = report
            .fields
            .iter()
            .filter(|(key, _)| {
                !matches!(
                    key.as_str(),
                    "File name"
                        | "Path"
                        | "Extension"
                        | "Dimensions"
                        | "Format"
                        | "File size"
                        | "Frame count"
                )
            })
            .collect();
        order.sort_by_key(|(key, _)| {
            let key = key.to_lowercase();
            ![
                "parameters",
                "prompt",
                "workflow",
                "usercomment",
                "user comment",
                "generation data",
            ]
            .iter()
            .any(|name| key.contains(name))
        });
        for (key, text) in order {
            let root = tree.add(key.clone(), Value::String(text.clone()), 0);
            tree.roots.push(root);
        }
        tree
    }

    fn add(&mut self, mut key: String, mut value: Value, depth: usize) -> usize {
        let mut original = None;
        if let Value::String(text) = &value
            && let Some((parsed, prefix)) = json_document(text, depth == 0)
        {
            if let Some(count) = json_size(&parsed, depth, JSON_NODE_LIMIT - self.json_nodes) {
                self.json_nodes += count;
                if let Some(prefix) = prefix {
                    key.push_str(" · ");
                    key.push_str(prefix);
                }
                let Value::String(text) = std::mem::take(&mut value) else {
                    unreachable!()
                };
                original = Some(text);
                value = parsed;
            } else {
                self.limited = true;
            }
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            key,
            kind: Kind::Object,
            children: Vec::new(),
            original,
        });
        let (kind, children) = match value {
            Value::Object(values) => (
                Kind::Object,
                values
                    .into_iter()
                    .map(|(key, value)| self.add(key, value, depth + 1))
                    .collect(),
            ),
            Value::Array(values) => (
                Kind::Array,
                values
                    .into_iter()
                    .enumerate()
                    .map(|(i, value)| self.add(format!("[{i}]"), value, depth + 1))
                    .collect(),
            ),
            Value::String(text) => (Kind::Text(text), Vec::new()),
            value => (Kind::Scalar(value), Vec::new()),
        };
        self.nodes[id].kind = kind;
        self.nodes[id].children = children;
        id
    }

    pub fn copy_text(&self, id: usize) -> String {
        let node = &self.nodes[id];
        if let Some(original) = &node.original {
            return original.clone();
        }
        if let Kind::Text(text) = &node.kind {
            return text.clone();
        }
        serde_json::to_string_pretty(&self.json_value(id)).expect("metadata JSON value")
    }

    fn json_value(&self, id: usize) -> Value {
        let node = &self.nodes[id];
        if let Some(original) = &node.original {
            return Value::String(original.clone());
        }
        match &node.kind {
            Kind::Text(text) => Value::String(text.clone()),
            Kind::Scalar(value) => value.clone(),
            Kind::Object => Value::Object(
                node.children
                    .iter()
                    .map(|&id| (self.nodes[id].key.clone(), self.json_value(id)))
                    .collect(),
            ),
            Kind::Array => Value::Array(
                node.children
                    .iter()
                    .map(|&id| self.json_value(id))
                    .collect(),
            ),
        }
    }

    // One continuous list. The UI virtualizes rows; long values are split only
    // internally to bound each text layout job, never into user-facing pages.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for &root in &self.roots {
            self.push_subtree(root, 0, &mut rows);
        }
        rows
    }

    /// Rows whose key or value contains `query`, ignoring case. Ancestors stay
    /// as the path to a match, and a matching branch keeps all its descendants.
    pub fn filtered_rows(&self, query: &str) -> Vec<Row> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return self.rows();
        }
        let mut rows = Vec::new();
        for &root in &self.roots {
            self.push_matches(root, 0, &query, &mut rows);
        }
        rows
    }

    fn push_matches(&self, id: usize, depth: usize, query: &str, rows: &mut Vec<Row>) -> bool {
        let node = &self.nodes[id];
        if node.matches(query) {
            self.push_subtree(id, depth, rows);
            return true;
        }
        let start = rows.len();
        rows.push(Row {
            node: id,
            chunk: 0,
            depth,
        });
        let mut found = false;
        for &child in &node.children {
            found |= self.push_matches(child, depth + 1, query, rows);
        }
        if !found {
            rows.truncate(start);
        }
        found
    }

    fn push_subtree(&self, id: usize, depth: usize, rows: &mut Vec<Row>) {
        let mut stack = vec![(id, depth)];
        while let Some((id, depth)) = stack.pop() {
            let node = &self.nodes[id];
            for chunk in 0..node.chunks() {
                rows.push(Row {
                    node: id,
                    chunk,
                    depth,
                });
            }
            stack.extend(node.children.iter().rev().map(|&id| (id, depth + 1)));
        }
    }
}

// ComfyUI's WebP/AVIF EXIF fields contain prompt:JSON or workflow:JSON.
// Recognize those field envelopes without interpreting arbitrary prose as JSON.
// Nested serialized JSON strings use the same bounded decoder. The caller keeps
// the untouched original for copying, including prefixes and string escaping.
fn json_document(text: &str, field: bool) -> Option<(Value, Option<&'static str>)> {
    let mut text = std::borrow::Cow::Borrowed(text);
    let mut prefix = None;
    for _ in 0..=JSON_STRING_LAYERS {
        let mut document = text.trim().trim_start_matches('\u{feff}').trim_start();
        if field
            && prefix.is_none()
            && let Some((name, payload)) = document.split_once(':')
            && let Some(name) = ["prompt", "workflow"]
                .into_iter()
                .find(|known| name.trim().eq_ignore_ascii_case(known))
        {
            prefix = Some(name);
            document = payload.trim();
        }
        if !document.starts_with(['{', '[', '"']) {
            return None;
        }
        match serde_json::from_str::<Value>(document).ok()? {
            value @ (Value::Object(_) | Value::Array(_)) => return Some((value, prefix)),
            Value::String(inner) => text = std::borrow::Cow::Owned(inner),
            _ => return None,
        }
    }
    None
}

// serde_json keeps its recursion limit enabled. Inspect the parsed document
// before retaining/expanding it; over-budget documents remain copyable text.
fn json_size(value: &Value, depth: usize, remaining: usize) -> Option<usize> {
    if depth > JSON_DEPTH_LIMIT || remaining == 0 {
        return None;
    }
    let mut count = 1;
    match value {
        Value::Object(values) => {
            for value in values.values() {
                count += json_size(value, depth + 1, remaining.checked_sub(count)?)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                count += json_size(value, depth + 1, remaining.checked_sub(count)?)?;
            }
        }
        _ => {}
    }
    Some(count)
}

pub(crate) fn value_chunk(value: &str, page: usize) -> &str {
    let boundary = |n: usize| {
        let mut i = n.saturating_mul(VALUE_CHUNK_BYTES).min(value.len());
        while !value.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    &value[boundary(page)..boundary(page.saturating_add(1))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(text: &str) -> Tree {
        let mut report = Metadata::default();
        report.add("PNG tEXt · prompt", text);
        Tree::new(&report)
    }

    fn child(tree: &Tree, id: usize, key: &str) -> usize {
        *tree.nodes[id]
            .children
            .iter()
            .find(|&&id| tree.nodes[id].key == key)
            .unwrap()
    }

    #[test]
    fn comfyui_fields_copy_values_and_branches_without_losing_types() {
        let original = r#"{"1":{"class_type":"KSampler","inputs":{"seed":18446744073709551615,"large":123456789012345678901234567890,"precise":0.1234567890123456789,"cfg":6.5,"enabled":true,"optional":null,"text":"별밤\n🌙","model":["4",0],"empty":{}}}}"#;
        let tree = report(original);
        let root = tree.roots[0];
        let sampler = child(&tree, root, "1");
        let inputs = child(&tree, sampler, "inputs");
        assert_eq!(tree.copy_text(root), original);
        for (key, expected) in [
            ("seed", "18446744073709551615"),
            ("large", "123456789012345678901234567890"),
            ("precise", "0.1234567890123456789"),
            ("cfg", "6.5"),
            ("enabled", "true"),
            ("optional", "null"),
            ("text", "별밤\n🌙"),
            ("empty", "{}"),
        ] {
            assert_eq!(tree.copy_text(child(&tree, inputs, key)), expected);
        }
        let model = child(&tree, inputs, "model");
        assert_eq!(tree.copy_text(child(&tree, model, "[0]")), "4");
        assert_eq!(tree.copy_text(child(&tree, model, "[1]")), "0");
        let copied: Value = serde_json::from_str(&tree.copy_text(inputs)).unwrap();
        let original: Value = serde_json::from_str(original).unwrap();
        assert_eq!(copied, original["1"]["inputs"]);
    }

    #[test]
    fn embedded_json_strings_expand_but_keep_their_type_in_parent_copies() {
        let value =
            serde_json::json!({"node":{"extra":"{\"steps\":20}","plain":"[not JSON]","empty":""}});
        let tree = report(&value.to_string());
        let node = child(&tree, tree.roots[0], "node");
        let extra = child(&tree, node, "extra");
        assert_eq!(tree.copy_text(child(&tree, extra, "steps")), "20");
        assert_eq!(tree.copy_text(extra), "{\"steps\":20}");
        assert_eq!(tree.copy_text(child(&tree, node, "empty")), "");
        assert_eq!(tree.copy_text(child(&tree, node, "plain")), "[not JSON]");
        assert_eq!(
            serde_json::from_str::<Value>(&tree.copy_text(node)).unwrap(),
            value["node"]
        );
    }

    #[test]
    fn comfyui_exif_prefixes_expand_and_preserve_original_copies() {
        // ComfyUI stores workflow:JSON in Make and prompt:JSON in Model.
        let workflow = r#"workflow:{"nodes":[{"id":1}]}"#;
        let prompt =
            r#"prompt:{"1":{"inputs":{"text":"\ubcc4\ubc24","seed":18446744073709551615}}}"#;
        let mut exif = b"II\x2a\0\x08\0\0\0\x02\0".to_vec();
        let mut offset = 38u32;
        for (tag, text) in [(0x010fu16, workflow), (0x0110, prompt)] {
            exif.extend(tag.to_le_bytes());
            exif.extend(2u16.to_le_bytes()); // TIFF ASCII, including the trailing NUL.
            exif.extend((text.len() as u32 + 1).to_le_bytes());
            exif.extend(offset.to_le_bytes());
            offset += text.len() as u32 + 1;
        }
        exif.extend([0; 4]);
        for text in [workflow, prompt] {
            exif.extend(text.as_bytes());
            exif.push(0);
        }
        let mut metadata = Metadata::default();
        crate::core::metadata::tags::exif(&exif, &mut metadata);
        let tree = Tree::new(&metadata);
        let root = *tree
            .roots
            .iter()
            .find(|&&id| tree.nodes[id].key.contains("Model"))
            .unwrap();
        let node = child(&tree, root, "1");
        let inputs = child(&tree, node, "inputs");
        let text = child(&tree, inputs, "text");
        assert!(tree.nodes[root].key.ends_with(" · prompt"));
        assert_eq!(tree.copy_text(root), prompt);
        assert_eq!(tree.copy_text(text), "별밤");
        assert_eq!(
            tree.copy_text(child(&tree, inputs, "seed")),
            "18446744073709551615"
        );
        assert_eq!(tree.nodes[text].label(0, true), "text: 별밤");
        assert!(
            tree.rows()
                .iter()
                .any(|row| row.node == text && row.depth == 3)
        );
        let root = *tree
            .roots
            .iter()
            .find(|&&id| tree.nodes[id].key.contains("Make"))
            .unwrap();
        assert_eq!(tree.copy_text(root), workflow);
        let nodes = child(&tree, root, "nodes");
        assert_eq!(
            tree.copy_text(child(&tree, child(&tree, nodes, "[0]"), "id")),
            "1"
        );
    }

    #[test]
    fn quoted_json_expands_without_rewriting_parent_string_values() {
        let inner = r#"{"steps":20}"#;
        let encoded = serde_json::to_string(inner).unwrap();
        let value = serde_json::json!({"extra": encoded});
        let tree = report(&value.to_string());
        let root = tree.roots[0];
        let extra = child(&tree, root, "extra");
        assert_eq!(tree.copy_text(child(&tree, extra, "steps")), "20");
        assert_eq!(tree.copy_text(extra), encoded);
        assert_eq!(tree.json_value(extra), value["extra"]);
        let prefixed = format!("\u{feff} prompt: {encoded}");
        let tree = report(&prefixed);
        assert_eq!(tree.copy_text(tree.roots[0]), prefixed);
        assert_eq!(tree.copy_text(child(&tree, tree.roots[0], "steps")), "20");
    }

    #[test]
    fn malformed_and_over_budget_json_stays_readable_and_copyable() {
        for value in [
            "plain prompt".to_owned(),
            "{\"truncated\":".to_owned(),
            "prompt: plain prompt".to_owned(),
            "prompt: {\"truncated\":".to_owned(),
            "other: {\"steps\":20}".to_owned(),
            "prompt: 42".to_owned(),
            format!(
                "{}0{}",
                "[".repeat(JSON_DEPTH_LIMIT + 2),
                "]".repeat(JSON_DEPTH_LIMIT + 2)
            ),
            format!("[{}0]", "0,".repeat(JSON_NODE_LIMIT)),
            format!("prompt:[{}0]", "0,".repeat(JSON_NODE_LIMIT)),
        ] {
            let tree = report(&value);
            assert_eq!(tree.nodes.len(), 1);
            assert_eq!(tree.copy_text(tree.roots[0]), value);
        }
    }

    #[test]
    fn duplicate_fields_keep_all_their_children_in_a_continuous_list() {
        let mut metadata = Metadata::default();
        let value = serde_json::json!({"nodes": (0..60).map(|n| serde_json::json!({"seed":n})).collect::<Vec<_>>()}).to_string();
        metadata.add("workflow", &value);
        metadata.add("workflow", &value);
        let tree = Tree::new(&metadata);
        assert_ne!(tree.roots[0], tree.roots[1]);
        assert_eq!(
            tree.rows().len(),
            244,
            "Every nested seed is always visible"
        );
        assert_eq!(tree.copy_text(tree.roots[0]), value);
        assert_eq!(tree.copy_text(tree.roots[1]), value);
    }

    #[test]
    fn filter_matches_keys_or_values_and_keeps_the_path_to_each_match() {
        let mut metadata = Metadata::default();
        metadata.add("Software", "ComfyUI");
        metadata.add(
            "prompt",
            serde_json::json!({
                "3": {"class_type": "KSampler", "inputs": {"seed": 314159, "cfg": 6.5}},
                "6": {"class_type": "CLIPTextEncode", "inputs": {"text": "Starry Night"}}
            })
            .to_string(),
        );
        let tree = Tree::new(&metadata);
        let shown = |query: &str| {
            tree.filtered_rows(query)
                .iter()
                .map(|row| (tree.nodes[row.node].key.as_str(), row.depth))
                .collect::<Vec<_>>()
        };
        assert_eq!(tree.filtered_rows("").len(), tree.rows().len());
        assert_eq!(tree.filtered_rows("   ").len(), tree.rows().len());
        // A value match keeps its ancestors but not its siblings.
        assert_eq!(
            shown("  starry "),
            [("prompt", 0), ("6", 1), ("inputs", 2), ("text", 3)]
        );
        // A key match keeps its whole branch.
        assert_eq!(
            shown("INPUTS"),
            [
                ("prompt", 0),
                ("3", 1),
                ("inputs", 2),
                ("cfg", 3),
                ("seed", 3),
                ("6", 1),
                ("inputs", 2),
                ("text", 3),
            ]
        );
        assert_eq!(
            shown("1415"),
            [("prompt", 0), ("3", 1), ("inputs", 2), ("seed", 3)]
        );
        assert_eq!(shown("comfy"), [("Software", 0)]);
        // Container summaries such as "{…} (2)" are not values.
        assert!(shown("…").is_empty());
        assert!(shown("missing").is_empty());
    }

    #[test]
    fn long_unicode_leaves_can_be_read_in_full_and_copy_in_one_click() {
        let text = "별밤🌙".repeat(3000);
        let tree = report(&serde_json::json!({"text":text}).to_string());
        let id = child(&tree, tree.roots[0], "text");
        let chunks: String = tree
            .rows()
            .iter()
            .filter(|row| row.node == id)
            .map(|row| value_chunk(&text, row.chunk))
            .collect();
        assert_eq!(chunks, text);
        assert_eq!(tree.copy_text(id), text);
    }
}
