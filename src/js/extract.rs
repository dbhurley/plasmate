//! Extract executable classic and module `<script>` blocks from parsed HTML.

use html5ever::parse_document;
use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{Handle, NodeData, RcDom};

/// An extracted script block.
#[derive(Debug, Clone)]
pub struct ScriptBlock {
    /// The script source code.
    pub source: String,
    /// Label for error reporting (e.g. "inline-1", or the src URL).
    pub label: String,
    /// Whether this was an inline script (vs external src).
    pub is_inline: bool,
    /// The script's position in document order.
    pub index: usize,
    /// JavaScript execution semantics requested by the element.
    pub kind: ScriptKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Classic,
    Module,
    ImportMap,
}

/// The exact JavaScript MIME type essence strings defined by WHATWG MIME
/// Sniffing section 4.6. Keep this shared by HTML `type` classification and
/// module response validation so the two standards surfaces cannot drift.
pub(crate) const JAVASCRIPT_MIME_TYPE_ESSENCES: &[&str] = &[
    "application/ecmascript",
    "application/javascript",
    "application/x-ecmascript",
    "application/x-javascript",
    "text/ecmascript",
    "text/javascript",
    "text/javascript1.0",
    "text/javascript1.1",
    "text/javascript1.2",
    "text/javascript1.3",
    "text/javascript1.4",
    "text/javascript1.5",
    "text/jscript",
    "text/livescript",
    "text/x-ecmascript",
    "text/x-javascript",
];

