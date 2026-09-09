//! Pure connection resolution shared by CLI, server and TUI. A selected
//! provider, endpoint, key pool and model must never be assembled from
//! unrelated credential records. Call once with one parsed saved snapshot.
use super::credentials::{is_placeholder_key, is_placeholder_key_lenient, CredentialsFile};
use crate::types::{config::ProviderConfig, model_registry::default_model};
use std::collections::HashMap;

fn candidate_key(key: &str) -> bool {
    !key.trim().is_empty() && !key.trim().starts_with("${")
}

pub fn configured(config: &ProviderConfig) -> bool {
    config.api_key.as_deref().is_some_and(candidate_key)
        || config.keys.iter().any(|key| candidate_key(key))
        || config.name.as_deref() == Some("openai-codex")
}

pub fn scoped_headers(
    config: &ProviderConfig,
    provider: &str,
    endpoint: &Option<String>,
) -> HashMap<String, String> {
    let name = config.name.as_deref().unwrap_or("anthropic");
    if name == provider
        && &config.base_url == endpoint
        && (config.name.is_some() || configured(config))
    {
        config.extra_headers.clone()
    } else {
        HashMap::new()
    }
}

pub fn resolve(config: &ProviderConfig, saved: Option<&CredentialsFile>) -> Option<ProviderConfig> {
    if configured(config) {
        let name = config.name.clone().unwrap_or_else(|| "anthropic".into());
        let mut keys: Vec<String> = config
            .keys
            .iter()
            .filter(|key| candidate_key(key))
            .cloned()
            .collect();
        if let Some(key) = config
            .api_key
            .as_ref()
            .filter(|key| candidate_key(key) && !keys.contains(key))
        {
            keys.insert(0, key.clone());
        }
        return Some(ProviderConfig {
            model: Some(
                config
                    .model
                    .clone()
                    .unwrap_or_else(|| default_model(&name).into()),
            ),
            name: Some(name),
            api_key: keys.first().cloned(),
            keys,
            base_url: config.base_url.clone(),
            extra_headers: config.extra_headers.clone(),
        });
    }
    let saved = saved?;
    // Preserve legacy saved-selection behavior. The on-disk active name is
    // not yet a unique endpoint/account identifier.
    let provider = saved
        .providers
        .iter()
        .find(|p| p.name == saved.active)
        .or_else(|| saved.providers.first())?;
    let keys: Vec<String> = provider
        .keys
        .iter()
        .filter(|key| {
            if provider.base_url.is_some() {
                !is_placeholder_key_lenient(key)
            } else {
                !is_placeholder_key(key)
            }
        })
        .cloned()
        .collect();
    if provider.name.is_empty() || (keys.is_empty() && provider.name != "openai-codex") {
        return None;
    }
    Some(ProviderConfig {
        name: Some(provider.name.clone()),
        api_key: keys.first().cloned(),
        keys,
        model: Some(provider.model.clone()),
        base_url: provider.base_url.clone(),
        extra_headers: scoped_headers(config, &provider.name, &provider.base_url),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::credentials::CredentialsProvider;
    #[test]
    fn configured_key_pool_and_endpoint_cannot_be_replaced_by_saved_data() {
        let config = ProviderConfig {
            name: Some("openai".into()),
            keys: vec![
                "synthetic-selected-key".into(),
                "synthetic-selected-backup".into(),
            ],
            base_url: Some("http://127.0.0.1:1111/v1".into()),
            ..Default::default()
        };
        let saved = CredentialsFile {
            active: "anthropic".into(),
            providers: vec![CredentialsProvider {
                name: "anthropic".into(),
                keys: vec!["synthetic-other-key".into()],
                model: "other".into(),
                base_url: Some("http://127.0.0.1:2222/v1".into()),
            }],
        };
        let resolved = resolve(&config, Some(&saved)).unwrap();
        assert_eq!(resolved.keys, config.keys);
        assert_eq!(resolved.base_url, config.base_url);
        assert_eq!(resolved.name.as_deref(), Some("openai"));
        assert!(resolved.model.is_some());
    }
    #[test]
    fn default_config_provider_preserves_its_headers_but_foreign_saved_route_does_not() {
        let mut config = ProviderConfig {
            api_key: Some("synthetic-selected-key".into()),
            ..Default::default()
        };
        config
            .extra_headers
            .insert("Authorization".into(), "synthetic-selected-header".into());
        let resolved = resolve(&config, None).unwrap();
        assert_eq!(resolved.extra_headers, config.extra_headers);
        assert_eq!(
            scoped_headers(&config, "anthropic", &None),
            config.extra_headers
        );
        assert!(scoped_headers(&config, "openai", &None).is_empty());
        assert!(scoped_headers(
            &config,
            "anthropic",
            &Some("http://127.0.0.1:3333/v1".into())
        )
        .is_empty());
    }
    #[test]
    fn explicit_codex_needs_no_api_key_and_keeps_selected_model() {
        let config = ProviderConfig {
            name: Some("openai-codex".into()),
            model: Some("gpt-6-astra".into()),
            ..Default::default()
        };
        let resolved = resolve(&config, None).unwrap();
        assert!(resolved.api_key.is_none() && resolved.keys.is_empty());
        assert_eq!(resolved.model, config.model);
    }
}
