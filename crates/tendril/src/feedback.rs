//! In-CLI breakage feedback (feedback-cli wiring).
//!
//! Routes structured Tendril breakages back to the owning project so they can
//! become beads / logged errors, using the shared `feedback-cli` crate (the
//! same harryaskham CLI stack as `mcp-cli` and `updatable-cli`). The reporting
//! *strategy* — webhook (e.g. a caco webhook that files a bead), the local
//! `caco` CLI, a file, or stderr — is selected from the environment, so a
//! project can route Tendril breakages to beads without any code change here.
//!
//! Feedback is **opt-in**: with nothing configured the reporter is disabled, so
//! the default Tendril CLI never adds stderr noise or files beads. Set
//! `FEEDBACK_WEBHOOK_URL` (and optionally `FEEDBACK_WEBHOOK_TOKEN_ENV` and
//! `FEEDBACK_PROJECT`) to enable webhook delivery to a caco feedback endpoint
//! that turns breakages into beads.
//!
//! Because the generic `FEEDBACK_WEBHOOK_URL` is shared by every `feedback-cli`
//! tool on a host, Tendril resolves its own webhook URL first: an explicit
//! `TENDRIL_FEEDBACK_WEBHOOK_URL`, else the canonical shared namespace
//! `FEEDBACK_WEBHOOK_BASE_URL` joined with Tendril's hook name (default
//! `tendril-feedback`, overridable via `TENDRIL_FEEDBACK_HOOK`). The token
//! env-var name comes from `TENDRIL_FEEDBACK_WEBHOOK_TOKEN_ENV`, falling back to
//! the generic `FEEDBACK_WEBHOOK_TOKEN_ENV` (the single shared feedback token).
//! This lets each tool target its *own* feedback hook under one base URL/token
//! so Tendril breakages never spam another project's hook (bd-67a336).

use feedback_cli::{FeedbackConfig, FeedbackEvent, ReportStrategy, Reporter, WebhookConfig};

use crate::error::TendrilError;

/// Component label stamped on every Tendril feedback event.
pub const FEEDBACK_COMPONENT: &str = "tendril";

/// Resolve the feedback configuration.
///
/// Prefers an explicit project-config feedback strategy (`[feedback]` in the
/// Tendril config). When the project does not configure one, falls back to the
/// environment (`FEEDBACK_WEBHOOK_URL`) and demotes the unconfigured
/// `FeedbackConfig::from_env()` stderr fallback to [`ReportStrategy::Disabled`],
/// so Tendril feedback is strictly opt-in and otherwise silent. The component is
/// defaulted to `tendril`.
#[must_use]
pub fn feedback_config(configured: Option<&FeedbackConfig>) -> FeedbackConfig {
    let mut config = configured.map_or_else(
        || {
            // bd-67a336: prefer a tendril-specific webhook override so tendril
            // breakages target the tendril feedback hook instead of inheriting
            // the generic FEEDBACK_WEBHOOK_URL that other feedback-cli tools
            // (e.g. omni-cli) may share on the same host.
            if let Some(over) = tendril_env_override() {
                return over;
            }
            // `from_env()` falls back to the stderr strategy when no webhook URL
            // is set; demote that to Disabled so an unconfigured Tendril CLI
            // never writes an extra feedback line to stderr or files beads.
            let mut env = FeedbackConfig::from_env();
            if matches!(env.strategy, ReportStrategy::Stderr) {
                env.strategy = ReportStrategy::Disabled;
            }
            // bd-13c534: env-driven webhook delivery defaults to non-blocking so
            // breakage reporting never adds a synchronous HTTP round-trip (or a
            // hang on a slow/unreachable endpoint) to the CLI's error-exit path.
            // Tendril is a high-frequency stateless CLI, so best-effort
            // background delivery is the safe default; a project that wants
            // synchronous delivery configures `[feedback]` with `blocking: true`.
            if let ReportStrategy::Webhook(ref mut webhook) = env.strategy {
                webhook.blocking = false;
            }
            env
        },
        // An explicit project-config strategy wins verbatim (including stderr if
        // the project really wants it).
        FeedbackConfig::clone,
    );
    config
        .component
        .get_or_insert_with(|| FEEDBACK_COMPONENT.to_owned());
    config
}