pub(crate) fn is_javascript_mime_essence(value: &str) -> bool {
    JAVASCRIPT_MIME_TYPE_ESSENCES
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

/// Extract all executable script blocks from HTML.
///
/// Skips:
/// - Scripts with type="application/json" or type="application/ld+json"
/// - Scripts with src="" (external; would need fetch, handled separately)
/// - Empty scripts
pub fn extract_scripts(html: &str) -> Vec<ScriptBlock> {
    let dom = parse_document(RcDom::default(), Default::default())
        .from_utf8()
        .read_from(&mut html.as_bytes())
        .unwrap();

    let mut scripts = Vec::new();
    let mut index = 0;
    visit_scripts(&dom.document, &mut scripts, &mut index);
    scripts
}

fn visit_scripts(node: &Handle, scripts: &mut Vec<ScriptBlock>, index: &mut usize) {
    if let NodeData::Element { name, attrs, .. } = &node.data {
        if name.local.as_ref() == "script" {
            let attrs_borrowed = attrs.borrow();
            let script_type = attrs_borrowed
                .iter()
                .find(|a| a.name.local.as_ref() == "type")
                .map(|a| {
                    a.value
                        .trim_matches(|character: char| character.is_ascii_whitespace())
                        .to_ascii_lowercase()
                });
            let has_src = attrs_borrowed
                .iter()
                .any(|a| a.name.local.as_ref() == "src");

            let kind = match script_type.as_deref() {
                Some("module") => ScriptKind::Module,
                Some("importmap") => ScriptKind::ImportMap,
                _ => ScriptKind::Classic,
            };

            // Skip non-executable types.
            let skip = match script_type.as_deref() {
                Some("module") | Some("importmap") => false,
                Some("application/json") | Some("application/ld+json") => true,
                Some("text/html") | Some("text/template") => true,
                Some(t) if !is_javascript_mime_essence(t) && !t.is_empty() => true,
                _ => false,
            };

            if !skip && !has_src {
                // Collect inline text content
                let mut source = String::new();
                collect_script_text(node, &mut source);
                let source = source.trim().to_string();

                if !source.is_empty() {
                    scripts.push(ScriptBlock {
                        source,
                        label: format!("inline-{}", *index),
                        is_inline: true,
                        index: *index,
                        kind,
                    });
                    *index += 1;
                }
            }

            if !skip && has_src {
                let src = attrs_borrowed
                    .iter()
                    .find(|a| a.name.local.as_ref() == "src")
                    .map(|a| a.value.to_string())
                    .unwrap_or_default();
                scripts.push(ScriptBlock {
                    source: String::new(),
                    label: src,
                    is_inline: false,
                    index: *index,
                    kind,
                });
                *index += 1;
            }

            return; // Don't recurse into script contents
        }
    }

    for child in node.children.borrow().iter() {
        visit_scripts(child, scripts, index);
    }
}

fn collect_script_text(node: &Handle, buf: &mut String) {
    match &node.data {
        NodeData::Text { contents } => {
            buf.push_str(&contents.borrow());
        }
        _ => {
            for child in node.children.borrow().iter() {
                collect_script_text(child, buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_inline_script() {
        let html = r#"<html><head><script>var x = 1;</script></head><body></body></html>"#;
        let scripts = extract_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].source, "var x = 1;");
        assert!(scripts[0].is_inline);
    }

    #[test]
    fn test_skip_json_ld() {
        let html = r#"<html><head>
            <script type="application/ld+json">{"@type":"WebPage"}</script>
            <script>var x = 1;</script>
        </head><body></body></html>"#;
        let scripts = extract_scripts(html);
        let inline: Vec<_> = scripts.iter().filter(|s| s.is_inline).collect();
        assert_eq!(inline.len(), 1);
        assert_eq!(inline[0].source, "var x = 1;");
    }

    #[test]
    fn test_extract_inline_module() {
        let html =
            r#"<html><head><script type="module">import x from './x';</script></head></html>"#;
        let scripts = extract_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert!(scripts[0].is_inline);
        assert_eq!(scripts[0].kind, ScriptKind::Module);
    }

    // HTML's script type matching strips ASCII whitespace before comparing the
    // module keyword (mirrors the WPT script-type module cases).
    #[test]
    fn test_module_type_ignores_ascii_whitespace() {
        let scripts =
            extract_scripts("<script type=' module '>globalThis.moduleWhitespace = true;</script>");
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].kind, ScriptKind::Module);
        assert!(extract_scripts(
            "<script type='\u{a0}module\u{a0}'>globalThis.notAModule = true;</script>"
        )
        .is_empty());
    }

    #[test]
    fn test_extract_import_map_for_explicit_diagnostic() {
        let scripts =
            extract_scripts(r#"<script type="importmap">{"imports":{"pkg":"./pkg.js"}}</script>"#);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].kind, ScriptKind::ImportMap);
    }

    #[test]
    fn classic_script_type_accepts_exact_whatwg_javascript_mime_essences() {
        for essence in JAVASCRIPT_MIME_TYPE_ESSENCES {
            let html = format!("<script type='{essence}'>globalThis.acceptedMime = true;</script>");
            let scripts = extract_scripts(&html);
            assert_eq!(scripts.len(), 1, "classic type was rejected: {essence}");
            assert_eq!(scripts[0].kind, ScriptKind::Classic);

            let uppercase = essence.to_ascii_uppercase();
            let html =
                format!("<script type='{uppercase}'>globalThis.acceptedMime = true;</script>");
            assert_eq!(
                extract_scripts(&html).len(),
                1,
                "ASCII-insensitive classic type was rejected: {uppercase}"
            );
        }
    }

    #[test]
    fn classic_script_type_uses_essence_match_not_media_type_parsing() {
        for value in [
            "text/javascript; charset=utf-8",
            "text/javascript1.6",
            "text/javascript+json",
            "application/json",
        ] {
            let html = format!("<script type='{value}'>globalThis.nope = true;</script>");
            assert!(
                extract_scripts(&html).is_empty(),
                "non-essence classic type was accepted: {value}"
            );
        }
    }

    #[test]
    fn test_external_script_noted() {
        let html = r#"<html><head><script src="/app.js"></script></head></html>"#;
        let scripts = extract_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert!(!scripts[0].is_inline);
        assert_eq!(scripts[0].label, "/app.js");
    }

    #[test]
    fn test_multiple_scripts_ordered() {
        let html = r#"<html><body>
            <script>var a = 1;</script>
            <p>content</p>
            <script>var b = 2;</script>
            <script>var c = 3;</script>
        </body></html>"#;
        let scripts = extract_scripts(html);
        let inline: Vec<_> = scripts.iter().filter(|s| s.is_inline).collect();
        assert_eq!(inline.len(), 3);
        assert_eq!(inline[0].index, 0);
        assert_eq!(inline[1].index, 1);
        assert_eq!(inline[2].index, 2);
    }

    #[test]
    fn test_skip_empty_scripts() {
        let html = r#"<html><head><script></script><script>  </script></head></html>"#;
        let scripts = extract_scripts(html);
        let inline: Vec<_> = scripts.iter().filter(|s| s.is_inline).collect();
        assert_eq!(inline.len(), 0);
    }

    #[test]
    fn test_extract_external_module() {
        let html = r#"<html><head><script type="module" src="/app.js"></script></head></html>"#;
        let scripts = extract_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert!(!scripts[0].is_inline);
        assert_eq!(scripts[0].kind, ScriptKind::Module);
    }
}
