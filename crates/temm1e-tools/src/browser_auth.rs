//! Credential-bound browser operations. No provider or browser startup is needed
//! to validate the origin policy and secret-free report formatting.
use temm1e_core::types::error::Temm1eError;

pub(crate) fn authorize_origin(current: &str, saved: &str) -> Result<(), Temm1eError> {
    let deny = || {
        Temm1eError::Tool("Credential destination does not match the saved service origin; use the service login flow to configure it explicitly.".into())
    };
    let current = reqwest::Url::parse(current).map_err(|_| deny())?;
    let saved = reqwest::Url::parse(saved).map_err(|_| deny())?;
    for url in [&current, &saved] {
        if !matches!(url.scheme(), "https" | "http")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(deny());
        }
    }
    if current.origin() != saved.origin() {
        return Err(deny());
    }
    Ok(())
}

pub(crate) fn submission_report(
    service: &str,
    tree: &str,
    username: &str,
    password: &str,
) -> String {
    // Explicit credentials must be removed even when shorter than the general
    // scrubber's heuristic threshold. They are known secrets, not guesses.
    let mut redacted = tree.to_owned();
    for value in [password, username] {
        if !value.is_empty() {
            redacted = redacted.replace(value, "[REDACTED]");
        }
    }
    let redacted = crate::credential_scrub::scrub(&redacted, &[]);
    format!("Login form submitted for '{service}'. Authentication is not verified; inspect positive account evidence or complete any challenge before claiming success. Post-submit page:\n{redacted}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credentials_are_bound_to_scheme_host_and_effective_port() {
        assert!(authorize_origin(
            "https://example.com/account",
            "https://example.com:443/login"
        )
        .is_ok());
        for current in [
            "https://example.com.attacker.test/login",
            "https://other.example.com/login",
            "http://example.com/login",
            "https://example.com:444/login",
            "data:text/html,login",
            "about:blank",
        ] {
            assert!(
                authorize_origin(current, "https://example.com/login").is_err(),
                "{current}"
            );
        }
        assert!(authorize_origin("https://example.com/login", "").is_err());
        let userinfo = format!("https://{}@example.com", "user");
        assert!(authorize_origin(&userinfo, "https://example.com/login").is_err());
    }
    #[test]
    fn submission_does_not_assert_success_or_expose_short_credentials() {
        let report = submission_report(
            "fixture",
            "Account zz / value p! / challenge required",
            "zz",
            "p!",
        );
        assert!(!report.contains("zz") && !report.contains("p!"));
        assert!(report.contains("Authentication is not verified"));
        assert!(report.contains("challenge required"));
        assert!(!report.contains("Authenticated to"));
    }
}
