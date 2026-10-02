use super::*;

fn endpoints(
    proxy: &str,
    models_base_url: Option<&str>,
    models_list_url: Option<&str>,
) -> EndpointsConfig {
    EndpointsConfig {
        cli_chat_proxy_base_url: Some(proxy.to_owned()),
        models_base_url: models_base_url.map(|s| s.to_owned()),
        models_list_url: models_list_url.map(|s| s.to_owned()),
        ..Default::default()
    }
}

#[test]
fn inference_url_defaults_to_proxy() {
    let ep = endpoints("https://proxy.codel.dev/v1", None, None);
    assert_eq!(ep.resolve_inference_base_url(), "https://proxy.codel.dev/v1");
}

#[test]
fn inference_url_uses_models_base_url() {
    let ep = endpoints(
        "https://proxy.codel.dev/v1",
        Some("https://enterprise.acme.com/v1"),
        None,
    );
    assert_eq!(
        ep.resolve_inference_base_url(),
        "https://enterprise.acme.com/v1"
    );
}

#[test]
fn inference_url_ignores_models_list_url() {
    let ep = endpoints(
        "https://proxy.codel.dev/v1",
        Some("https://inference.acme.com/v1"),
        Some("https://registry.acme.com/api/models"),
    );
    assert_eq!(
        ep.resolve_inference_base_url(),
        "https://inference.acme.com/v1"
    );
}

#[test]
fn list_url_defaults_to_proxy_models() {
    let ep = endpoints("https://proxy.codel.dev/v1", None, None);
    assert_eq!(
        ep.resolve_models_list_url(),
        "https://proxy.codel.dev/v1/models"
    );
}

#[test]
fn list_url_derived_from_base_url() {
    let ep = endpoints(
        "https://proxy.codel.dev/v1",
        Some("https://api.codel.dev/v1"),
        None,
    );
    assert_eq!(ep.resolve_models_list_url(), "https://api.codel.dev/v1/models");
}

#[test]
fn list_url_explicit_overrides_derivation() {
    let ep = endpoints(
        "https://proxy.codel.dev/v1",
        Some("https://inference.acme.com/v1"),
        Some("https://registry.acme.com/api/list-models"),
    );