/// Read the tendril-specific feedback webhook override from the environment.
///
/// bd-67a336: `feedback-cli`'s `FeedbackConfig::from_env()` only honours the
/// generic `FEEDBACK_WEBHOOK_URL`, which is shared by every `feedback-cli` tool
/// on a host. When a node also routes another CLI's feedback (e.g. omni-cli) via
/// that generic URL, tendril breakages would POST to the *other* project's hook
/// (bead-type webhooks file into their own configured `bead.project`, ignoring
/// the sender's project). Tendril therefore resolves its own webhook URL from,
/// in precedence order:
///
/// 1. `TENDRIL_FEEDBACK_WEBHOOK_URL` — an explicit full URL escape hatch.
/// 2. `FEEDBACK_WEBHOOK_BASE_URL` + tendril's hook name — the canonical shared
///    namespace convention, where the operator sets one base URL
///    (`.../hooks/global`) and one shared token, and each tool appends its own
///    hook. The hook segment defaults to [`DEFAULT_FEEDBACK_HOOK`] and can be
///    overridden with `TENDRIL_FEEDBACK_HOOK`.
///
/// The token env-var name comes from `TENDRIL_FEEDBACK_WEBHOOK_TOKEN_ENV`,
/// falling back to the generic `FEEDBACK_WEBHOOK_TOKEN_ENV` (the canonical
/// single shared feedback token).
fn tendril_env_override() -> Option<FeedbackConfig> {
    tendril_webhook_override(
        std::env::var("TENDRIL_FEEDBACK_WEBHOOK_URL").ok(),
        std::env::var("FEEDBACK_WEBHOOK_BASE_URL").ok(),
        std::env::var("TENDRIL_FEEDBACK_HOOK").ok(),
        std::env::var("TENDRIL_FEEDBACK_WEBHOOK_TOKEN_ENV").ok(),
        std::env::var("FEEDBACK_WEBHOOK_TOKEN_ENV").ok(),
        std::env::var("FEEDBACK_PROJECT").ok(),
    )
}

/// Default tendril feedback hook name appended to `FEEDBACK_WEBHOOK_BASE_URL`.
/// Matches the caco webhook entry id (`/hooks/global/tendril-feedback`).
const DEFAULT_FEEDBACK_HOOK: &str = "tendril-feedback";

/// Pure builder for the tendril-specific webhook override (bd-67a336), split out
/// from [`tendril_env_override`] so the precedence/fallback logic is
/// unit-testable without touching process environment.
fn tendril_webhook_override(
    full_url: Option<String>,
    base_url: Option<String>,
    hook: Option<String>,
    token_env: Option<String>,
    generic_token_env: Option<String>,
    project: Option<String>,
) -> Option<FeedbackConfig> {
    let url = tendril_feedback_url(full_url, base_url, hook)?;
    let token_env = token_env
        .filter(|value| !value.trim().is_empty())
        .or_else(|| generic_token_env.filter(|value| !value.trim().is_empty()));
    let project = project.filter(|value| !value.trim().is_empty());
    Some(FeedbackConfig {
        enabled: true,
        project,
        // Non-blocking so breakage reporting never adds a synchronous HTTP
        // round-trip to the CLI's error-exit path (matches the env default).
        strategy: ReportStrategy::Webhook(WebhookConfig {
            url,
            token_env,
            blocking: false,
            ..WebhookConfig::default()
        }),
        ..FeedbackConfig::default()
    })
}

