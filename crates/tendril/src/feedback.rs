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

use feedback_cli::{FeedbackConfig, FeedbackEvent, ReportStrategy, Reporter};

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
            // `from_env()` falls back to the stderr strategy when no webhook URL
            // is set; demote that to Disabled so an unconfigured Tendril CLI
            // never writes an extra feedback line to stderr or files beads.
            let mut env = FeedbackConfig::from_env();
            if matches!(env.strategy, ReportStrategy::Stderr) {
                env.strategy = ReportStrategy::Disabled;
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
        let mut configured = feedback_cli::FeedbackConfig::default();
        configured.strategy = ReportStrategy::Disabled;
        let resolved = feedback_config(Some(&configured));
        assert!(matches!(resolved.strategy, ReportStrategy::Disabled));
        assert_eq!(resolved.component.as_deref(), Some(FEEDBACK_COMPONENT));
    }
}
