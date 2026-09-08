//! Capture one connection before starting a bridge. No credential store reads
//! are permitted while constructing a provider from this snapshot.
use crate::agent_bridge::AgentSetup;
use temm1e_core::config::credentials::CredentialsFile;
use temm1e_core::types::config::{ProviderConfig, Temm1eConfig};

pub(crate) fn resolve(
    config: &Temm1eConfig,
    saved: Option<&CredentialsFile>,
) -> Option<AgentSetup> {
    let connection = temm1e_core::config::connection::resolve(&config.provider, saved)?;
    Some(AgentSetup {
        provider_name: connection.name?,
        api_key: connection.api_key.unwrap_or_default(),
        keys: connection.keys,
        model: connection.model?,
        base_url: connection.base_url,
        config: config.clone(),
        mode: None,
    })
}

/// Preserve an existing rotation pool only when its route and selected key match.
pub(crate) fn onboarding_keys(
    saved: Option<&CredentialsFile>,
    name: &str,
    endpoint: &Option<String>,
    selected: &str,
) -> Vec<String> {
    let matches: Vec<_> = saved
        .into_iter()
        .flat_map(|file| file.providers.iter())
        .filter(|p| {
            p.name == name && &p.base_url == endpoint && p.keys.iter().any(|key| key == selected)
        })
        .collect();
    if matches.len() == 1 {
        matches[0].keys.clone()
    } else {
        vec![selected.into()]
    }
}

pub(crate) fn provider_config(setup: &AgentSetup) -> ProviderConfig {
    let mut keys = setup.keys.clone();
    if !setup.api_key.is_empty() && !keys.contains(&setup.api_key) {
        keys.insert(0, setup.api_key.clone());
    }
    // Headers can contain credentials too. Only inherit them from the exact
    // configured provider/endpoint; a saved/onboarded connection is independent.
    let headers = temm1e_core::config::connection::scoped_headers(
        &setup.config.provider,
        &setup.provider_name,
        &setup.base_url,
    );
    ProviderConfig {
        name: Some(setup.provider_name.clone()),
        api_key: keys.first().cloned(),
        keys,
        model: Some(setup.model.clone()),
        base_url: setup.base_url.clone(),
        extra_headers: headers,
    }
}

/// Update only a still-matching saved selection. A config-owned connection is
/// session-only here: changing credentials would not override its config model.
pub(crate) fn update_saved_model(setup: &AgentSetup, saved: &mut CredentialsFile) -> bool {
    if temm1e_core::config::connection::configured(&setup.config.provider)
        || saved.active != setup.provider_name
    {
        return false;
    }
    let matching: Vec<usize> = saved
        .providers
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            p.name == setup.provider_name
                && p.base_url == setup.base_url
                && (setup.provider_name == "openai-codex"
                    || p.keys.iter().any(|key| setup.keys.contains(key)))
        })
        .map(|(i, _)| i)
        .collect();
    if matching.len() != 1 {
        return false;
    }
    saved.providers[matching[0]].model.clone_from(&setup.model);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use temm1e_core::config::credentials::CredentialsProvider;

    fn saved() -> CredentialsFile {
        CredentialsFile {
            active: "anthropic".into(),
            providers: vec![CredentialsProvider {
                name: "anthropic".into(),
                keys: vec![
                    "synthetic-old-key-one".into(),
                    "synthetic-old-key-two".into(),
                ],
                model: "old-model".into(),
                base_url: Some("http://127.0.0.1:9999/v1".into()),
            }],
        }
    }

    #[test]
    fn explicit_connection_never_inherits_saved_credentials_or_endpoint() {
        let mut config = Temm1eConfig::default();
        config.provider.name = Some("openai".into());
        config.provider.api_key = Some("synthetic-selected-key".into());
        config.provider.keys = vec!["synthetic-selected-backup".into()];
        config.provider.base_url = Some("http://127.0.0.1:8888/v1".into());
        config.provider.model = Some("selected-model".into());
        let mut store = saved();
        let mut setup = resolve(&config, Some(&store)).unwrap();
        store.providers[0].keys = vec!["changed-after-resolution".into()];
        let connection = provider_config(&setup);
        assert_eq!(connection.name.as_deref(), Some("openai"));
        assert_eq!(connection.base_url, config.provider.base_url);
        assert_eq!(
            connection.keys,
            vec!["synthetic-selected-key", "synthetic-selected-backup"]
        );
        assert_eq!(connection.model.as_deref(), Some("selected-model"));
        setup.model = "replacement".into();
        assert!(!update_saved_model(&setup, &mut store));
        assert_eq!(store.providers[0].model, "old-model");
    }

    #[test]
    fn saved_connection_keeps_all_keys_and_rejects_unrelated_headers() {
        let mut config = Temm1eConfig::default();
        config.provider.name = Some("openai".into());
        config
            .provider
            .extra_headers
            .insert("Authorization".into(), "synthetic-other-secret".into());
        let mut store = saved();
        let mut setup = resolve(&config, Some(&store)).unwrap();
        let connection = provider_config(&setup);
        assert!(connection.extra_headers.is_empty());
        assert_eq!(connection.keys, store.providers[0].keys);
        assert_eq!(connection.base_url, store.providers[0].base_url);
        setup.model = "replacement".into();
        assert!(update_saved_model(&setup, &mut store));
        assert_eq!(store.providers[0].keys.len(), 2);
        assert_eq!(store.providers[0].model, "replacement");
        store.providers[0].base_url = Some("http://127.0.0.1:7777/v1".into());
        assert!(!update_saved_model(&setup, &mut store));
    }

    #[test]
    fn onboarding_pool_requires_exact_route_and_selected_key() {
        let store = saved();
        let p = &store.providers[0];
        assert_eq!(
            onboarding_keys(Some(&store), &p.name, &p.base_url, &p.keys[0]),
            p.keys
        );
        for (name, endpoint, key) in [
            ("openai", p.base_url.clone(), p.keys[0].as_str()),
            (p.name.as_str(), None, p.keys[0].as_str()),
            (p.name.as_str(), p.base_url.clone(), "new-selected-key"),
        ] {
            assert_eq!(
                onboarding_keys(Some(&store), name, &endpoint, key),
                vec![key.to_string()]
            );
        }
    }

    #[test]
    fn keyless_codex_is_resolved_without_inventing_api_credentials() {
        let mut store = saved();
        store.active = "openai-codex".into();
        store.providers[0].name = "openai-codex".into();
        store.providers[0].keys.clear();
        store.providers[0].base_url = None;
        let setup = resolve(&Temm1eConfig::default(), Some(&store)).unwrap();
        let connection = provider_config(&setup);
        assert!(connection.keys.is_empty());
        assert!(connection.api_key.is_none());
        assert_eq!(connection.name.as_deref(), Some("openai-codex"));
    }

    #[test]
    fn changed_or_ambiguous_saved_selection_is_not_overwritten() {
        let mut store = saved();
        let setup = resolve(&Temm1eConfig::default(), Some(&store)).unwrap();
        store.active = "different".into();
        assert!(!update_saved_model(&setup, &mut store));
        store.active = setup.provider_name.clone();
        store.providers.push(store.providers[0].clone());
        assert!(!update_saved_model(&setup, &mut store));
    }
}