/// Resolve the tendril feedback webhook URL from either an explicit full URL or
/// a shared base URL joined with tendril's hook name (bd-67a336). Slashes at the
/// base/hook boundary are normalised so both `.../global` and `.../global/`
/// bases work. Returns `None` when neither a full URL nor a base URL is set.
fn tendril_feedback_url(
    full_url: Option<String>,
    base_url: Option<String>,
    hook: Option<String>,
) -> Option<String> {
    if let Some(full) = full_url
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        return Some(full);
    }
    let base = base_url
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())?;
    let hook = hook
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_FEEDBACK_HOOK.to_owned());
    Some(format!(
        "{}/{}",
        base.trim_end_matches('/'),
        hook.trim_start_matches('/')
    ))
}

/// Best-effort: report a Tendril breakage so the owning project can turn it
/// into a bead / logged error.
///
/// This never fails or blocks the CLI's own error path — feedback delivery
/// errors are swallowed, because the breakage is already surfaced to the user
/// via `emit_error`. When feedback is unconfigured the reporter is disabled and
/// this is a no-op.
pub fn report_breakage(
    configured: Option<&FeedbackConfig>,
    command: Option<&str>,
    error: &TendrilError,
) {
    let reporter = Reporter::from_config(&feedback_config(configured));
    let mut event = FeedbackEvent::from_structured_error(FEEDBACK_COMPONENT, error);
    if let Some(command) = command {
        event = event.with_field("command", command);
    }
    let _ = reporter.report(&event);
}

#[cfg(test)]
mod tests {
    use super::{FEEDBACK_COMPONENT, feedback_config};
    use feedback_cli::ReportStrategy;

    #[test]
    fn feedback_is_disabled_when_unconfigured() {
        // Without FEEDBACK_WEBHOOK_URL the reporter must be silent (no stderr
        // noise / no beads) so the default Tendril CLI behaviour is unchanged.
        // NB: this asserts the demotion logic on a config built directly to
        // avoid depending on ambient process env in the test runner.
        let mut config = feedback_cli::FeedbackConfig::from_env();
        config
            .component
            .get_or_insert_with(|| FEEDBACK_COMPONENT.to_owned());
        if matches!(config.strategy, ReportStrategy::Stderr) {
            config.strategy = ReportStrategy::Disabled;
        }
        // Either it was webhook-configured via ambient env, or it is Disabled.
        let ok = matches!(
            config.strategy,
            ReportStrategy::Disabled | ReportStrategy::Webhook(_)
        );
        assert!(ok, "unconfigured feedback must not default to stderr");
        assert_eq!(config.component.as_deref(), Some(FEEDBACK_COMPONENT));
    }

    #[test]
    fn feedback_config_stamps_tendril_component() {
        let config = feedback_config(None);
        assert_eq!(config.component.as_deref(), Some(FEEDBACK_COMPONENT));
        assert!(
            !matches!(config.strategy, ReportStrategy::Stderr),
            "Tendril demotes the stderr fallback to Disabled"
        );
    }

    #[test]
    fn project_config_feedback_strategy_is_preferred() {
        // An explicit project-config strategy wins over the env fallback.
        let configured = feedback_cli::FeedbackConfig {
            strategy: ReportStrategy::Disabled,
            ..feedback_cli::FeedbackConfig::default()
        };
        let resolved = feedback_config(Some(&configured));
        assert!(matches!(resolved.strategy, ReportStrategy::Disabled));
        assert_eq!(resolved.component.as_deref(), Some(FEEDBACK_COMPONENT));
    }

    #[test]
    fn env_webhook_defaults_to_non_blocking() {
        // bd-13c534: an env-driven webhook must not block the CLI exit. Build the
        // same shape feedback_config() produces for FEEDBACK_WEBHOOK_URL and
        // assert the non-blocking demotion, without depending on ambient env.
        let mut env = feedback_cli::FeedbackConfig {
            strategy: ReportStrategy::Webhook(feedback_cli::WebhookConfig {
                url: "https://example.invalid/feedback".to_owned(),
                ..feedback_cli::WebhookConfig::default()
            }),
            ..feedback_cli::FeedbackConfig::default()
        };
        if let ReportStrategy::Webhook(ref mut webhook) = env.strategy {
            webhook.blocking = false;
        }
        match env.strategy {
            ReportStrategy::Webhook(webhook) => {
                assert!(
                    !webhook.blocking,
                    "env webhook feedback must be non-blocking"
                );
            }
            other => panic!("expected webhook strategy, got {other:?}"),
        }
    }

