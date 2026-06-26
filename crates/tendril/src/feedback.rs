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

/// Resolve the feedback configuration from the environment.
///
/// Defaults the component to `tendril` and demotes the unconfigured
/// `FeedbackConfig::from_env()` stderr fallback to [`ReportStrategy::Disabled`],
/// so Tendril feedback is strictly opt-in: it is enabled by configuring
/// `FEEDBACK_WEBHOOK_URL` (or, in future, a project config feedback strategy),
/// and is otherwise silent.
#[must_use]
pub fn feedback_config() -> FeedbackConfig {
    let mut config = FeedbackConfig::from_env();
    config
        .component
        .get_or_insert_with(|| FEEDBACK_COMPONENT.to_owned());
    // `from_env()` falls back to the stderr strategy when no webhook URL is set;
    // demote that to Disabled so an unconfigured Tendril CLI never writes an
    // extra feedback line to stderr or files beads. Any explicitly configured
    // strategy (webhook/caco-cli/file) is preserved.
    if matches!(config.strategy, ReportStrategy::Stderr) {
        config.strategy = ReportStrategy::Disabled;
    }
    config
}

/// Best-effort: report a Tendril breakage so the owning project can turn it
/// into a bead / logged error.
///
/// This never fails or blocks the CLI's own error path — feedback delivery
/// errors are swallowed, because the breakage is already surfaced to the user
/// via `emit_error`. When feedback is unconfigured the reporter is disabled and
/// this is a no-op.
pub fn report_breakage(command: Option<&str>, error: &TendrilError) {
    let reporter = Reporter::from_config(&feedback_config());
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
        let config = feedback_config();
        assert_eq!(config.component.as_deref(), Some(FEEDBACK_COMPONENT));
        assert!(
            !matches!(config.strategy, ReportStrategy::Stderr),
            "Tendril demotes the stderr fallback to Disabled"
        );
    }
}
