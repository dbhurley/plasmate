use plasmate::cache::store::{CacheConfig, CacheLookup, SomCache};
use plasmate::js::pipeline::{process_page, PipelineConfig};
use plasmate::webmcp::{
    validate_invocation_input, InvocationAvailability, TrustClassification, WebMcpCatalog,
    WebMcpToolKind,
};
use serde_json::json;

#[test]
fn declarative_fixture_is_discovered_alongside_som() {
    let html = include_str!("fixtures/webmcp_declarative.html");
    let page = process_page(
        html,
        "https://shop.example/support",
        &PipelineConfig::default(),
    )
    .unwrap();

    assert!(!page.som.regions.is_empty());
    assert_eq!(page.webmcp.contract_version, "plasmate.webmcp.v1");
    assert_eq!(page.webmcp.tools.len(), 1);
    let tool = &page.webmcp.tools[0];
    assert_eq!(tool.name, "createSupportRequest");
    assert_eq!(tool.source.kind, WebMcpToolKind::DeclarativeForm);
    assert_eq!(tool.availability, InvocationAvailability::DiscoveryOnly);
    assert!(tool.confirmation.required);
    assert!(
        validate_invocation_input(tool, &json!({"customerName":"Ari","team":"technical"})).is_ok()
    );
    assert!(
        validate_invocation_input(tool, &json!({"customerName":"Ari","team":"unknown"})).is_err()
    );
}

#[test]
fn imperative_fixture_captures_metadata_but_not_callback() {
    let html = include_str!("fixtures/webmcp_imperative.html");
    let page = process_page(
        html,
        "https://shop.example/orders",
        &PipelineConfig::default(),
    )
    .unwrap();

    assert_eq!(page.webmcp.tools.len(), 2);
    let read = page
        .webmcp
        .tools
        .iter()
        .find(|tool| tool.name == "getOrderStatus")
        .unwrap();
    assert_eq!(read.source.kind, WebMcpToolKind::Imperative);
    assert_eq!(read.annotations.read_only_hint, Some(true));
    assert!(!read.confirmation.required);
    assert_eq!(
        read.metadata_trust,
        TrustClassification::UntrustedWebContent
    );
    assert!(read.availability_reason.contains("does not retain"));
    assert!(read.output_schema.is_some());

    let mutation = page
        .webmcp
        .tools
        .iter()
        .find(|tool| tool.name == "cancelOrder")
        .unwrap();
    assert!(mutation.confirmation.required);

    // Discovery must never run the callback as a side effect.
    assert!(page.effective_html.contains(">Ready<"));
    assert!(!page.effective_html.contains(">Cancelled<"));
}

#[test]
fn imperative_catalog_survives_page_state_cache_round_trip() {
    let html = include_str!("fixtures/webmcp_imperative.html");
    let url = "https://shop.example/orders";
    let page = process_page(html, url, &PipelineConfig::default()).unwrap();
    let catalog_json = serde_json::to_vec(&page.webmcp).unwrap();
    let som_json = serde_json::to_vec(&page.som).unwrap();
    let content_hash = SomCache::content_hash(html.as_bytes());
    let cache = SomCache::new(CacheConfig::default());

    cache.store_page_state_with_webmcp(
        url,
        content_hash,
        som_json,
        html.len(),
        page.effective_html,
        catalog_json,
    );

    let entry = match cache.lookup(url, content_hash) {
        CacheLookup::Hit(entry) => entry,
        other => panic!("expected exact page-state cache hit, got {other:?}"),
    };
    let restored: WebMcpCatalog =
        serde_json::from_slice(entry.webmcp_json.as_deref().unwrap()).unwrap();

    assert!(restored
        .tools
        .iter()
        .any(|tool| tool.name == "getOrderStatus"));
    assert!(restored.tools.iter().any(|tool| tool.name == "cancelOrder"));
}

#[test]
fn page_document_domain_disables_both_api_shapes() {
    let html = r#"
      <form toolname="submit" tooldescription="Submit"><input name="x"></form>
      <script>
        document.domain = "example.com";
        document.modelContext.registerTool({
          name: "read", description: "Read", execute: () => "value"
        });
      </script>
    "#;
    let page = process_page(html, "https://app.example.com", &PipelineConfig::default()).unwrap();
    assert!(page.webmcp.tools.is_empty());
    assert!(page
        .webmcp
        .warnings
        .iter()
        .any(|warning| warning.contains("document.domain")));
}

#[test]
fn nested_cross_origin_markup_is_not_discovered_as_a_tool_source() {
    let html = r#"
      <iframe src="https://partner.example/tool" allow="tools"
        srcdoc='<form toolname="framed" tooldescription="Framed"></form>'></iframe>
      <form toolname="top" tooldescription="Top"></form>
    "#;
    let page = process_page(html, "https://app.example.com", &PipelineConfig::default()).unwrap();
    assert_eq!(page.webmcp.tools.len(), 1);
    assert_eq!(page.webmcp.tools[0].name, "top");
    assert_eq!(page.webmcp.tools[0].source.frame, "top");
}