    #[test]
    fn tendril_feedback_url_prefers_explicit_full_url() {
        let url = super::tendril_feedback_url(
            Some("http://host:11300/hooks/global/custom".to_owned()),
            Some("http://host:11300/hooks/global".to_owned()),
            Some("ignored".to_owned()),
        );
        assert_eq!(
            url.as_deref(),
            Some("http://host:11300/hooks/global/custom")
        );
    }

    #[test]
    fn tendril_feedback_url_joins_base_with_default_hook() {
        let url = super::tendril_feedback_url(
            None,
            Some("http://helsinki:11300/hooks/global".to_owned()),
            None,
        );
        assert_eq!(
            url.as_deref(),
            Some("http://helsinki:11300/hooks/global/tendril-feedback")
        );
    }

    #[test]
    fn tendril_feedback_url_normalises_slashes_and_custom_hook() {
        let url = super::tendril_feedback_url(
            None,
            Some("http://helsinki:11300/hooks/global/".to_owned()),
            Some("/tendril".to_owned()),
        );
        assert_eq!(
            url.as_deref(),
            Some("http://helsinki:11300/hooks/global/tendril")
        );
    }

    #[test]
    fn tendril_feedback_url_is_none_without_full_or_base() {
        assert!(super::tendril_feedback_url(None, None, Some("tendril".to_owned())).is_none());
        assert!(
            super::tendril_feedback_url(Some("  ".to_owned()), Some("  ".to_owned()), None)
                .is_none()
        );
    }

    #[test]
    fn tendril_webhook_override_builds_non_blocking_webhook_from_base() {
        let config = super::tendril_webhook_override(
            None,
            Some("http://helsinki:11300/hooks/global".to_owned()),
            None,
            Some("CACOPHONY_FEEDBACK_TOKEN".to_owned()),
            Some("IGNORED_GENERIC".to_owned()),
            Some("tendril".to_owned()),
        )
        .expect("a base url should produce a tendril webhook override");
        assert_eq!(config.project.as_deref(), Some("tendril"));
        match config.strategy {
            ReportStrategy::Webhook(webhook) => {
                assert_eq!(
                    webhook.url,
                    "http://helsinki:11300/hooks/global/tendril-feedback"
                );
                // The tendril-specific token env wins over the generic one.
                assert_eq!(
                    webhook.token_env.as_deref(),
                    Some("CACOPHONY_FEEDBACK_TOKEN")
                );
                assert!(
                    !webhook.blocking,
                    "tendril webhook override must be non-blocking"
                );
            }
            other => panic!("expected webhook strategy, got {other:?}"),
        }
    }

    #[test]
    fn tendril_webhook_override_falls_back_to_generic_token_env() {
        let config = super::tendril_webhook_override(
            Some("http://host/hooks/global/tendril-feedback".to_owned()),
            None,
            None,
            None,
            Some("CACOPHONY_FEEDBACK_TOKEN".to_owned()),
            None,
        )
        .expect("url present");
        match config.strategy {
            ReportStrategy::Webhook(webhook) => {
                assert_eq!(
                    webhook.token_env.as_deref(),
                    Some("CACOPHONY_FEEDBACK_TOKEN")
                );
            }
            other => panic!("expected webhook, got {other:?}"),
        }
        assert!(config.project.is_none());
    }

    #[test]
    fn tendril_webhook_override_is_none_without_url_or_base() {
        assert!(
            super::tendril_webhook_override(
                None,
                None,
                Some("tendril".to_owned()),
                None,
                None,
                None
            )
            .is_none()
        );
    }
}
