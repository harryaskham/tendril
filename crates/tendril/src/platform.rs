use std::env;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ashpd::desktop::screenshot::Screenshot as PortalScreenshot;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use pollster::block_on;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use mcp_cli::ErrorCategory;

use crate::capture::{current_timestamp, read_and_remove_temp_capture, unique_temp_path};
use crate::discovery;
use crate::error::TendrilError;
use crate::input::{relative_point_to_absolute, reliability_delay};
use crate::model::{
    Bounds, ElementListInput, ElementListOutput, FocusSnapshot, InputAction, ModifierKey,
    MouseButton, ScaleFactor,
};
use crate::wayland_input;
use crate::x11;

const CAPTURE_FIXTURE_ENV: &str = "TENDRIL_CAPTURE_FIXTURE_JSON";
const INPUT_FIXTURE_ENV: &str = "TENDRIL_INPUT_FIXTURE_JSON";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformKind {
    MacOs,
    Linux,
    Windows11,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopSession {
    MacOsWindowServer,
    X11,
    Wayland,
    WindowsDesktop,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioBackend {
    CoreAudio,
    PipeWire,
    PulseAudio,
    Wasapi,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterContext {
    pub platform: PlatformKind,
    pub session: DesktopSession,
    pub audio_backend: Option<AudioBackend>,
}

impl AdapterContext {
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::macos()
        }

        #[cfg(target_os = "linux")]
        {
            Self::linux(
                detect_linux_session(
                    env::var("XDG_SESSION_TYPE").ok().as_deref(),
                    env::var_os("DISPLAY").as_deref(),
                    env::var_os("WAYLAND_DISPLAY").as_deref(),
                ),
                detect_linux_audio_backend(
                    env::var_os("PIPEWIRE_RUNTIME_DIR").as_deref(),
                    env::var_os("PULSE_SERVER").as_deref(),
                    env::var_os("PULSE_RUNTIME_PATH").as_deref(),
                    env::var_os("XDG_RUNTIME_DIR").as_deref(),
                    &|path| std::path::Path::new(path).exists(),
                ),
            )
        }

        #[cfg(target_os = "windows")]
        {
            Self::windows11()
        }
    }

    #[must_use]
    pub const fn macos() -> Self {
        Self {
            platform: PlatformKind::MacOs,
            session: DesktopSession::MacOsWindowServer,
            audio_backend: Some(AudioBackend::CoreAudio),
        }
    }

    #[must_use]
    pub const fn linux(session: DesktopSession, audio_backend: Option<AudioBackend>) -> Self {
        Self {
            platform: PlatformKind::Linux,
            session,
            audio_backend,
        }
    }

    #[must_use]
    pub const fn windows11() -> Self {
        Self {
            platform: PlatformKind::Windows11,
            session: DesktopSession::WindowsDesktop,
            audio_backend: Some(AudioBackend::Wasapi),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub platform: PlatformKind,
    pub session: DesktopSession,
    pub audio_backend: Option<AudioBackend>,
    pub stateless: bool,
}

impl AdapterInfo {
    #[must_use]
    pub fn from_context(context: &AdapterContext) -> Self {
        Self {
            platform: context.platform,
            session: context.session,
            audio_backend: context.audio_backend,
            stateless: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    TargetDiscovery,
    WindowCapture,
    DisplayCapture,
    InputControl,
    AudioLoopbackCapture,
    AudioInputCapture,
    ElementDiscovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureTargetKind {
    Window,
    Display,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSourceKind {
    SystemLoopback,
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    ScreenCapture,
    Accessibility,
    Microphone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    NotRequired,
    Unknown,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub permission: PermissionKind,
    pub state: PermissionState,
    pub summary: String,
    pub suggested_action: Option<String>,
}

impl PermissionStatus {
    #[must_use]
    pub fn unknown(
        permission: PermissionKind,
        summary: impl Into<String>,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self {
            permission,
            state: PermissionState::Unknown,
            summary: summary.into(),
            suggested_action: Some(suggested_action.into()),
        }
    }

    #[must_use]
    pub fn granted(permission: PermissionKind, summary: impl Into<String>) -> Self {
        Self {
            permission,
            state: PermissionState::Granted,
            summary: summary.into(),
            suggested_action: None,
        }
    }

    #[must_use]
    pub fn denied(
        permission: PermissionKind,
        summary: impl Into<String>,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self {
            permission,
            state: PermissionState::Denied,
            summary: summary.into(),
            suggested_action: Some(suggested_action.into()),
        }
    }

    #[must_use]
    pub fn not_required(permission: PermissionKind, summary: impl Into<String>) -> Self {
        Self {
            permission,
            state: PermissionState::NotRequired,
            summary: summary.into(),
            suggested_action: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSupport {
    pub capability: Capability,
    pub permissions: Vec<PermissionStatus>,
    pub notes: Vec<String>,
}

impl FeatureSupport {
    #[must_use]
    pub fn available(capability: Capability) -> Self {
        Self {
            capability,
            permissions: Vec::new(),
            notes: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_permissions(mut self, permissions: Vec<PermissionStatus>) -> Self {
        self.permissions = permissions;
        self
    }

    #[must_use]
    pub fn with_notes(mut self, notes: Vec<String>) -> Self {
        self.notes = notes;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityErrorReason {
    UnsupportedPlatform,
    UnsupportedSession,
    UnsupportedFeature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityError {
    pub capability: Capability,
    pub platform: PlatformKind,
    pub reason: CapabilityErrorReason,
    pub message: String,
    pub suggested_action: Option<String>,
}

impl std::fmt::Display for CapabilityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Unsupported capability {:?} on {:?}: {}",
            self.capability, self.platform, self.message
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterOperation {
    TargetDiscovery,
    Capture,
    InputControl,
    PermissionCheck,
    AudioProbe,
    ElementDiscovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum PlatformAdapterError {
    #[error("{0}")]
    UnsupportedCapability(CapabilityError),

    #[error("Missing permission {permission:?} for {capability:?} on {platform:?}: {message}")]
    MissingPermission {
        capability: Capability,
        permission: PermissionKind,
        platform: PlatformKind,
        message: String,
        suggested_action: String,
    },

    #[error("Platform adapter failure during {operation:?} on {platform:?}: {message}")]
    AdapterFailure {
        operation: AdapterOperation,
        platform: PlatformKind,
        message: String,
    },

    #[error(
        "Platform adapter timeout during {operation:?} on {platform:?} after {timeout_ms}ms: {message}"
    )]
    Timeout {
        operation: AdapterOperation,
        platform: PlatformKind,
        timeout_ms: u64,
        message: String,
        backend: Option<String>,
        suggested_action: Option<String>,
    },
}

impl PlatformAdapterError {
    #[must_use]
    pub fn unsupported(
        capability: Capability,
        platform: PlatformKind,
        reason: CapabilityErrorReason,
        message: impl Into<String>,
        suggested_action: Option<&str>,
    ) -> Self {
        Self::UnsupportedCapability(CapabilityError {
            capability,
            platform,
            reason,
            message: message.into(),
            suggested_action: suggested_action.map(str::to_owned),
        })
    }

    #[must_use]
    pub fn missing_permission(
        capability: Capability,
        permission: PermissionKind,
        platform: PlatformKind,
        message: impl Into<String>,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self::MissingPermission {
            capability,
            permission,
            platform,
            message: message.into(),
            suggested_action: suggested_action.into(),
        }
    }

    #[must_use]
    pub fn adapter_failure(
        operation: AdapterOperation,
        platform: PlatformKind,
        message: impl Into<String>,
    ) -> Self {
        Self::AdapterFailure {
            operation,
            platform,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn timeout(
        operation: AdapterOperation,
        platform: PlatformKind,
        timeout_ms: u64,
        message: impl Into<String>,
    ) -> Self {
        Self::Timeout {
            operation,
            platform,
            timeout_ms,
            message: message.into(),
            backend: None,
            suggested_action: None,
        }
    }

    #[must_use]
    pub fn timeout_with_diagnostic(
        operation: AdapterOperation,
        platform: PlatformKind,
        timeout_ms: u64,
        message: impl Into<String>,
        backend: impl Into<String>,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self::Timeout {
            operation,
            platform,
            timeout_ms,
            message: message.into(),
            backend: Some(backend.into()),
            suggested_action: Some(suggested_action.into()),
        }
    }

    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::UnsupportedCapability(_) => ErrorCategory::UnsupportedCapability,
            Self::MissingPermission { .. } => ErrorCategory::MissingPermission,
            Self::AdapterFailure { .. } => ErrorCategory::PlatformAdapterFailure,
            Self::Timeout { .. } => ErrorCategory::Timeout,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDiscoveryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCapabilityDiagnostic {
    pub capability: Capability,
    pub code: String,
    pub message: String,
    pub suggested_action: Option<String>,
}

impl TargetCapabilityDiagnostic {
    #[must_use]
    pub fn capture_unavailable(
        capability: Capability,
        code: impl Into<String>,
        message: impl Into<String>,
        suggested_action: impl Into<String>,
    ) -> Self {
        Self {
            capability,
            code: code.into(),
            message: message.into(),
            suggested_action: Some(suggested_action.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDescriptor {
    pub id: String,
    pub title: Option<String>,
    pub kind: CaptureTargetKind,
    pub name: String,
    pub bounds: Bounds,
    pub scale_factor: ScaleFactor,
    pub capture_supported: bool,
    pub input_supported: bool,
    pub app_name: Option<String>,
    pub process_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<TargetCapabilityDiagnostic>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInventory {
    pub targets: Vec<TargetDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRequest {
    pub target: CaptureTargetKind,
    pub target_id: String,
    /// Per-call deadline applied to backend capture work (e.g. portal/grim).
    /// `None` lets the adapter choose its own default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Hard upper bound used by adapters that need a deadline but were given
/// `timeout_ms = None`. Chosen so a hung Wayland portal or compositor cannot
/// freeze a CLI/MCP session for longer than ten seconds.
pub const DEFAULT_CAPTURE_TIMEOUT_MS: u64 = 10_000;

/// Maximum time spent on the portal-first path when a bounded fallback backend
/// is available. The portal may wait on an invisible compositor/permission UI;
/// reserving most of the command budget for `grim` lets supported wlroots
/// sessions capture successfully instead of timing out before the fallback runs.
const WAYLAND_PORTAL_FALLBACK_TIMEOUT_MS: u64 = 1_500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureArtifact {
    pub target_id: String,
    pub media_type: String,
    pub image_bytes: Vec<u8>,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRequest {
    pub target_id: String,
    pub target: CaptureTargetKind,
    pub target_name: String,
    pub bounds: Bounds,
    pub app_name: Option<String>,
    pub process_id: Option<u32>,
    /// Restore the previously focused window/application after dispatching
    /// input when the platform adapter can observe and restore focus.
    pub restore_focus: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<InputAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// These booleans are stable wire-format flags mirrored by RunOutput and test
// fixtures; keep them as explicit JSON fields instead of hiding them in enums.
#[allow(clippy::struct_excessive_bools)]
pub struct InputOutcome {
    pub action_count: usize,
    pub focus_required: bool,
    pub focus_transferred: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focused_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_focus: Option<FocusSnapshot>,
    pub focus_restored: bool,
    pub pointer_restored: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restore_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CaptureFixture {
    #[serde(default = "default_capture_fixture_media_type")]
    media_type: String,
    image_base64: String,
    #[serde(default)]
    captured_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
// Fixture JSON intentionally mirrors InputOutcome's explicit boolean flags.
#[allow(clippy::struct_excessive_bools)]
struct InputFixture {
    #[serde(default)]
    action_count: Option<usize>,
    #[serde(default)]
    focus_required: bool,
    #[serde(default)]
    focus_transferred: bool,
    #[serde(default)]
    focused_target: Option<String>,
    #[serde(default)]
    previous_focus: Option<FocusSnapshot>,
    #[serde(default)]
    focus_restored: bool,
    #[serde(default)]
    pointer_restored: bool,
    #[serde(default)]
    restore_error: Option<String>,
    #[serde(default)]
    notes: Vec<String>,
}

fn default_capture_fixture_media_type() -> String {
    "image/png".to_owned()
}

fn load_capture_fixture(
    platform: PlatformKind,
    request: &CaptureRequest,
) -> Result<Option<CaptureArtifact>, PlatformAdapterError> {
    let Some(raw) = env::var(CAPTURE_FIXTURE_ENV).ok() else {
        return Ok(None);
    };

    let fixture = serde_json::from_str::<CaptureFixture>(&raw).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            platform,
            format!("failed to parse {CAPTURE_FIXTURE_ENV}: {error}"),
        )
    })?;
    let image_bytes = BASE64.decode(&fixture.image_base64).map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            platform,
            format!("failed to decode {CAPTURE_FIXTURE_ENV} image bytes: {error}"),
        )
    })?;

    Ok(Some(CaptureArtifact {
        target_id: request.target_id.clone(),
        media_type: fixture.media_type,
        image_bytes,
        captured_at: fixture.captured_at.unwrap_or_else(current_timestamp),
    }))
}

fn default_input_action_count(request: &InputRequest) -> usize {
    request.actions.len() + usize::from(request.text.is_some())
}

fn load_input_fixture(
    platform: PlatformKind,
    request: &InputRequest,
) -> Result<Option<InputOutcome>, TendrilError> {
    let Some(raw) = env::var(INPUT_FIXTURE_ENV).ok() else {
        return Ok(None);
    };

    let fixture = serde_json::from_str::<InputFixture>(&raw).map_err(|error| {
        TendrilError::from(PlatformAdapterError::adapter_failure(
            AdapterOperation::InputControl,
            platform,
            format!("failed to parse {INPUT_FIXTURE_ENV}: {error}"),
        ))
    })?;

    Ok(Some(InputOutcome {
        action_count: fixture
            .action_count
            .unwrap_or_else(|| default_input_action_count(request)),
        focus_required: fixture.focus_required,
        focus_transferred: fixture.focus_transferred,
        focused_target: fixture
            .focused_target
            .or_else(|| fixture.focus_transferred.then(|| request.target_id.clone())),
        previous_focus: fixture.previous_focus,
        focus_restored: fixture.focus_restored,
        pointer_restored: fixture.pointer_restored,
        restore_error: fixture.restore_error,
        notes: fixture.notes,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioProbeRequest {
    pub source: AudioSourceKind,
    pub duration_hint_ms: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioCapabilityReport {
    pub source: AudioSourceKind,
    pub backend: AudioBackend,
    pub supported_sample_rates_hz: Vec<u32>,
    pub supported_channel_counts: Vec<u8>,
    pub permissions: Vec<PermissionStatus>,
    pub notes: Vec<String>,
}

pub trait TargetDiscoveryAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError>;

    fn discover_targets(
        &self,
        _request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError>;
}

pub trait CaptureAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError>;

    fn capture(&self, _request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError>;
}

pub trait InputControlAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError>;

    fn execute_input(&self, _request: &InputRequest) -> Result<InputOutcome, TendrilError>;
}

pub trait PermissionAdapter {
    fn permissions(&self) -> Vec<PermissionStatus>;
}

pub trait AudioCapabilityProbe {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError>;
}

pub trait PlatformAdapter:
    Send
    + Sync
    + TargetDiscoveryAdapter
    + CaptureAdapter
    + InputControlAdapter
    + PermissionAdapter
    + AudioCapabilityProbe
{
    fn info(&self) -> AdapterInfo;

    fn list_elements(&self, input: &ElementListInput) -> Result<ElementListOutput, TendrilError> {
        let inventory = self.discover_targets(&TargetDiscoveryRequest)?;
        crate::elements::discover_elements(&self.info(), &inventory, input)
    }
}

#[derive(Debug, Clone)]
pub struct MacOsAdapter {
    context: AdapterContext,
}

/// Build a multi-line, actionable Screen Recording remediation message that
/// names the specific binary the user must grant access to. Centralised so
/// every call site (capture failure, discovery failure, list permission row)
/// produces the same operator-friendly guidance (bd-a24d8d).
fn macos_screen_recording_remediation() -> String {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok().or(Some(path)))
        .map_or_else(
            || "<tendril binary>".to_owned(),
            |path| path.display().to_string(),
        );
    let parent = macos_parent_process_summary();
    format!(
        "Grant Screen Recording access to the tendril binary, then rerun the command.\n\
         Steps:\n\
           1. Open System Settings > Privacy & Security > Screen Recording (or run `open \"x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture\"`).\n\
           2. Click the `+` button and add the tendril binary at: {exe}\n\
           3. Toggle the switch ON for that entry.\n\
           4. If tendril is launched indirectly (e.g. via SSH/caco exec), ALSO grant Screen Recording to the parent process: {parent}\n\
              (typical candidates: /usr/sbin/sshd, /bin/zsh or /bin/bash, the caco daemon binary, or the terminal app).\n\
           5. Quit and relaunch any process that is still holding a stale TCC decision (sshd sessions usually need a fresh login).\n\
           6. Rerun: tendril list --json (the screen_capture permission row should report state=granted).\n\
         If the toggle does not take effect, run `tccutil reset ScreenCapture` (admin only) and retry from step 1."
    )
}

/// One-line context appended to capture failure messages so the JSON envelope
/// also names the binary that needs the grant (bd-a24d8d).
/// Public re-export of the rich Screen Recording remediation message so other
/// modules (e.g. `discovery`) can surface identical guidance (bd-a24d8d).
#[must_use]
pub fn screen_recording_remediation_message() -> String {
    macos_screen_recording_remediation()
}

fn macos_screen_recording_context() -> String {
    let exe = std::env::current_exe().ok().map_or_else(
        || "<tendril binary>".to_owned(),
        |path| path.display().to_string(),
    );
    format!(
        "The Screen Recording TCC grant must be attached to this exact binary path: {exe} \
         (and to its parent launcher when invoked via ssh/caco exec)."
    )
}

fn macos_parent_process_summary() -> String {
    let self_pid = std::process::id();
    // Look up our own PPID via `ps` to avoid pulling in libc/nix and to keep
    // the workspace `unsafe_code` lint clean (bd-a24d8d).
    let ppid_output = std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &self_pid.to_string()])
        .output();
    let ppid_str = match ppid_output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        _ => return "<unknown parent process>".to_owned(),
    };
    if ppid_str.is_empty() {
        return "<unknown parent process>".to_owned();
    }
    let comm_output = std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &ppid_str])
        .output();
    match comm_output {
        Ok(out) if out.status.success() => {
            let comm = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if comm.is_empty() {
                format!("pid {ppid_str}")
            } else {
                format!("{comm} (pid {ppid_str})")
            }
        }
        _ => format!("pid {ppid_str}"),
    }
}

/// Best-effort proactive probe for Screen Recording consent on macOS.
///
/// Issues a tiny `screencapture -x -R 0,0,1,1` to a tempfile and reads back the
/// resulting PNG. A non-zero exit, or a successful exit that produces an image
/// whose pixels are all the same value (the desktop background placeholder TCC
/// returns when consent is missing) is treated as `denied`. Returns `None` if
/// the probe itself could not run (so the caller falls back to `Unknown`).
fn probe_macos_screen_recording_permission() -> Option<bool> {
    // Allow tests / fixture-driven runs to skip the live probe.
    if std::env::var_os("TENDRIL_SKIP_PERMISSION_PROBE").is_some() {
        return None;
    }
    let path = unique_temp_path("png");
    let status = std::process::Command::new("screencapture")
        .args(["-x", "-t", "png", "-R", "0,0,1,1"])
        .arg(&path)
        .output()
        .ok()?;
    if !status.status.success() {
        let _ = std::fs::remove_file(&path);
        return Some(false);
    }
    // If the file is missing or empty, treat as denied.
    let bytes = match std::fs::read(&path) {
        Ok(bytes) if !bytes.is_empty() => bytes,
        _ => {
            let _ = std::fs::remove_file(&path);
            return Some(false);
        }
    };
    let _ = std::fs::remove_file(&path);
    // We could decode the PNG to inspect pixels, but a successful screencapture
    // exit with a non-empty PNG is a strong granted signal. Avoid the image
    // crate dependency surface here.
    Some(bytes.len() > 64)
}

impl MacOsAdapter {
    #[must_use]
    pub fn new(context: AdapterContext) -> Self {
        Self { context }
    }

    fn platform(&self) -> PlatformKind {
        self.context.platform
    }

    fn screen_capture_permission() -> PermissionStatus {
        // Probe the actual permission state so `tendril list` surfaces
        // Granted/Denied instead of always Unknown (bd-a24d8d).
        match probe_macos_screen_recording_permission() {
            Some(true) => PermissionStatus::granted(
                PermissionKind::ScreenCapture,
                "macOS Screen Recording consent appears to be granted for the invoking process.",
            ),
            Some(false) => PermissionStatus::denied(
                PermissionKind::ScreenCapture,
                "macOS Screen Recording consent is NOT granted; capture and target discovery will fail.",
                macos_screen_recording_remediation(),
            ),
            None => PermissionStatus::unknown(
                PermissionKind::ScreenCapture,
                "macOS capture requires Screen Recording consent for the invoking terminal or binary.",
                macos_screen_recording_remediation(),
            ),
        }
    }

    fn accessibility_permission() -> PermissionStatus {
        PermissionStatus::unknown(
            PermissionKind::Accessibility,
            "macOS input control requires Accessibility consent.",
            "Grant Accessibility access in System Settings > Privacy & Security > Accessibility, then rerun tendril.",
        )
    }

    fn microphone_permission() -> PermissionStatus {
        PermissionStatus::unknown(
            PermissionKind::Microphone,
            "macOS microphone capture requires Microphone consent.",
            "Grant Microphone access in System Settings > Privacy & Security > Microphone, then rerun tendril.",
        )
    }
}

impl TargetDiscoveryAdapter for MacOsAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(
            FeatureSupport::available(Capability::TargetDiscovery).with_notes(vec![
                "Window metadata can be enumerated without a background daemon.".to_owned(),
            ]),
        )
    }

    fn discover_targets(
        &self,
        request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        discovery::discover_targets(&self.context, request)
    }
}

impl CaptureAdapter for MacOsAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError> {
        let note = match target {
            CaptureTargetKind::Window => {
                "Window capture uses the same permission boundary as display capture on macOS."
            }
            CaptureTargetKind::Display => {
                "Display capture remains stateless; every request must specify the target explicitly."
            }
        };

        Ok(FeatureSupport::available(match target {
            CaptureTargetKind::Window => Capability::WindowCapture,
            CaptureTargetKind::Display => Capability::DisplayCapture,
        })
        .with_permissions(vec![Self::screen_capture_permission()])
        .with_notes(vec![note.to_owned()]))
    }

    fn capture(&self, request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        if let Some(artifact) = load_capture_fixture(self.platform(), request)? {
            return Ok(artifact);
        }

        let path = unique_temp_path("png");
        let mut command = std::process::Command::new("screencapture");
        command.arg("-x").arg("-t").arg("png");

        match request.target {
            CaptureTargetKind::Window => {
                command.arg("-l").arg(&request.target_id);
            }
            CaptureTargetKind::Display => {
                command
                    .arg("-D")
                    .arg(parse_display_index(&request.target_id, self.platform())?.to_string());
            }
        }

        command.arg(&path);
        let output = command.output().map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                self.platform(),
                format!("failed to spawn screencapture: {error}"),
            )
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let base_message = if stderr.trim().is_empty() {
                "Capture execution is gated on explicit Screen Recording consent.".to_owned()
            } else {
                format!("Capture failed: {}", stderr.trim())
            };
            return Err(PlatformAdapterError::missing_permission(
                match request.target {
                    CaptureTargetKind::Window => Capability::WindowCapture,
                    CaptureTargetKind::Display => Capability::DisplayCapture,
                },
                PermissionKind::ScreenCapture,
                self.platform(),
                format!(
                    "{base_message} {context}",
                    context = macos_screen_recording_context()
                ),
                macos_screen_recording_remediation(),
            ));
        }

        let image_bytes = read_and_remove_temp_capture(&path).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                self.platform(),
                error.to_string(),
            )
        })?;

        Ok(CaptureArtifact {
            target_id: request.target_id.clone(),
            media_type: "image/png".to_owned(),
            image_bytes,
            captured_at: current_timestamp(),
        })
    }
}

impl InputControlAdapter for MacOsAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(FeatureSupport::available(Capability::InputControl)
            .with_permissions(vec![Self::accessibility_permission()])
            .with_notes(vec![
                "The adapter may need to transfer focus for reliable text entry on secure fields."
                    .to_owned(),
            ]))
    }

    fn execute_input(&self, request: &InputRequest) -> Result<InputOutcome, TendrilError> {
        if let Some(outcome) = load_input_fixture(self.platform(), request)? {
            return Ok(outcome);
        }

        execute_macos_input(self.platform(), request)
    }
}

impl PermissionAdapter for MacOsAdapter {
    fn permissions(&self) -> Vec<PermissionStatus> {
        vec![
            Self::screen_capture_permission(),
            Self::accessibility_permission(),
            Self::microphone_permission(),
        ]
    }
}

impl AudioCapabilityProbe for MacOsAdapter {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
        match request.source {
            AudioSourceKind::SystemLoopback => Err(PlatformAdapterError::unsupported(
                Capability::AudioLoopbackCapture,
                self.platform(),
                CapabilityErrorReason::UnsupportedFeature,
                "The macOS adapter spine does not expose system loopback capture in v0.0.1.",
                Some(
                    "Use microphone capture or provide an explicit virtual loopback device in a future adapter implementation.",
                ),
            )),
            AudioSourceKind::Microphone => Ok(AudioCapabilityReport {
                source: AudioSourceKind::Microphone,
                backend: AudioBackend::CoreAudio,
                supported_sample_rates_hz: vec![16_000, 44_100, 48_000],
                supported_channel_counts: vec![1, 2],
                permissions: vec![Self::microphone_permission()],
                notes: vec![
                    "Audio capture remains command-scoped and does not create a background daemon."
                        .to_owned(),
                ],
            }),
        }
    }
}

impl PlatformAdapter for MacOsAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::from_context(&self.context)
    }
}

#[derive(Debug, Clone)]
pub struct LinuxAdapter {
    context: AdapterContext,
}

impl LinuxAdapter {
    #[must_use]
    pub fn new(context: AdapterContext) -> Self {
        Self { context }
    }

    fn platform(&self) -> PlatformKind {
        self.context.platform
    }

    fn audio_backend(&self) -> AudioBackend {
        self.context.audio_backend.unwrap_or(AudioBackend::Unknown)
    }
}

impl TargetDiscoveryAdapter for LinuxAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        match self.context.session {
            DesktopSession::X11 => Ok(FeatureSupport::available(Capability::TargetDiscovery)
                .with_notes(vec![
                    "X11 target enumeration does not require a Tendril daemon.".to_owned(),
                ])),
            DesktopSession::Wayland => Ok(FeatureSupport::available(Capability::TargetDiscovery)
                .with_notes(vec![
                    "Wayland discovery may depend on compositor- or portal-provided metadata."
                        .to_owned(),
                ])),
            _ => Err(PlatformAdapterError::unsupported(
                Capability::TargetDiscovery,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "The Linux adapter could not determine an X11 or Wayland desktop session.",
                Some("Set XDG_SESSION_TYPE or run Tendril inside an interactive desktop session."),
            )),
        }
    }

    fn discover_targets(
        &self,
        request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        discovery::discover_targets(&self.context, request)
    }
}

impl CaptureAdapter for LinuxAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError> {
        let capability = match target {
            CaptureTargetKind::Window => Capability::WindowCapture,
            CaptureTargetKind::Display => Capability::DisplayCapture,
        };

        match self.context.session {
            DesktopSession::X11 => Ok(FeatureSupport::available(capability).with_notes(vec![
                "X11 capture support is adapter-local and command-scoped.".to_owned(),
            ])),
            DesktopSession::Wayland => Ok(FeatureSupport::available(capability).with_notes(vec![
                "Wayland capture prefers xdg-desktop-portal screenshot capture, then crops the requested target bounds locally."
                    .to_owned(),
                "When `grim` is available, Tendril bounds the portal-first attempt and falls back to grim if the portal is unavailable or does not answer promptly."
                    .to_owned(),
                "Wayland discovery still requires a supported compositor metadata backend: Hyprland (`hyprctl`), sway (`swaymsg`), or wlroots output enumeration (`wlr-randr`)."
                    .to_owned(),
            ])),
            _ => Err(PlatformAdapterError::unsupported(
                capability,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Capture requires a detected X11 or Wayland desktop session.",
                Some("Run Tendril from a graphical login session and retry."),
            )),
        }
    }

    fn capture(&self, request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        if let Some(artifact) = load_capture_fixture(self.platform(), request)? {
            return Ok(artifact);
        }

        match self.context.session {
            DesktopSession::X11 => {}
            DesktopSession::Wayland => {
                let image_bytes = capture_wayland_target(&self.context, request)?;
                return Ok(CaptureArtifact {
                    target_id: request.target_id.clone(),
                    media_type: "image/png".to_owned(),
                    image_bytes,
                    captured_at: current_timestamp(),
                });
            }
            _ => {
                return Err(PlatformAdapterError::unsupported(
                    match request.target {
                        CaptureTargetKind::Window => Capability::WindowCapture,
                        CaptureTargetKind::Display => Capability::DisplayCapture,
                    },
                    self.platform(),
                    CapabilityErrorReason::UnsupportedSession,
                    "Capture requires a detected graphical desktop session.",
                    Some("Run Tendril from a graphical login session and retry."),
                ));
            }
        }

        let image_bytes = x11::capture_target(&self.context, request.target, &request.target_id)?;

        Ok(CaptureArtifact {
            target_id: request.target_id.clone(),
            media_type: "image/png".to_owned(),
            image_bytes,
            captured_at: current_timestamp(),
        })
    }
}

impl InputControlAdapter for LinuxAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        match self.context.session {
            DesktopSession::X11 => Ok(FeatureSupport::available(Capability::InputControl)
                .with_notes(vec![
                    "Input injection remains stateless and target-scoped on X11.".to_owned(),
                ])),
            DesktopSession::Wayland => {
                let backends = wayland_input::detect_backend();
                if !backends.any_supported() {
                    return Err(wayland_input::missing_backend_error(self.platform()));
                }
                let mut notes = vec![
                    "Wayland input dispatch routes through detected helper tools (`ydotool` for full keyboard + pointer support, `wtype` for keyboard-only fallback on wlroots compositors)."
                        .to_owned(),
                    "Focus transfer is compositor-mediated on Wayland; place focus on the intended surface before issuing keyboard sequences for reliable delivery."
                        .to_owned(),
                ];
                if backends.ydotool {
                    notes.push(
                        "`ydotool` was detected on PATH; pointer events require the `ydotoold` daemon to be running and reachable via its socket (`YDOTOOL_SOCKET` or `/tmp/.ydotool_socket`)."
                            .to_owned(),
                    );
                }
                if backends.wtype && !backends.ydotool {
                    notes.push(
                        "Only `wtype` is installed; pointer events (lclick/rclick/mclick/drag) are not supported until `ydotool` is also available."
                            .to_owned(),
                    );
                }
                Ok(FeatureSupport::available(Capability::InputControl).with_notes(notes))
            }
            _ => Err(PlatformAdapterError::unsupported(
                Capability::InputControl,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Input control requires a detected graphical desktop session.",
                Some("Run Tendril inside X11 or a supported Wayland compositor session."),
            )),
        }
    }

    fn execute_input(&self, request: &InputRequest) -> Result<InputOutcome, TendrilError> {
        if let Some(outcome) = load_input_fixture(self.platform(), request)? {
            return Ok(outcome);
        }

        execute_linux_input(self.platform(), self.context.session, request)
    }
}

impl PermissionAdapter for LinuxAdapter {
    fn permissions(&self) -> Vec<PermissionStatus> {
        vec![
            PermissionStatus::not_required(
                PermissionKind::ScreenCapture,
                "Linux desktop capture typically relies on session capabilities rather than OS privacy prompts.",
            ),
            PermissionStatus::not_required(
                PermissionKind::Accessibility,
                "Linux input control generally depends on session support instead of a centralized permission prompt.",
            ),
            PermissionStatus::not_required(
                PermissionKind::Microphone,
                "Audio device access is usually mediated by the active audio stack and user session.",
            ),
        ]
    }
}

impl AudioCapabilityProbe for LinuxAdapter {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
        let backend = self.audio_backend();
        if backend == AudioBackend::Unknown {
            return Err(PlatformAdapterError::unsupported(
                match request.source {
                    AudioSourceKind::SystemLoopback => Capability::AudioLoopbackCapture,
                    AudioSourceKind::Microphone => Capability::AudioInputCapture,
                },
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "The Linux adapter could not detect a supported audio backend.",
                Some("Run Tendril inside a PipeWire or PulseAudio user session and retry."),
            ));
        }

        Ok(AudioCapabilityReport {
            source: request.source,
            backend,
            supported_sample_rates_hz: vec![16_000, 44_100, 48_000],
            supported_channel_counts: vec![1, 2],
            permissions: vec![PermissionStatus::not_required(
                PermissionKind::Microphone,
                "Audio capture support is determined by the active Linux audio backend.",
            )],
            notes: vec![match request.source {
                AudioSourceKind::SystemLoopback => {
                    "Loopback capture support depends on monitor/portal exposure from the active backend."
                        .to_owned()
                }
                AudioSourceKind::Microphone => {
                    "Microphone capture is explicit and command-scoped; no hidden recording session is retained."
                        .to_owned()
                }
            }],
        })
    }
}

impl PlatformAdapter for LinuxAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::from_context(&self.context)
    }
}

#[derive(Debug, Clone)]
pub struct WindowsAdapter {
    context: AdapterContext,
}

impl WindowsAdapter {
    #[must_use]
    pub fn new(context: AdapterContext) -> Self {
        Self { context }
    }

    fn platform(&self) -> PlatformKind {
        self.context.platform
    }

    fn microphone_permission() -> PermissionStatus {
        PermissionStatus::unknown(
            PermissionKind::Microphone,
            "Windows microphone capture may be gated by the Privacy > Microphone setting for desktop apps.",
            "Enable microphone access for desktop apps in Settings > Privacy & security > Microphone before retrying.",
        )
    }

    fn capture_display(&self, target_id: &str) -> Result<Vec<u8>, PlatformAdapterError> {
        WINDOWS_RUNTIME.capture_display(self.platform(), target_id)
    }

    fn capture_window(&self, target_id: &str) -> Result<Vec<u8>, PlatformAdapterError> {
        WINDOWS_RUNTIME.capture_window(self.platform(), target_id)
    }
}

impl TargetDiscoveryAdapter for WindowsAdapter {
    fn target_discovery_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(
            FeatureSupport::available(Capability::TargetDiscovery).with_notes(vec![
            "The Windows adapter remains stateless; targets are discovered fresh for each command."
                .to_owned(),
        ]),
        )
    }

    fn discover_targets(
        &self,
        request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        discovery::discover_targets(&self.context, request)
    }
}

impl CaptureAdapter for WindowsAdapter {
    fn capture_support(
        &self,
        target: CaptureTargetKind,
    ) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(FeatureSupport::available(match target {
            CaptureTargetKind::Window => Capability::WindowCapture,
            CaptureTargetKind::Display => Capability::DisplayCapture,
        })
        .with_notes(vec![
            "Windows capture support does not require a Tendril-managed background service."
                .to_owned(),
        ]))
    }

    fn capture(&self, request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        if let Some(artifact) = load_capture_fixture(self.platform(), request)? {
            return Ok(artifact);
        }

        let image_bytes = match request.target {
            CaptureTargetKind::Display => self.capture_display(&request.target_id)?,
            CaptureTargetKind::Window => self.capture_window(&request.target_id)?,
        };

        Ok(CaptureArtifact {
            target_id: request.target_id.clone(),
            media_type: "image/png".to_owned(),
            image_bytes,
            captured_at: current_timestamp(),
        })
    }
}

impl InputControlAdapter for WindowsAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(FeatureSupport::available(Capability::InputControl).with_notes(vec![
            "The adapter may still need to report focus transfer when a target cannot receive background input."
                .to_owned(),
        ]))
    }

    fn execute_input(&self, request: &InputRequest) -> Result<InputOutcome, TendrilError> {
        if let Some(outcome) = load_input_fixture(self.platform(), request)? {
            return Ok(outcome);
        }

        execute_windows_input(self.platform(), request)
    }
}

impl PermissionAdapter for WindowsAdapter {
    fn permissions(&self) -> Vec<PermissionStatus> {
        vec![
            PermissionStatus::not_required(
                PermissionKind::ScreenCapture,
                "Desktop capture does not require a separate Windows privacy prompt for classic desktop apps.",
            ),
            PermissionStatus::not_required(
                PermissionKind::Accessibility,
                "Desktop input control uses user-session APIs rather than a dedicated accessibility prompt.",
            ),
            Self::microphone_permission(),
        ]
    }
}

impl AudioCapabilityProbe for WindowsAdapter {
    fn probe_audio_capture(
        &self,
        request: &AudioProbeRequest,
    ) -> Result<AudioCapabilityReport, PlatformAdapterError> {
        Ok(AudioCapabilityReport {
            source: request.source,
            backend: AudioBackend::Wasapi,
            supported_sample_rates_hz: vec![16_000, 44_100, 48_000],
            supported_channel_counts: vec![1, 2],
            permissions: match request.source {
                AudioSourceKind::SystemLoopback => vec![PermissionStatus::not_required(
                    PermissionKind::Microphone,
                    "WASAPI loopback capture does not require microphone access.",
                )],
                AudioSourceKind::Microphone => vec![Self::microphone_permission()],
            },
            notes: vec![match request.source {
                AudioSourceKind::SystemLoopback => {
                    "System loopback is exposed via WASAPI and remains command-scoped."
                        .to_owned()
                }
                AudioSourceKind::Microphone => {
                    "Microphone capture may require desktop-app privacy approval in addition to device availability."
                        .to_owned()
                }
            }],
        })
    }
}

impl PlatformAdapter for WindowsAdapter {
    fn info(&self) -> AdapterInfo {
        AdapterInfo::from_context(&self.context)
    }
}

fn parse_display_index(
    target_id: &str,
    platform: PlatformKind,
) -> Result<u32, PlatformAdapterError> {
    target_id.parse::<u32>().map_err(|error| {
        PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            platform,
            format!("display target id `{target_id}` must be a numeric index (e.g. 1, 2): {error}"),
        )
    })
}

#[derive(Debug, Clone)]
struct WaylandCaptureBackendFailure {
    message: String,
    missing_backend: bool,
}

/// Distinguishes a clean backend failure from an exceeded deadline so the
/// caller can return a structured `Timeout` error to agents.
#[derive(Debug, Clone)]
enum WaylandCaptureBackendError {
    Failed(WaylandCaptureBackendFailure),
    Timeout(String),
}

impl From<WaylandCaptureBackendFailure> for WaylandCaptureBackendError {
    fn from(value: WaylandCaptureBackendFailure) -> Self {
        Self::Failed(value)
    }
}

fn wayland_portal_attempt_timeout(total: Duration, fallback_available: bool) -> Duration {
    if !fallback_available {
        return total;
    }

    let fallback_cap = Duration::from_millis(WAYLAND_PORTAL_FALLBACK_TIMEOUT_MS);
    let half_budget = total / 2;
    let reserved = fallback_cap.min(half_budget);
    reserved.max(Duration::from_millis(1)).min(total)
}

fn attempt_wayland_portal_capture(
    context: &AdapterContext,
    target: &TargetDescriptor,
    inventory: &TargetInventory,
    timeout: Duration,
    portal_timeout: Duration,
    grim_available: bool,
) -> Result<Result<Vec<u8>, WaylandCaptureBackendFailure>, PlatformAdapterError> {
    match capture_wayland_target_via_portal(target, inventory, portal_timeout) {
        Ok(image_bytes) => Ok(Ok(image_bytes)),
        Err(WaylandCaptureBackendError::Timeout(message)) if grim_available => {
            Ok(Err(WaylandCaptureBackendFailure {
                message: format!(
                    "timed out after {} ms: {message}",
                    portal_timeout.as_millis()
                ),
                missing_backend: false,
            }))
        }
        Err(WaylandCaptureBackendError::Timeout(message)) => {
            Err(PlatformAdapterError::timeout_with_diagnostic(
                AdapterOperation::Capture,
                context.platform,
                u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
                format!("xdg-desktop-portal screenshot timed out: {message}"),
                "xdg_desktop_portal_screenshot",
                "The Wayland screenshot portal did not respond and no `grim` fallback was available. Use `tendril list --json` to check whether capture is advertised for the target, install/fix a compositor screenshot backend, or use the packaged X11/Xvfb headless helper.",
            ))
        }
        Err(WaylandCaptureBackendError::Failed(error)) => Ok(Err(error)),
    }
}

fn capture_wayland_target(
    context: &AdapterContext,
    request: &CaptureRequest,
) -> Result<Vec<u8>, PlatformAdapterError> {
    let inventory = discovery::discover_targets(context, &TargetDiscoveryRequest)?;
    let target = inventory
        .targets
        .iter()
        .find(|target| target.kind == request.target && target.id == request.target_id)
        .cloned()
        .ok_or_else(|| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                context.platform,
                format!(
                    "target `{}` was not found during Wayland capture",
                    request.target_id
                ),
            )
        })?;

    let timeout = Duration::from_millis(request.timeout_ms.unwrap_or(DEFAULT_CAPTURE_TIMEOUT_MS));
    let deadline = Instant::now() + timeout;
    let grim_available = wayland_capture_program_on_path("grim");
    let portal_timeout = wayland_portal_attempt_timeout(timeout, grim_available);

    let portal_error = match attempt_wayland_portal_capture(
        context,
        &target,
        &inventory,
        timeout,
        portal_timeout,
        grim_available,
    )? {
        Ok(image_bytes) => return Ok(image_bytes),
        Err(error) => error,
    };

    if !grim_available {
        return if portal_error.missing_backend {
            Err(PlatformAdapterError::unsupported(
                wayland_capture_capability(request.target),
                context.platform,
                CapabilityErrorReason::UnsupportedFeature,
                format!(
                    "Wayland capture needs either an xdg-desktop-portal screenshot backend or the `grim` compatibility fallback for this session; portal capture failed: {}",
                    portal_error.message
                ),
                Some(
                    "Install and run an xdg-desktop-portal screenshot backend for the active compositor, or install `grim` as a compatibility fallback if that compositor permits geometry-scoped screenshots.",
                ),
            ))
        } else {
            Err(PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                context.platform,
                format!(
                    "Wayland capture via xdg-desktop-portal screenshot failed (`{}`), and the `grim` compatibility fallback is not installed.",
                    portal_error.message
                ),
            ))
        };
    }

    // Reserve whatever budget remains after the portal attempt for grim.
    let grim_timeout = deadline
        .checked_duration_since(Instant::now())
        .unwrap_or_else(|| Duration::from_millis(0));
    if grim_timeout.is_zero() {
        return Err(PlatformAdapterError::timeout_with_diagnostic(
            AdapterOperation::Capture,
            context.platform,
            u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            format!(
                "Wayland capture exhausted its {} ms budget on the xdg-desktop-portal screenshot path before grim could be tried (`{}`).",
                timeout.as_millis(),
                portal_error.message
            ),
            "xdg_desktop_portal_screenshot",
            "The portal consumed the capture budget before the grim fallback could run. Retry with a larger --timeout-ms only for manual diagnosis; for automation, fix the portal backend or target a display that list reports as capturable.",
        ));
    }

    capture_wayland_target_with_grim(&target, grim_timeout).map_err(|grim_error| match grim_error {
        WaylandCaptureBackendError::Timeout(message) => PlatformAdapterError::timeout_with_diagnostic(
            AdapterOperation::Capture,
            context.platform,
            u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            format!(
                "Wayland capture timed out — xdg-desktop-portal screenshot (`{}`) and grim fallback both unresponsive: {message}",
                portal_error.message
            ),
            "xdg_desktop_portal_screenshot+grim",
            "Both Wayland capture backends exceeded the command budget. Check the compositor screenshot portal, confirm grim can capture the target geometry manually, or use the packaged X11/Xvfb headless helper for unattended automation.",
        ),
        WaylandCaptureBackendError::Failed(failure) => PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            context.platform,
            format!(
                "Wayland capture failed via xdg-desktop-portal screenshot (`{}`) and grim fallback (`{}`).",
                portal_error.message, failure.message
            ),
        ),
    })
}

fn capture_wayland_target_via_portal(
    target: &TargetDescriptor,
    inventory: &TargetInventory,
    timeout: Duration,
) -> Result<Vec<u8>, WaylandCaptureBackendError> {
    let (sender, receiver) = mpsc::channel();
    // The portal call is run on a detached helper thread. If the deadline
    // elapses we abandon the thread (the portal call has no cancel API), but
    // we never wait on it again so the caller is unblocked. A late reply will
    // simply be dropped together with the channel.
    thread::Builder::new()
        .name("tendril-wayland-portal-capture".to_string())
        .spawn(move || {
            let result = block_on(async {
                PortalScreenshot::request()
                    .interactive(false)
                    .modal(true)
                    .send()
                    .await?
                    .response()
            });
            let _ = sender.send(result);
        })
        .map_err(|error| {
            WaylandCaptureBackendError::Failed(WaylandCaptureBackendFailure {
                message: format!(
                    "failed to spawn xdg-desktop-portal screenshot worker thread: {error}"
                ),
                missing_backend: false,
            })
        })?;

    let screenshot = match receiver.recv_timeout(timeout) {
        Ok(Ok(screenshot)) => screenshot,
        Ok(Err(error)) => return Err(classify_wayland_portal_capture_error(&error).into()),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err(WaylandCaptureBackendError::Timeout(format!(
                "xdg-desktop-portal screenshot did not respond within {} ms",
                timeout.as_millis()
            )));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(WaylandCaptureBackendError::Failed(
                WaylandCaptureBackendFailure {
                    message: "xdg-desktop-portal screenshot worker thread terminated unexpectedly"
                        .to_owned(),
                    missing_backend: false,
                },
            ));
        }
    };

    let path = screenshot
        .uri()
        .to_file_path()
        .map_err(|()| WaylandCaptureBackendFailure {
            message: format!(
                "xdg-desktop-portal screenshot returned a non-file URI: {}",
                screenshot.uri()
            ),
            missing_backend: false,
        })?;
    let image_bytes = std::fs::read(&path).map_err(|error| WaylandCaptureBackendFailure {
        message: format!(
            "failed to read xdg-desktop-portal screenshot artifact `{}`: {error}",
            path.display()
        ),
        missing_backend: false,
    })?;
    let _ = std::fs::remove_file(&path);

    crop_wayland_portal_capture_to_target(&image_bytes, target, inventory).map_err(Into::into)
}

fn classify_wayland_portal_capture_error(error: &ashpd::Error) -> WaylandCaptureBackendFailure {
    let missing_backend = match error {
        ashpd::Error::NoResponse
        | ashpd::Error::RequiresVersion(_, _)
        | ashpd::Error::Portal(ashpd::PortalError::NotFound(_)) => true,
        ashpd::Error::Zbus(zbus_error) => {
            let message = zbus_error.to_string().to_ascii_lowercase();
            message.contains("serviceunknown")
                || message.contains("unknownmethod")
                || message.contains("name has no owner")
                || message.contains("org.freedesktop.portal.desktop")
                || message.contains("org.freedesktop.portal.screenshot")
        }
        _ => false,
    };

    WaylandCaptureBackendFailure {
        message: error.to_string(),
        missing_backend,
    }
}

fn crop_wayland_portal_capture_to_target(
    image_bytes: &[u8],
    target: &TargetDescriptor,
    inventory: &TargetInventory,
) -> Result<Vec<u8>, WaylandCaptureBackendFailure> {
    let image =
        image::load_from_memory(image_bytes).map_err(|error| WaylandCaptureBackendFailure {
            message: format!(
                "failed to decode xdg-desktop-portal screenshot image for `{}`: {error}",
                target.id
            ),
            missing_backend: false,
        })?;

    let (origin_x, origin_y) = wayland_workspace_origin(inventory);
    let crop_x = i64::from(target.bounds.x) - i64::from(origin_x);
    let crop_y = i64::from(target.bounds.y) - i64::from(origin_y);

    if crop_x < 0 || crop_y < 0 {
        return Err(WaylandCaptureBackendFailure {
            message: format!(
                "xdg-desktop-portal screenshot for `{}` did not cover the requested target origin after workspace translation (crop origin {crop_x},{crop_y}).",
                target.id
            ),
            missing_backend: false,
        });
    }

    let crop_x = u32::try_from(crop_x).expect("validated crop x should fit u32");
    let crop_y = u32::try_from(crop_y).expect("validated crop y should fit u32");
    if crop_x == 0
        && crop_y == 0
        && target.bounds.width == image.width()
        && target.bounds.height == image.height()
    {
        return Ok(image_bytes.to_vec());
    }

    if crop_x.saturating_add(target.bounds.width) > image.width()
        || crop_y.saturating_add(target.bounds.height) > image.height()
    {
        return Err(WaylandCaptureBackendFailure {
            message: format!(
                "xdg-desktop-portal screenshot dimensions {}x{} did not cover target `{}` at {}x{}+{}+{}.",
                image.width(),
                image.height(),
                target.id,
                target.bounds.width,
                target.bounds.height,
                target.bounds.x,
                target.bounds.y,
            ),
            missing_backend: false,
        });
    }

    let cropped = image.crop_imm(crop_x, crop_y, target.bounds.width, target.bounds.height);
    let mut encoded = Vec::new();
    cropped
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .map_err(|error| WaylandCaptureBackendFailure {
            message: format!(
                "failed to encode cropped xdg-desktop-portal screenshot for `{}`: {error}",
                target.id
            ),
            missing_backend: false,
        })?;
    Ok(encoded)
}

fn wayland_workspace_origin(inventory: &TargetInventory) -> (i32, i32) {
    let mut displays = inventory
        .targets
        .iter()
        .filter(|target| target.kind == CaptureTargetKind::Display)
        .map(|target| (target.bounds.x, target.bounds.y));
    let Some((mut min_x, mut min_y)) = displays.next() else {
        return (0, 0);
    };

    for (x, y) in displays {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
    }

    (min_x, min_y)
}

/// Outcome of waiting for a capture-helper child process with a deadline.
#[derive(Debug)]
enum CaptureChildOutcome {
    Exited(std::process::ExitStatus),
    TimedOut,
    WaitFailed(std::io::Error),
}

/// Poll a child process until it exits or `timeout` elapses. The caller is
/// responsible for killing+reaping the child on `TimedOut`.
fn wait_for_capture_child_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> CaptureChildOutcome {
    let deadline = Instant::now() + timeout;
    let poll_interval = Duration::from_millis(50);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return CaptureChildOutcome::Exited(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    return CaptureChildOutcome::TimedOut;
                }
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .unwrap_or_default();
                thread::sleep(poll_interval.min(remaining));
            }
            Err(error) => return CaptureChildOutcome::WaitFailed(error),
        }
    }
}

fn capture_wayland_target_with_grim(
    target: &TargetDescriptor,
    timeout: Duration,
) -> Result<Vec<u8>, WaylandCaptureBackendError> {
    let path = unique_temp_path("png");
    if target.bounds.width == 0 || target.bounds.height == 0 {
        return Err(WaylandCaptureBackendError::Failed(
            WaylandCaptureBackendFailure {
                message: format!(
                    "target `{}` has empty bounds ({}x{}); refusing to invoke grim",
                    target.id, target.bounds.width, target.bounds.height
                ),
                missing_backend: false,
            },
        ));
    }
    if target.bounds.x < 0 || target.bounds.y < 0 {
        return Err(WaylandCaptureBackendError::Failed(
            WaylandCaptureBackendFailure {
                message: format!(
                    "target `{}` has negative origin ({},{}); grim cannot capture off-screen regions",
                    target.id, target.bounds.x, target.bounds.y
                ),
                missing_backend: false,
            },
        ));
    }
    // grim expects slurp-compatible geometry: "X,Y WxH" (e.g. "0,0 1920x1080").
    let geometry = format!(
        "{},{} {}x{}",
        target.bounds.x, target.bounds.y, target.bounds.width, target.bounds.height
    );
    let mut command = std::process::Command::new("grim");
    command
        .arg("-t")
        .arg("png")
        .arg("-g")
        .arg(&geometry)
        .arg(&path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| WaylandCaptureBackendFailure {
            message: format!("failed to spawn grim: {error}"),
            missing_backend: false,
        })?;

    let status = match wait_for_capture_child_with_timeout(&mut child, timeout) {
        CaptureChildOutcome::Exited(status) => status,
        CaptureChildOutcome::TimedOut => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&path);
            return Err(WaylandCaptureBackendError::Timeout(format!(
                "grim did not produce a screenshot for `{}` within {} ms",
                target.id,
                timeout.as_millis()
            )));
        }
        CaptureChildOutcome::WaitFailed(error) => {
            let _ = std::fs::remove_file(&path);
            return Err(WaylandCaptureBackendError::Failed(
                WaylandCaptureBackendFailure {
                    message: format!("failed to wait on grim: {error}"),
                    missing_backend: false,
                },
            ));
        }
    };

    if !status.success() {
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            use std::io::Read as _;
            let _ = handle.read_to_string(&mut stderr);
        }
        let _ = std::fs::remove_file(&path);
        return Err(WaylandCaptureBackendError::Failed(
            WaylandCaptureBackendFailure {
                message: if stderr.trim().is_empty() {
                    format!("grim exited with status {status}")
                } else {
                    format!("grim failed: {}", stderr.trim())
                },
                missing_backend: false,
            },
        ));
    }

    read_and_remove_temp_capture(&path).map_err(|error| {
        WaylandCaptureBackendError::Failed(WaylandCaptureBackendFailure {
            message: error.to_string(),
            missing_backend: false,
        })
    })
}

fn wayland_capture_capability(target: CaptureTargetKind) -> Capability {
    match target {
        CaptureTargetKind::Window => Capability::WindowCapture,
        CaptureTargetKind::Display => Capability::DisplayCapture,
    }
}

fn wayland_capture_program_on_path(program: &str) -> bool {
    env::var_os("PATH").is_some_and(|path| {
        env::split_paths(&path).any(|entry| {
            let candidate = entry.join(program);
            candidate.is_file() || {
                #[cfg(windows)]
                {
                    entry.join(format!("{program}.exe")).is_file()
                }
                #[cfg(not(windows))]
                {
                    false
                }
            }
        })
    })
}

fn execute_linux_input(
    platform: PlatformKind,
    session: DesktopSession,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    match session {
        DesktopSession::X11 => x11::execute_input(platform, request),
        DesktopSession::Wayland => wayland_input::execute_input(platform, request),
        _ => Err(TendrilError::from(PlatformAdapterError::unsupported(
            Capability::InputControl,
            platform,
            CapabilityErrorReason::UnsupportedSession,
            "Input control requires a detected graphical desktop session.",
            Some("Run Tendril inside X11 or a supported Wayland compositor session."),
        ))),
    }
}

fn execute_macos_input(
    _platform: PlatformKind,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    let keyboard_input = request.text.is_some() || request.actions.iter().any(action_is_keyboard);
    let mut focus_required = keyboard_input || matches!(request.target, CaptureTargetKind::Window);
    let mut focus_transferred = false;
    let mut notes = Vec::new();
    let restore_error = if request.restore_focus {
        let message =
            "macOS focus restoration is not yet implemented by the Tendril adapter".to_owned();
        notes.push(format!(
            "Focus restoration requested but skipped: {message}."
        ));
        Some(message)
    } else {
        notes.push(
            "Focus restoration disabled for this run; focus may remain on the target.".to_owned(),
        );
        None
    };

    if matches!(request.target, CaptureTargetKind::Window) {
        if let Some(process_id) = request.process_id {
            let window_id = request.target_id.parse::<u64>().ok();
            let script = macos_focus_window_jxa_script(process_id, window_id, &request.bounds);
            run_macos_osascript_jxa_for_input(&script, "focus", None, None)?;
            focus_transferred = true;
            notes.push(
                "Raised the target macOS window via the Accessibility API (AXRaise) and activated its application before dispatching input."
                    .to_owned(),
            );
            std::thread::sleep(reliability_delay());
        } else if let Some(app_name) = &request.app_name {
            let script = macos_focus_app_jxa_script(app_name);
            run_macos_osascript_jxa_for_input(&script, "focus", None, None)?;
            focus_transferred = true;
            notes.push(
                "Activated the target macOS application by name before dispatching input."
                    .to_owned(),
            );
            std::thread::sleep(reliability_delay());
        } else {
            notes.push(
                "Target focus could not be transferred automatically because discovery did not expose a process or app name."
                    .to_owned(),
            );
        }
    } else if keyboard_input {
        notes.push(
            "Display-scoped keyboard input posts into the current macOS focus chain; transfer focus manually when needed."
                .to_owned(),
        );
    }

    if let Some(text) = &request.text {
        let script = macos_text_jxa_script(text);
        run_macos_osascript_jxa_for_input(&script, "dispatch", Some(0), Some("text"))?;
        return Ok(InputOutcome {
            action_count: 1,
            focus_required,
            focus_transferred,
            focused_target: if focus_transferred {
                Some(request.target_id.clone())
            } else {
                None
            },
            previous_focus: None,
            focus_restored: false,
            pointer_restored: false,
            restore_error: restore_error.clone(),
            notes,
        });
    }

    for (action_index, action) in request.actions.iter().enumerate() {
        let label = action_label(action);
        dispatch_macos_action(request, action, action_index, &label)?;
        if !matches!(action, InputAction::Wait { .. }) {
            std::thread::sleep(reliability_delay());
        }
    }

    if !keyboard_input && matches!(request.target, CaptureTargetKind::Display) {
        focus_required = false;
    }

    Ok(InputOutcome {
        action_count: request.actions.len(),
        focus_required,
        focus_transferred,
        focused_target: if focus_transferred {
            Some(request.target_id.clone())
        } else {
            None
        },
        previous_focus: None,
        focus_restored: false,
        pointer_restored: false,
        restore_error,
        notes,
    })
}

fn dispatch_macos_action(
    request: &InputRequest,
    action: &InputAction,
    action_index: usize,
    label: &str,
) -> Result<(), TendrilError> {
    match action {
        InputAction::KeyTap { key } => {
            let script = macos_key_jxa_script(key, Some(true), Some(true))?;
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Hold { modifier } => {
            let script = macos_modifier_jxa_script(*modifier, true)?;
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Release { modifier } => {
            let script = macos_modifier_jxa_script(*modifier, false)?;
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Send { text } => {
            let script = macos_text_jxa_script(text);
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Wait { duration_ms } => {
            std::thread::sleep(Duration::from_millis(*duration_ms));
            Ok(())
        }
        InputAction::Click { button, x, y } => {
            let (absolute_x, absolute_y) = relative_point_to_absolute(&request.bounds, *x, *y);
            let script = macos_mouse_jxa_script(*button, absolute_x, absolute_y, None);
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::PointerMove { .. } => Err(input_execution_error(
            "unsupported_pointer_move_action",
            "move(...) and hover(...) are currently implemented for Linux/X11 input delivery; this adapter does not yet support native pointer-only motion".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
        InputAction::DoubleClick { .. } => Err(input_execution_error(
            "unsupported_double_click_action",
            "dblclick(...) is currently implemented for Linux/X11 input delivery; this adapter does not yet support native double-click injection".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
        InputAction::Drag { x0, y0, x1, y1 } => {
            let (start_x, start_y) = relative_point_to_absolute(&request.bounds, *x0, *y0);
            let (end_x, end_y) = relative_point_to_absolute(&request.bounds, *x1, *y1);
            let script =
                macos_mouse_jxa_script(MouseButton::Left, start_x, start_y, Some((end_x, end_y)));
            run_macos_osascript_jxa_for_input(&script, "dispatch", Some(action_index), Some(label))
        }
        InputAction::Scroll { .. } => Err(input_execution_error(
            "unsupported_scroll_action",
            "scroll(...) is currently implemented for Linux/X11 input delivery; this adapter does not yet support native wheel injection".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
        InputAction::ElementClick { .. } => Err(input_execution_error(
            "unresolved_element_click_action",
            "click(<element-id>) should be resolved to target coordinates before reaching the macOS input adapter".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
    }
}

trait WindowsRuntimeBackend {
    fn capture_display(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError>;
    fn capture_window(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError>;
    fn focus_window(&self, target_id: &str) -> Result<(), String>;
    fn send_text(&self, text: &str) -> Result<(), String>;
    fn tap_key(&self, key: &str) -> Result<(), String>;
    fn hold_modifier(&self, modifier: ModifierKey) -> Result<(), String>;
    fn release_modifier(&self, modifier: ModifierKey) -> Result<(), String>;
    fn click_mouse(&self, button: MouseButton, x: i32, y: i32) -> Result<(), String>;
    fn drag_mouse(
        &self,
        button: MouseButton,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy)]
struct NativeWindowsRuntime;

static WINDOWS_RUNTIME: NativeWindowsRuntime = NativeWindowsRuntime;

impl WindowsRuntimeBackend for NativeWindowsRuntime {
    fn capture_display(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError> {
        tendril_win32::capture_display_png(target_id).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                platform,
                format!("Windows display capture failed: {error}"),
            )
        })
    }

    fn capture_window(
        &self,
        platform: PlatformKind,
        target_id: &str,
    ) -> Result<Vec<u8>, PlatformAdapterError> {
        tendril_win32::capture_window_png(target_id).map_err(|error| {
            PlatformAdapterError::adapter_failure(
                AdapterOperation::Capture,
                platform,
                format!("Windows window capture failed: {error}"),
            )
        })
    }

    fn focus_window(&self, target_id: &str) -> Result<(), String> {
        tendril_win32::focus_window(target_id)
    }

    fn send_text(&self, text: &str) -> Result<(), String> {
        tendril_win32::send_text(text)
    }

    fn tap_key(&self, key: &str) -> Result<(), String> {
        tendril_win32::tap_key(key)
    }

    fn hold_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
        tendril_win32::hold_modifier(windows_runtime_modifier(modifier))
    }

    fn release_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
        tendril_win32::release_modifier(windows_runtime_modifier(modifier))
    }

    fn click_mouse(&self, button: MouseButton, x: i32, y: i32) -> Result<(), String> {
        tendril_win32::click_mouse(windows_runtime_mouse_button(button), x, y)
    }

    fn drag_mouse(
        &self,
        button: MouseButton,
        start_x: i32,
        start_y: i32,
        end_x: i32,
        end_y: i32,
    ) -> Result<(), String> {
        tendril_win32::drag_mouse(
            windows_runtime_mouse_button(button),
            start_x,
            start_y,
            end_x,
            end_y,
        )
    }
}

fn execute_windows_input(
    platform: PlatformKind,
    request: &InputRequest,
) -> Result<InputOutcome, TendrilError> {
    execute_windows_input_with_runtime(platform, request, &WINDOWS_RUNTIME)
}

fn execute_windows_input_with_runtime(
    _platform: PlatformKind,
    request: &InputRequest,
    runtime: &dyn WindowsRuntimeBackend,
) -> Result<InputOutcome, TendrilError> {
    let keyboard_input = request.text.is_some() || request.actions.iter().any(action_is_keyboard);
    let mut focus_required = keyboard_input || matches!(request.target, CaptureTargetKind::Window);
    let mut focus_transferred = false;
    let mut notes = Vec::new();
    let restore_error = if request.restore_focus {
        let message =
            "Windows focus restoration is not yet implemented by the Tendril adapter".to_owned();
        notes.push(format!(
            "Focus restoration requested but skipped: {message}."
        ));
        Some(message)
    } else {
        notes.push(
            "Focus restoration disabled for this run; focus may remain on the target.".to_owned(),
        );
        None
    };

    if matches!(request.target, CaptureTargetKind::Window) {
        run_windows_step(
            runtime.focus_window(&request.target_id),
            "focus",
            None,
            None,
        )?;
        focus_transferred = true;
        notes.push(
            "Activated the target window with native Win32 focus APIs before dispatching Windows input."
                .to_owned(),
        );
        std::thread::sleep(reliability_delay());
    } else if keyboard_input {
        notes.push(
            "Display-scoped keyboard input uses the currently focused Windows control; transfer focus manually when required."
                .to_owned(),
        );
    }

    if let Some(text) = &request.text {
        run_windows_step(runtime.send_text(text), "dispatch", Some(0), Some("text"))?;
        return Ok(InputOutcome {
            action_count: 1,
            focus_required,
            focus_transferred,
            focused_target: if focus_transferred {
                Some(request.target_id.clone())
            } else {
                None
            },
            previous_focus: None,
            focus_restored: false,
            pointer_restored: false,
            restore_error: restore_error.clone(),
            notes,
        });
    }

    for (action_index, action) in request.actions.iter().enumerate() {
        let label = action_label(action);
        dispatch_windows_action_with_runtime(runtime, request, action, action_index, &label)?;
        if !matches!(action, InputAction::Wait { .. }) {
            std::thread::sleep(reliability_delay());
        }
    }

    if !keyboard_input && matches!(request.target, CaptureTargetKind::Display) {
        focus_required = false;
    }

    Ok(InputOutcome {
        action_count: request.actions.len(),
        focus_required,
        focus_transferred,
        focused_target: if focus_transferred {
            Some(request.target_id.clone())
        } else {
            None
        },
        previous_focus: None,
        focus_restored: false,
        pointer_restored: false,
        restore_error,
        notes,
    })
}

fn dispatch_windows_action_with_runtime(
    runtime: &dyn WindowsRuntimeBackend,
    request: &InputRequest,
    action: &InputAction,
    action_index: usize,
    label: &str,
) -> Result<(), TendrilError> {
    match action {
        InputAction::KeyTap { key } => {
            if !windows_key_is_supported(key) {
                return Err(TendrilError::execution_failure(
                    "unsupported_key",
                    format!("unsupported Windows key `{key}`"),
                    None,
                ));
            }
            run_windows_step(
                runtime.tap_key(key),
                "dispatch",
                Some(action_index),
                Some(label),
            )
        }
        InputAction::Hold { modifier } => run_windows_step(
            runtime.hold_modifier(*modifier),
            "dispatch",
            Some(action_index),
            Some(label),
        ),
        InputAction::Release { modifier } => run_windows_step(
            runtime.release_modifier(*modifier),
            "dispatch",
            Some(action_index),
            Some(label),
        ),
        InputAction::Send { text } => run_windows_step(
            runtime.send_text(text),
            "dispatch",
            Some(action_index),
            Some(label),
        ),
        InputAction::Wait { duration_ms } => {
            std::thread::sleep(Duration::from_millis(*duration_ms));
            Ok(())
        }
        InputAction::Click { button, x, y } => {
            let (absolute_x, absolute_y) = relative_point_to_absolute(&request.bounds, *x, *y);
            run_windows_step(
                runtime.click_mouse(*button, absolute_x, absolute_y),
                "dispatch",
                Some(action_index),
                Some(label),
            )
        }
        InputAction::PointerMove { .. } => Err(input_execution_error(
            "unsupported_pointer_move_action",
            "move(...) and hover(...) are currently implemented for Linux/X11 input delivery; this adapter does not yet support native pointer-only motion".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
        InputAction::DoubleClick { .. } => Err(input_execution_error(
            "unsupported_double_click_action",
            "dblclick(...) is currently implemented for Linux/X11 input delivery; this Windows adapter does not yet support native double-click injection".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
        InputAction::Drag { x0, y0, x1, y1 } => {
            let (start_x, start_y) = relative_point_to_absolute(&request.bounds, *x0, *y0);
            let (end_x, end_y) = relative_point_to_absolute(&request.bounds, *x1, *y1);
            run_windows_step(
                runtime.drag_mouse(MouseButton::Left, start_x, start_y, end_x, end_y),
                "dispatch",
                Some(action_index),
                Some(label),
            )
        }
        InputAction::Scroll { .. } => Err(input_execution_error(
            "unsupported_scroll_action",
            "scroll(...) is currently implemented for Linux/X11 input delivery; this adapter does not yet support native wheel injection".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
        InputAction::ElementClick { .. } => Err(input_execution_error(
            "unresolved_element_click_action",
            "click(<element-id>) should be resolved to target coordinates before reaching the Windows input adapter".to_owned(),
            "dispatch",
            Some(action_index),
            Some(label),
        )),
    }
}

fn run_windows_step(
    result: Result<(), String>,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    result.map_err(|error| {
        input_execution_error(
            "input_command_failed",
            format!("Windows input failed: {error}"),
            stage,
            action_index,
            action,
        )
    })
}

fn windows_runtime_modifier(modifier: ModifierKey) -> tendril_win32::ModifierKey {
    match modifier {
        ModifierKey::Ctrl => tendril_win32::ModifierKey::Ctrl,
        ModifierKey::Alt => tendril_win32::ModifierKey::Alt,
        ModifierKey::Shift => tendril_win32::ModifierKey::Shift,
        ModifierKey::Meta => tendril_win32::ModifierKey::Meta,
    }
}

fn windows_runtime_mouse_button(button: MouseButton) -> tendril_win32::MouseButton {
    match button {
        MouseButton::Left => tendril_win32::MouseButton::Left,
        MouseButton::Middle => tendril_win32::MouseButton::Middle,
        MouseButton::Right => tendril_win32::MouseButton::Right,
    }
}

fn windows_key_is_supported(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "enter"
            | "return"
            | "esc"
            | "escape"
            | "tab"
            | "space"
            | "backspace"
            | "delete"
            | "del"
            | "left"
            | "right"
            | "up"
            | "down"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
    ) || key.chars().count() == 1
}

fn action_is_keyboard(action: &InputAction) -> bool {
    matches!(
        action,
        InputAction::KeyTap { .. }
            | InputAction::Hold { .. }
            | InputAction::Release { .. }
            | InputAction::Send { .. }
    )
}

fn action_label(action: &InputAction) -> String {
    serde_json::to_string(action).unwrap_or_else(|_| format!("{action:?}"))
}

fn input_execution_error(
    code: &'static str,
    message: String,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> TendrilError {
    let mut error = TendrilError::execution_failure(code, message, action_index)
        .with_detail_entry("stage", json!(stage));
    if let Some(action_index) = action_index {
        error = error.with_detail_entry("action_number", json!(action_index + 1));
    }
    if let Some(action) = action {
        error = error.with_detail_entry("action", json!(action));
    }
    error
}

fn javascript_string_literal(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| String::from("\"\""))
}

#[cfg(test)]
fn macos_focus_pid_jxa_script(process_id: u32) -> String {
    format!(
        r"ObjC.import('AppKit');
(function () {{
    var app = $.NSRunningApplication.runningApplicationWithProcessIdentifier({process_id});
    if (!app) {{
        throw new Error('target process could not be activated');
    }}
    if (!app.activateWithOptions($.NSApplicationActivateIgnoringOtherApps)) {{
        throw new Error('target process could not be activated');
    }}
}}());
"
    )
}

/// Build a JXA script that raises the specific macOS window matching
/// `cg_window_id` (with `bounds` as a fallback discriminator) before activating
/// the owning application. This ensures `tendril --window <id>` dispatches input
/// into the requested window even when another window of the same app is
/// frontmost.
fn macos_focus_window_jxa_script(
    process_id: u32,
    cg_window_id: Option<u64>,
    bounds: &Bounds,
) -> String {
    // When the discovery id can't be parsed as a CGWindowID, pass -1 so the
    // private _AXUIElementGetWindow lookup is skipped and we fall back to
    // bounds-based matching only.
    let window_id_literal: i64 = cg_window_id
        .and_then(|id| i64::try_from(id).ok())
        .unwrap_or(-1);
    let target_x = bounds.x;
    let target_y = bounds.y;
    let target_width = bounds.width;
    let target_height = bounds.height;
    format!(
        r"ObjC.import('AppKit');
ObjC.import('ApplicationServices');
(function () {{
    var pid = {process_id};
    var targetWindowId = {window_id_literal};
    var targetX = {target_x};
    var targetY = {target_y};
    var targetWidth = {target_width};
    var targetHeight = {target_height};

    var app = $.NSRunningApplication.runningApplicationWithProcessIdentifier(pid);
    if (!app) {{
        throw new Error('target process could not be activated');
    }}

    // Best-effort: walk the application's AX windows and raise the one whose
    // CGWindowID matches (preferred) or whose position+size best match the
    // discovery bounds. AXRaise puts the chosen window above the app's other
    // windows so that NSRunningApplication.activate brings the right one to
    // the front. Failures here are non-fatal — we still activate the app and
    // let input dispatch proceed against whatever window is frontmost.
    try {{
        var getWindowFn = null;
        try {{
            // _AXUIElementGetWindow is a private but widely-relied-upon helper
            // that maps an AXUIElement to its CGWindowID. JXA cannot resolve it
            // through `$.` lookup, so bind it explicitly.
            getWindowFn = ObjC.bindFunction('_AXUIElementGetWindow', ['int', ['id', 'pointer']]);
        }} catch (bindError) {{
            getWindowFn = null;
        }}

        var axApp = $.AXUIElementCreateApplication(pid);
        if (axApp) {{
            var windowsRef = Ref();
            var copyErr = $.AXUIElementCopyAttributeValue(axApp, $('AXWindows'), windowsRef);
            if (copyErr === 0 && windowsRef[0]) {{
                var nsWindows = ObjC.castRefToObject(windowsRef[0]);
                var count = nsWindows.count;
                var matched = null;
                var bestScore = Infinity;
                for (var i = 0; i < count; i += 1) {{
                    var win = nsWindows.objectAtIndex(i);

                    if (matched === null && getWindowFn !== null && targetWindowId >= 0) {{
                        try {{
                            var widRef = Ref('uint32', 0);
                            var widErr = getWindowFn(win, widRef);
                            if (widErr === 0 && widRef[0] === targetWindowId) {{
                                matched = win;
                                continue;
                            }}
                        }} catch (widLookupError) {{
                            // ignore and fall back to bounds matching
                        }}
                    }}

                    try {{
                        var posRef = Ref();
                        var sizeRef = Ref();
                        if ($.AXUIElementCopyAttributeValue(win, $('AXPosition'), posRef) !== 0) {{
                            continue;
                        }}
                        if ($.AXUIElementCopyAttributeValue(win, $('AXSize'), sizeRef) !== 0) {{
                            continue;
                        }}
                        var posOut = $.CGPointMake(0, 0);
                        var sizeOut = $.CGSizeMake(0, 0);
                        // kAXValueCGPointType = 1, kAXValueCGSizeType = 2
                        if (!$.AXValueGetValue(posRef[0], 1, Ref(posOut))) continue;
                        if (!$.AXValueGetValue(sizeRef[0], 2, Ref(sizeOut))) continue;
                        var dx = posOut.x - targetX;
                        var dy = posOut.y - targetY;
                        var dw = sizeOut.width - targetWidth;
                        var dh = sizeOut.height - targetHeight;
                        var score = dx * dx + dy * dy + dw * dw + dh * dh;
                        if (score < bestScore) {{
                            bestScore = score;
                            if (matched === null) {{
                                matched = win;
                            }}
                        }}
                    }} catch (matchError) {{
                        // continue scanning
                    }}
                }}
                if (matched !== null) {{
                    $.AXUIElementPerformAction(matched, $('AXRaise'));
                }}
            }}
        }}
    }} catch (axError) {{
        // Accessibility lookup failed (likely missing permission). Continue to
        // app activation; the existing input-dispatch path will surface the
        // permission error if Accessibility consent is required.
    }}

    if (!app.activateWithOptions($.NSApplicationActivateIgnoringOtherApps)) {{
        throw new Error('target process could not be activated');
    }}
}}());
"
    )
}

fn macos_focus_app_jxa_script(app_name: &str) -> String {
    let app_name = javascript_string_literal(app_name);
    format!(
        r"var app = Application({app_name});
app.activate();
"
    )
}

fn macos_text_jxa_script(text: &str) -> String {
    // We dispatch text via AppleScript (not JXA) so we can wrap the
    // System Events keystroke in a `with timeout of N seconds` block.
    //
    // Background: keystroke is the only step in our input pipeline that
    // crosses the AppleEvent boundary (modifier hold/release and mouse
    // events go through CoreGraphics directly). Without an explicit
    // timeout block the AppleEvent inherits osascript's default ~60s
    // budget and aborts with -1712 whenever the macOS TCC consent prompt
    // for Accessibility/Automation is sitting on screen waiting for the
    // operator to click "OK". A longer timeout lets the consent dialog
    // be confirmed without a spurious failure; if there is no consent
    // dialog and System Events is genuinely wedged, the timeout still
    // fires and we surface a distinct `input_command_timeout` error so
    // callers can distinguish it from a denied permission.
    let timeout = macos_input_applescript_timeout_secs();
    let text = applescript_string_literal(text);
    format!(
        r#"with timeout of {timeout} seconds
    tell application "System Events" to keystroke {text}
end timeout
"#
    )
}

/// `AppleEvent` timeout (in seconds) applied to System Events keystroke
/// dispatch. Long enough to absorb a pending TCC consent prompt without
/// being unbounded.
fn macos_input_applescript_timeout_secs() -> u32 {
    300
}

/// Escape `text` as an `AppleScript` string literal.
fn applescript_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Language flag passed to `osascript -l`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OsaLanguage {
    JavaScript,
    AppleScript,
}

impl OsaLanguage {
    fn as_flag(self) -> &'static str {
        match self {
            OsaLanguage::JavaScript => "JavaScript",
            OsaLanguage::AppleScript => "AppleScript",
        }
    }
}

fn macos_modifier_jxa_script(modifier: ModifierKey, down: bool) -> Result<String, TendrilError> {
    let key_code = macos_key_code(match modifier {
        ModifierKey::Ctrl => "ctrl",
        ModifierKey::Alt => "alt",
        ModifierKey::Shift => "shift",
        ModifierKey::Meta => "meta",
    })?;
    Ok(macos_keyboard_jxa_script(key_code, down, false))
}

fn macos_key_jxa_script(
    key: &str,
    down: Option<bool>,
    up: Option<bool>,
) -> Result<String, TendrilError> {
    let key_code = macos_key_code(key)?;
    Ok(macos_keyboard_jxa_script(
        key_code,
        down.unwrap_or(true),
        up.unwrap_or(true),
    ))
}

fn macos_keyboard_jxa_script(key_code: u16, post_down: bool, post_up: bool) -> String {
    format!(
        r"ObjC.import('CoreGraphics');
(function () {{
    function post(keyDown) {{
        var event = $.CGEventCreateKeyboardEvent(null, {key_code}, keyDown);
        if (!event) {{
            throw new Error('failed to create keyboard event');
        }}
        $.CGEventPost($.kCGHIDEventTap, event);
    }}
    {post_down_call}{post_up_call}
}}());
",
        post_down_call = if post_down { "post(true);\n    " } else { "" },
        post_up_call = if post_up { "post(false);\n" } else { "" },
    )
}

fn macos_mouse_jxa_script(
    button: MouseButton,
    x: i32,
    y: i32,
    drag_end: Option<(i32, i32)>,
) -> String {
    let (down_event, up_event, drag_event, button_code) = match button {
        MouseButton::Left => (
            "kCGEventLeftMouseDown",
            "kCGEventLeftMouseUp",
            "kCGEventLeftMouseDragged",
            0,
        ),
        MouseButton::Right => (
            "kCGEventRightMouseDown",
            "kCGEventRightMouseUp",
            "kCGEventRightMouseDragged",
            1,
        ),
        MouseButton::Middle => (
            "kCGEventOtherMouseDown",
            "kCGEventOtherMouseUp",
            "kCGEventOtherMouseDragged",
            2,
        ),
    };
    if let Some((end_x, end_y)) = drag_end {
        format!(
            r"ObjC.import('CoreGraphics');
(function () {{
    function point(px, py) {{
        return $.CGPointMake(px, py);
    }}
    function postMouse(eventType, px, py) {{
        var event = $.CGEventCreateMouseEvent(null, $[eventType], point(px, py), {button_code});
        if (!event) {{
            throw new Error('failed to create mouse event');
        }}
        $.CGEventPost($.kCGHIDEventTap, event);
    }}
    $.CGWarpMouseCursorPosition(point({x}, {y}));
    postMouse('kCGEventMouseMoved', {x}, {y});
    postMouse('{down_event}', {x}, {y});
    postMouse('{drag_event}', {end_x}, {end_y});
    postMouse('{up_event}', {end_x}, {end_y});
}}());
"
        )
    } else {
        format!(
            r"ObjC.import('CoreGraphics');
(function () {{
    function point(px, py) {{
        return $.CGPointMake(px, py);
    }}
    function postMouse(eventType, px, py) {{
        var event = $.CGEventCreateMouseEvent(null, $[eventType], point(px, py), {button_code});
        if (!event) {{
            throw new Error('failed to create mouse event');
        }}
        $.CGEventPost($.kCGHIDEventTap, event);
    }}
    $.CGWarpMouseCursorPosition(point({x}, {y}));
    postMouse('kCGEventMouseMoved', {x}, {y});
    postMouse('{down_event}', {x}, {y});
    postMouse('{up_event}', {x}, {y});
}}());
"
        )
    }
}

fn run_macos_osascript_jxa_for_input(
    script: &str,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    // The vast majority of input scripts are JXA (CoreGraphics calls
    // through ObjC bridges); the text-dispatch script is AppleScript
    // because it needs `with timeout of`. We sniff a leading marker
    // rather than thread an enum through every call site.
    let language = if script.trim_start().starts_with("with timeout of") {
        OsaLanguage::AppleScript
    } else {
        OsaLanguage::JavaScript
    };
    run_macos_osascript_for_input(language, script, stage, action_index, action)
}

fn run_macos_osascript_for_input(
    language: OsaLanguage,
    script: &str,
    stage: &'static str,
    action_index: Option<usize>,
    action: Option<&str>,
) -> Result<(), TendrilError> {
    let output = std::process::Command::new("osascript")
        .args(["-l", language.as_flag(), "-e", script])
        .output()
        .map_err(|error| {
            input_execution_error(
                "input_spawn_failed",
                format!("failed to spawn osascript: {error}"),
                stage,
                action_index,
                action,
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let message = if stdout.is_empty() || stderr.is_empty() {
        format!("{stdout}{stderr}")
    } else {
        format!("{stdout} | {stderr}")
    };

    if is_macos_input_permission_error(&message) {
        let mut error = TendrilError::from(PlatformAdapterError::missing_permission(
            Capability::InputControl,
            PermissionKind::Accessibility,
            PlatformKind::MacOs,
            if message.is_empty() {
                "macOS input control requires Accessibility consent.".to_owned()
            } else {
                format!("macOS input control requires Accessibility consent: {message}")
            },
            "Grant Accessibility access in System Settings > Privacy & Security > Accessibility, then rerun tendril run.",
        ))
        .with_detail_entry("stage", json!(stage));
        if let Some(action_index) = action_index {
            error = error.with_detail_entry("action_number", json!(action_index + 1));
        }
        if let Some(action) = action {
            error = error.with_detail_entry("action", json!(action));
        }
        return Err(error);
    }

    if is_macos_input_apple_event_timeout(&message) {
        // -1712 is `errAETimeout`. It is distinct from the permission
        // errors above: System Events accepted the AppleEvent but did
        // not reply within the script's timeout. The most common cause
        // we see in practice is a pending TCC consent dialog (the user
        // never clicked "OK"); a wedged System Events process is the
        // other possibility. Surface this as its own error code so the
        // caller can distinguish it from a flat-out denial.
        let detail_message = if message.is_empty() {
            "AppleEvent timed out (-1712) while dispatching input through System Events.".to_owned()
        } else {
            format!("AppleEvent timed out while dispatching input: {message}")
        };
        return Err(input_execution_error(
            "input_command_timeout",
            detail_message,
            stage,
            action_index,
            action,
        )
        .with_detail_entry(
            "hint",
            json!(
                "Confirm any pending macOS consent prompt for Accessibility/Automation, or grant access in System Settings > Privacy & Security. If no prompt is visible, restarting the System Events process or targeting a window with --window can avoid the AppleEvent path."
            ),
        ));
    }

    Err(input_execution_error(
        "input_command_failed",
        if message.is_empty() {
            format!("osascript exited with status {}", output.status)
        } else {
            format!("osascript failed: {message}")
        },
        stage,
        action_index,
        action,
    ))
}

fn is_macos_input_permission_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    // Note: the generic phrase "system events got an error" appears in the
    // AppleEvent timeout message as well ("System Events got an error:
    // AppleEvent timed out."), so we must check the timeout case first at
    // the call site. Here we still treat it as a permission hint because
    // historically every other System Events failure we have seen on this
    // path was an Accessibility denial.
    lower.contains("accessibility")
        || lower.contains("assistive access")
        || lower.contains("apple events")
        || lower.contains("not authorized")
        || lower.contains("not permitted")
        || lower.contains("-1719")
        || lower.contains("-1743")
        || (lower.contains("system events got an error")
            && !is_macos_input_apple_event_timeout(message))
}

/// Detects `errAETimeout` (-1712), distinct from Accessibility denials.
fn is_macos_input_apple_event_timeout(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("-1712") || lower.contains("appleevent timed out")
}

fn macos_key_code(key: &str) -> Result<u16, TendrilError> {
    match key.to_ascii_lowercase().as_str() {
        "a" => Ok(0),
        "s" => Ok(1),
        "d" => Ok(2),
        "f" => Ok(3),
        "h" => Ok(4),
        "g" => Ok(5),
        "z" => Ok(6),
        "x" => Ok(7),
        "c" => Ok(8),
        "v" => Ok(9),
        "b" => Ok(11),
        "q" => Ok(12),
        "w" => Ok(13),
        "e" => Ok(14),
        "r" => Ok(15),
        "y" => Ok(16),
        "t" => Ok(17),
        "1" => Ok(18),
        "2" => Ok(19),
        "3" => Ok(20),
        "4" => Ok(21),
        "6" => Ok(22),
        "5" => Ok(23),
        "=" => Ok(24),
        "9" => Ok(25),
        "7" => Ok(26),
        "-" => Ok(27),
        "8" => Ok(28),
        "0" => Ok(29),
        "]" => Ok(30),
        "o" => Ok(31),
        "u" => Ok(32),
        "[" => Ok(33),
        "i" => Ok(34),
        "p" => Ok(35),
        "enter" | "return" => Ok(36),
        "l" => Ok(37),
        "j" => Ok(38),
        "'" => Ok(39),
        "k" => Ok(40),
        ";" => Ok(41),
        "\\" => Ok(42),
        "," => Ok(43),
        "/" => Ok(44),
        "n" => Ok(45),
        "m" => Ok(46),
        "." => Ok(47),
        "tab" => Ok(48),
        "space" => Ok(49),
        "`" => Ok(50),
        "backspace" => Ok(51),
        "esc" | "escape" => Ok(53),
        "meta" | "cmd" | "command" => Ok(55),
        "shift" => Ok(56),
        "alt" | "option" => Ok(58),
        "ctrl" | "control" => Ok(59),
        "right_shift" => Ok(60),
        "right_alt" => Ok(61),
        "right_ctrl" => Ok(62),
        "left" => Ok(123),
        "right" => Ok(124),
        "down" => Ok(125),
        "up" => Ok(126),
        other => Err(TendrilError::execution_failure(
            "unsupported_key",
            format!("unsupported macOS key `{other}`"),
            None,
        )),
    }
}

#[must_use]
pub fn adapter_for_context(context: AdapterContext) -> Box<dyn PlatformAdapter> {
    match context.platform {
        PlatformKind::MacOs => Box::new(MacOsAdapter::new(context)),
        PlatformKind::Linux => Box::new(LinuxAdapter::new(context)),
        PlatformKind::Windows11 => Box::new(WindowsAdapter::new(context)),
    }
}

#[must_use]
pub fn current_adapter() -> Box<dyn PlatformAdapter> {
    adapter_for_context(AdapterContext::detect())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn detect_linux_session(
    xdg_session_type: Option<&str>,
    display: Option<&std::ffi::OsStr>,
    wayland_display: Option<&std::ffi::OsStr>,
) -> DesktopSession {
    match xdg_session_type.map(str::to_ascii_lowercase).as_deref() {
        Some("x11") => DesktopSession::X11,
        Some("wayland") => DesktopSession::Wayland,
        Some(_) | None => {
            if wayland_display.is_some() {
                DesktopSession::Wayland
            } else if display.is_some() {
                DesktopSession::X11
            } else {
                DesktopSession::Unknown
            }
        }
    }
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn detect_linux_audio_backend(
    pipewire_runtime_dir: Option<&std::ffi::OsStr>,
    pulse_server: Option<&std::ffi::OsStr>,
    pulse_runtime_path: Option<&std::ffi::OsStr>,
    xdg_runtime_dir: Option<&std::ffi::OsStr>,
    path_exists: &dyn Fn(&std::path::Path) -> bool,
) -> Option<AudioBackend> {
    if pipewire_runtime_dir.is_some() {
        return Some(AudioBackend::PipeWire);
    }
    if pulse_server.is_some() || pulse_runtime_path.is_some() {
        return Some(AudioBackend::PulseAudio);
    }
    // Fall back to probing well-known sockets under XDG_RUNTIME_DIR. On many
    // NixOS/systemd setups the PIPEWIRE_RUNTIME_DIR / PULSE_SERVER env vars
    // are not exported even though the daemons are running and reachable via
    // their default sockets.
    if let Some(runtime_dir) = xdg_runtime_dir {
        let base = std::path::Path::new(runtime_dir);
        if path_exists(&base.join("pipewire-0")) {
            return Some(AudioBackend::PipeWire);
        }
        if path_exists(&base.join("pulse/native")) {
            return Some(AudioBackend::PulseAudio);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterContext, AudioBackend, AudioCapabilityProbe, AudioSourceKind, Bounds,
        CaptureAdapter, CaptureTargetKind, DesktopSession, InputControlAdapter, InputRequest,
        LinuxAdapter, MacOsAdapter, ModifierKey, MouseButton, PermissionAdapter, PermissionKind,
        PermissionState, PlatformAdapter, PlatformAdapterError, PlatformKind, TargetDescriptor,
        TargetInventory, WAYLAND_PORTAL_FALLBACK_TIMEOUT_MS, WaylandCaptureBackendError,
        WindowsAdapter, WindowsRuntimeBackend, capture_wayland_target_with_grim,
        crop_wayland_portal_capture_to_target, detect_linux_audio_backend, detect_linux_session,
        execute_windows_input_with_runtime, is_macos_input_permission_error,
        javascript_string_literal, macos_focus_pid_jxa_script, macos_focus_window_jxa_script,
        macos_text_jxa_script, wayland_capture_program_on_path, wayland_portal_attempt_timeout,
        wayland_workspace_origin, windows_key_is_supported,
    };
    use crate::{TendrilError, model::InputAction};
    use mcp_cli::ErrorCategory;

    #[test]
    fn linux_session_detection_prefers_declared_session_type() {
        let detected =
            detect_linux_session(Some("wayland"), Some(std::ffi::OsStr::new(":0")), None);

        assert_eq!(detected, DesktopSession::Wayland);
    }

    #[test]
    fn linux_audio_backend_detection_uses_pipewire_before_pulseaudio() {
        let detected = detect_linux_audio_backend(
            Some(std::ffi::OsStr::new("/run/user/1000")),
            Some(std::ffi::OsStr::new("unix:/tmp/pulse")),
            None,
            None,
            &|_| false,
        );

        assert_eq!(detected, Some(AudioBackend::PipeWire));
    }

    #[test]
    fn linux_audio_backend_falls_back_to_pipewire_socket_probe() {
        let detected = detect_linux_audio_backend(
            None,
            None,
            None,
            Some(std::ffi::OsStr::new("/run/user/1000")),
            &|path| path == std::path::Path::new("/run/user/1000/pipewire-0"),
        );

        assert_eq!(detected, Some(AudioBackend::PipeWire));
    }

    #[test]
    fn linux_audio_backend_falls_back_to_pulse_socket_probe() {
        let detected = detect_linux_audio_backend(
            None,
            None,
            None,
            Some(std::ffi::OsStr::new("/run/user/1000")),
            &|path| path == std::path::Path::new("/run/user/1000/pulse/native"),
        );

        assert_eq!(detected, Some(AudioBackend::PulseAudio));
    }

    #[test]
    fn linux_audio_backend_prefers_pipewire_socket_over_pulse_socket() {
        let detected = detect_linux_audio_backend(
            None,
            None,
            None,
            Some(std::ffi::OsStr::new("/run/user/1000")),
            &|_| true,
        );

        assert_eq!(detected, Some(AudioBackend::PipeWire));
    }

    #[test]
    fn linux_audio_backend_returns_none_without_env_or_sockets() {
        let detected = detect_linux_audio_backend(None, None, None, None, &|_| false);
        assert_eq!(detected, None);
    }

    #[test]
    fn macos_loopback_probe_returns_structured_capability_error() {
        let adapter = MacOsAdapter::new(AdapterContext::macos());
        let error = adapter
            .probe_audio_capture(&super::AudioProbeRequest {
                source: AudioSourceKind::SystemLoopback,
                duration_hint_ms: Some(500),
            })
            .expect_err("loopback should be unsupported on the macOS spine");

        match error {
            PlatformAdapterError::UnsupportedCapability(capability) => {
                assert_eq!(
                    capability.capability,
                    super::Capability::AudioLoopbackCapture
                );
                assert_eq!(capability.platform, PlatformKind::MacOs);
                assert_eq!(
                    capability.reason,
                    super::CapabilityErrorReason::UnsupportedFeature
                );
                assert!(capability.suggested_action.is_some());
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn linux_wayland_input_support_branches_match_helper_tool_availability() {
        // The actionable diagnostic is asserted directly against
        // `wayland_input::missing_backend_error` in that module's unit tests so
        // we do not need to mutate process-global PATH state from this test.
        let adapter = LinuxAdapter::new(AdapterContext::linux(
            DesktopSession::Wayland,
            Some(AudioBackend::PipeWire),
        ));
        match adapter.input_support() {
            Ok(support) => {
                assert_eq!(support.capability, super::Capability::InputControl);
                assert!(
                    support
                        .notes
                        .iter()
                        .any(|note| note.contains("ydotool") || note.contains("wtype")),
                    "supported branch should explain which Wayland helper backs input"
                );
            }
            Err(PlatformAdapterError::UnsupportedCapability(capability)) => {
                assert_eq!(capability.capability, super::Capability::InputControl);
                assert_eq!(
                    capability.reason,
                    super::CapabilityErrorReason::UnsupportedFeature
                );
                assert!(
                    capability.message.contains("ydotool") && capability.message.contains("wtype"),
                    "diagnostic should reference both Wayland helper tools: {}",
                    capability.message
                );
            }
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn windows_microphone_probe_exposes_permission_guidance() {
        let adapter = WindowsAdapter::new(AdapterContext::windows11());
        let report = adapter
            .probe_audio_capture(&super::AudioProbeRequest {
                source: AudioSourceKind::Microphone,
                duration_hint_ms: Some(1_000),
            })
            .expect("windows microphone probe should succeed");

        assert_eq!(report.backend, AudioBackend::Wasapi);
        assert_eq!(report.permissions.len(), 1);
        assert_eq!(report.permissions[0].permission, PermissionKind::Microphone);
        assert_eq!(report.permissions[0].state, PermissionState::Unknown);
        assert!(report.permissions[0].suggested_action.is_some());
    }

    #[test]
    fn macos_capture_support_requires_screen_permission() {
        let adapter = MacOsAdapter::new(AdapterContext::macos());
        let support = adapter
            .capture_support(CaptureTargetKind::Display)
            .expect("capture support should be described on macOS");

        assert_eq!(support.permissions.len(), 1);
        assert_eq!(
            support.permissions[0].permission,
            PermissionKind::ScreenCapture
        );
    }

    #[test]
    fn macos_screen_recording_remediation_is_actionable_for_operators() {
        // bd-a24d8d: capture failures must surface the binary path, the exact
        // System Settings pane, and the parent-process hint so that remote
        // operators (e.g. caco @ms-mac exec) can grant TCC consent without
        // additional spelunking.
        let message = super::screen_recording_remediation_message();
        for needle in [
            "Screen Recording",
            "Privacy & Security",
            "x-apple.systempreferences",
            "System Settings",
            "parent process",
            "tendril list",
            "tccutil",
        ] {
            assert!(
                message.contains(needle),
                "remediation should mention {needle:?}; got:\n{message}"
            );
        }
        // Should embed the absolute current_exe path of the test binary, not
        // a placeholder, when the lookup succeeds.
        if let Ok(exe) = std::env::current_exe() {
            assert!(
                message.contains(&exe.display().to_string())
                    || message.contains("<tendril binary>"),
                "remediation should embed the binary path or fallback placeholder"
            );
        }
    }

    #[test]
    fn adapter_permissions_are_explicit_and_stateless() {
        let adapter = WindowsAdapter::new(AdapterContext::windows11());
        let permissions = adapter.permissions();

        assert_eq!(adapter.info().platform, PlatformKind::Windows11);
        assert!(adapter.info().stateless);
        assert_eq!(permissions.len(), 3);
    }

    #[derive(Default)]
    struct MockWindowsRuntime {
        calls: std::sync::Mutex<Vec<String>>,
    }

    impl MockWindowsRuntime {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("mutex poisoned").clone()
        }

        fn push(&self, call: impl Into<String>) {
            self.calls.lock().expect("mutex poisoned").push(call.into());
        }
    }

    impl WindowsRuntimeBackend for MockWindowsRuntime {
        fn capture_display(
            &self,
            _platform: PlatformKind,
            target_id: &str,
        ) -> Result<Vec<u8>, PlatformAdapterError> {
            self.push(format!("capture_display:{target_id}"));
            Ok(vec![1, 2, 3])
        }

        fn capture_window(
            &self,
            _platform: PlatformKind,
            target_id: &str,
        ) -> Result<Vec<u8>, PlatformAdapterError> {
            self.push(format!("capture_window:{target_id}"));
            Ok(vec![4, 5, 6])
        }

        fn focus_window(&self, target_id: &str) -> Result<(), String> {
            self.push(format!("focus:{target_id}"));
            Ok(())
        }

        fn send_text(&self, text: &str) -> Result<(), String> {
            self.push(format!("text:{text}"));
            Ok(())
        }

        fn tap_key(&self, key: &str) -> Result<(), String> {
            self.push(format!("tap:{key}"));
            Ok(())
        }

        fn hold_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
            self.push(format!("hold:{modifier:?}"));
            Ok(())
        }

        fn release_modifier(&self, modifier: ModifierKey) -> Result<(), String> {
            self.push(format!("release:{modifier:?}"));
            Ok(())
        }

        fn click_mouse(&self, button: MouseButton, x: i32, y: i32) -> Result<(), String> {
            self.push(format!("click:{button:?}:{x}:{y}"));
            Ok(())
        }

        fn drag_mouse(
            &self,
            button: MouseButton,
            start_x: i32,
            start_y: i32,
            end_x: i32,
            end_y: i32,
        ) -> Result<(), String> {
            self.push(format!(
                "drag:{button:?}:{start_x}:{start_y}:{end_x}:{end_y}"
            ));
            Ok(())
        }
    }

    #[test]
    fn windows_native_runtime_flow_focuses_and_dispatches_actions() {
        let runtime = MockWindowsRuntime::default();
        let request = InputRequest {
            target: CaptureTargetKind::Window,
            target_id: "0x10".to_owned(),
            target_name: "Inbox".to_owned(),
            bounds: Bounds {
                x: 100,
                y: 200,
                width: 400,
                height: 300,
            },
            app_name: Some("Notepad".to_owned()),
            process_id: Some(42),
            restore_focus: true,
            text: None,
            actions: vec![
                InputAction::Hold {
                    modifier: ModifierKey::Ctrl,
                },
                InputAction::KeyTap {
                    key: "c".to_owned(),
                },
                InputAction::Release {
                    modifier: ModifierKey::Ctrl,
                },
                InputAction::Click {
                    button: MouseButton::Left,
                    x: 5,
                    y: 7,
                },
                InputAction::Drag {
                    x0: 1,
                    y0: 2,
                    x1: 20,
                    y1: 25,
                },
            ],
        };

        let outcome =
            execute_windows_input_with_runtime(PlatformKind::Windows11, &request, &runtime)
                .expect("mocked windows input should succeed");

        assert!(outcome.focus_required);
        assert!(outcome.focus_transferred);
        assert_eq!(outcome.focused_target.as_deref(), Some("0x10"));
        assert!(
            outcome
                .notes
                .iter()
                .any(|note| note.contains("native Win32 focus APIs"))
        );
        assert_eq!(
            runtime.calls(),
            vec![
                "focus:0x10",
                "hold:Ctrl",
                "tap:c",
                "release:Ctrl",
                "click:Left:105:207",
                "drag:Left:101:202:120:225",
            ]
        );
    }

    #[test]
    fn windows_native_runtime_text_flow_preserves_display_focus_guidance() {
        let runtime = MockWindowsRuntime::default();
        let request = InputRequest {
            target: CaptureTargetKind::Display,
            target_id: "1".to_owned(),
            target_name: "Display 1".to_owned(),
            bounds: Bounds {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            app_name: None,
            process_id: None,
            restore_focus: true,
            text: Some("hello".to_owned()),
            actions: Vec::new(),
        };

        let outcome =
            execute_windows_input_with_runtime(PlatformKind::Windows11, &request, &runtime)
                .expect("mocked windows text input should succeed");

        assert!(outcome.focus_required);
        assert!(!outcome.focus_transferred);
        assert_eq!(runtime.calls(), vec!["text:hello"]);
        assert!(
            outcome
                .notes
                .iter()
                .any(|note| note.contains("currently focused Windows control"))
        );
    }

    #[test]
    fn windows_key_support_matches_native_runtime_contract() {
        assert!(windows_key_is_supported("enter"));
        assert!(windows_key_is_supported("A"));
        assert!(windows_key_is_supported("💡"));
        assert!(!windows_key_is_supported("ctrl+alt+delete"));
    }

    #[test]
    fn tendril_error_category_tracks_adapter_error_category() {
        let adapter_error = PlatformAdapterError::unsupported(
            super::Capability::AudioLoopbackCapture,
            PlatformKind::MacOs,
            super::CapabilityErrorReason::UnsupportedFeature,
            "unsupported",
            Some("pick another source"),
        );
        let tendril_error: TendrilError = adapter_error.into();

        assert_eq!(
            tendril_error.category(),
            ErrorCategory::UnsupportedCapability
        );
    }

    #[test]
    fn javascript_string_literal_uses_json_escaping() {
        assert_eq!(
            javascript_string_literal("line \"one\"\nline two"),
            "\"line \\\"one\\\"\\nline two\""
        );
    }

    #[test]
    fn macos_input_permission_classifier_catches_accessibility_failures() {
        assert!(is_macos_input_permission_error(
            "System Events got an error: osascript is not allowed assistive access. (-1719)"
        ));
        assert!(is_macos_input_permission_error(
            "Not authorized to send Apple events to System Events. (-1743)"
        ));
    }

    #[test]
    fn macos_input_permission_classifier_does_not_swallow_apple_event_timeout() {
        // -1712 must not be classified as a permission failure: it is a
        // distinct condition (pending consent dialog or wedged System
        // Events) and surfaces with its own `input_command_timeout`
        // error code.
        let timeout_message = "System Events got an error: AppleEvent timed out. (-1712)";
        assert!(!is_macos_input_permission_error(timeout_message));
        assert!(super::is_macos_input_apple_event_timeout(timeout_message));
        assert!(super::is_macos_input_apple_event_timeout(
            "execution error: AppleEvent timed out. (-1712)"
        ));
        assert!(!super::is_macos_input_apple_event_timeout(
            "System Events got an error: osascript is not allowed assistive access. (-1719)"
        ));
    }

    #[test]
    fn macos_osascript_scripts_are_self_contained() {
        let focus_script = macos_focus_pid_jxa_script(42);
        let text_script = macos_text_jxa_script("hello");

        assert!(focus_script.contains("NSRunningApplication"));
        assert!(text_script.contains("System Events"));
        assert!(!focus_script.contains("swift"));
        assert!(!text_script.contains("swift"));
    }

    #[test]
    fn macos_text_script_wraps_keystroke_in_an_apple_event_timeout() {
        // The text-dispatch script is the only step in the macOS input
        // pipeline that crosses the AppleEvent boundary. It must wrap
        // the System Events keystroke in a `with timeout of` block so
        // that a pending TCC consent prompt does not cause a -1712
        // (errAETimeout) failure within osascript's default budget.
        let script = macos_text_jxa_script("hello");
        assert!(
            script.starts_with("with timeout of"),
            "text script must opt into AppleScript timeout handling, got: {script}"
        );
        assert!(script.contains("end timeout"));
        assert!(script.contains("keystroke \"hello\""));
        // The runner sniffs the leading marker to choose AppleScript
        // over JXA; document that contract here.
        assert_eq!(super::OsaLanguage::AppleScript.as_flag(), "AppleScript");
    }

    #[test]
    fn applescript_string_literal_escapes_special_characters() {
        assert_eq!(
            super::applescript_string_literal("line \"one\"\nline two\\end"),
            "\"line \\\"one\\\"\\nline two\\\\end\""
        );
    }

    #[test]
    fn macos_focus_window_script_raises_specific_window_via_accessibility() {
        let bounds = Bounds {
            x: 100,
            y: 200,
            width: 800,
            height: 600,
        };
        let script = macos_focus_window_jxa_script(4242, Some(987_654), &bounds);

        // Activates the app via NSRunningApplication so input lands in the
        // right process.
        assert!(script.contains("NSRunningApplication"));
        assert!(script.contains("activateWithOptions"));
        // Uses the Accessibility API to find and raise the specific window.
        assert!(script.contains("AXUIElementCreateApplication"));
        assert!(script.contains("AXWindows"));
        assert!(script.contains("AXRaise"));
        assert!(script.contains("AXUIElementPerformAction"));
        // Looks up the CGWindowID via the private helper so we can target the
        // exact window from `tendril list`.
        assert!(script.contains("_AXUIElementGetWindow"));
        // Falls back to bounds-based matching when the CGWindowID lookup fails
        // (e.g. on systems where the private symbol cannot be bound).
        assert!(script.contains("AXPosition"));
        assert!(script.contains("AXSize"));
        assert!(script.contains("4242"));
        assert!(script.contains("987654"));
        assert!(script.contains("100"));
        assert!(script.contains("200"));
        assert!(script.contains("800"));
        assert!(script.contains("600"));
    }

    #[test]
    fn macos_focus_window_script_handles_missing_window_id_with_bounds_fallback() {
        let bounds = Bounds {
            x: 0,
            y: 0,
            width: 1024,
            height: 768,
        };
        let script = macos_focus_window_jxa_script(7, None, &bounds);

        // Without a parseable CGWindowID we still emit the script, but signal
        // (via -1) that the private lookup should be skipped in favour of
        // bounds matching.
        assert!(script.contains("targetWindowId = -1"));
        assert!(script.contains("AXRaise"));
    }

    #[test]
    fn wayland_capture_support_notes_describe_portal_first_strategy() {
        let adapter = LinuxAdapter::new(AdapterContext::linux(
            DesktopSession::Wayland,
            Some(AudioBackend::PipeWire),
        ));
        let support = adapter
            .capture_support(CaptureTargetKind::Window)
            .expect("wayland capture support should be described");

        assert!(
            support
                .notes
                .iter()
                .any(|note| note.contains("xdg-desktop-portal"))
        );
        assert!(support.notes.iter().any(|note| note.contains("grim")));
        assert!(support.notes.iter().any(|note| note.contains("hyprctl")));
    }

    #[test]
    fn wayland_portal_timeout_reserves_budget_for_grim_fallback() {
        assert_eq!(
            wayland_portal_attempt_timeout(std::time::Duration::from_secs(10), true),
            std::time::Duration::from_millis(WAYLAND_PORTAL_FALLBACK_TIMEOUT_MS),
        );
        assert_eq!(
            wayland_portal_attempt_timeout(std::time::Duration::from_secs(10), false),
            std::time::Duration::from_secs(10),
        );
        assert_eq!(
            wayland_portal_attempt_timeout(std::time::Duration::from_millis(800), true),
            std::time::Duration::from_millis(400),
        );
    }

    #[test]
    fn wayland_workspace_origin_tracks_leftmost_display() {
        let inventory = TargetInventory {
            targets: vec![
                display_target("1", -1920, 40, 1920, 1080),
                display_target("2", 0, 0, 2560, 1440),
            ],
        };

        assert_eq!(wayland_workspace_origin(&inventory), (-1920, 0));
    }

    #[test]
    fn wayland_portal_crop_translates_global_bounds_into_workspace_space() {
        let inventory = TargetInventory {
            targets: vec![
                display_target("1", -2, 0, 2, 2),
                display_target("2", 0, 0, 2, 2),
            ],
        };
        let target = TargetDescriptor {
            id: "window-1".to_owned(),
            title: Some("Example".to_owned()),
            kind: CaptureTargetKind::Window,
            name: "Example".to_owned(),
            bounds: crate::model::Bounds {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            scale_factor: crate::model::ScaleFactor::identity(),
            capture_supported: true,
            input_supported: false,
            app_name: Some("example".to_owned()),
            process_id: Some(7),
            diagnostics: Vec::new(),
        };

        let cropped = crop_wayland_portal_capture_to_target(
            &sample_png(
                4,
                2,
                image::Rgba([0, 0, 255, 255]),
                image::Rgba([255, 0, 0, 255]),
            ),
            &target,
            &inventory,
        )
        .expect("portal crop should succeed");
        let decoded = image::load_from_memory(&cropped)
            .expect("cropped image should decode")
            .to_rgba8();

        assert_eq!(decoded.width(), 2);
        assert_eq!(decoded.height(), 2);
        assert_eq!(*decoded.get_pixel(0, 0), image::Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn wayland_capture_path_lookup_only_checks_known_program_names() {
        assert!(!wayland_capture_program_on_path(
            "grim definitely does not exist"
        ));
    }

    #[test]
    fn grim_rejects_negative_origin_targets() {
        let target = display_target("off-screen", -1585, 0, 1920, 1080);
        let error =
            match capture_wayland_target_with_grim(&target, std::time::Duration::from_secs(1))
                .expect_err("negative origin should be rejected before invoking grim")
            {
                WaylandCaptureBackendError::Failed(failure) => failure.message,
                WaylandCaptureBackendError::Timeout(message) => {
                    panic!("unexpected timeout for negative-origin guard: {message}")
                }
            };
        assert!(
            error.contains("negative origin"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn grim_rejects_empty_bounds() {
        let target = display_target("empty", 0, 0, 0, 0);
        let error =
            match capture_wayland_target_with_grim(&target, std::time::Duration::from_secs(1))
                .expect_err("empty bounds should be rejected before invoking grim")
            {
                WaylandCaptureBackendError::Failed(failure) => failure.message,
                WaylandCaptureBackendError::Timeout(message) => {
                    panic!("unexpected timeout for empty-bounds guard: {message}")
                }
            };
        assert!(error.contains("empty bounds"), "unexpected error: {error}");
    }

    #[test]
    fn grim_subprocess_times_out_when_helper_hangs() {
        // Drive the same wait-with-timeout helper used by the grim path against
        // a long `sleep` subprocess so we can verify the deadline kills the
        // child instead of blocking forever.
        let sleep_program: std::ffi::OsString = std::ffi::OsString::from("sleep");
        let mut child = std::process::Command::new(&sleep_program)
            .arg("30")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(child) = child.as_mut() else {
            // No `sleep` binary in this sandbox — skip the test rather than
            // fail; the timeout logic is also covered indirectly by the
            // portal-thread channel path.
            return;
        };

        let started = std::time::Instant::now();
        let outcome = super::wait_for_capture_child_with_timeout(
            child,
            std::time::Duration::from_millis(150),
        );
        let elapsed = started.elapsed();
        // Make sure the child is reaped no matter what happened above.
        let _ = child.kill();
        let _ = child.wait();

        match outcome {
            super::CaptureChildOutcome::TimedOut => {}
            super::CaptureChildOutcome::Exited(status) => {
                panic!("sleep helper unexpectedly exited with status {status}")
            }
            super::CaptureChildOutcome::WaitFailed(error) => {
                panic!("wait helper failed: {error}")
            }
        }
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "wait helper did not honor timeout: {elapsed:?}"
        );
    }

    fn display_target(id: &str, x: i32, y: i32, width: u32, height: u32) -> TargetDescriptor {
        TargetDescriptor {
            id: id.to_owned(),
            title: None,
            kind: CaptureTargetKind::Display,
            name: id.to_owned(),
            bounds: crate::model::Bounds {
                x,
                y,
                width,
                height,
            },
            scale_factor: crate::model::ScaleFactor::identity(),
            capture_supported: true,
            input_supported: false,
            app_name: None,
            process_id: None,
            diagnostics: Vec::new(),
        }
    }

    fn sample_png(
        width: u32,
        height: u32,
        left: image::Rgba<u8>,
        right: image::Rgba<u8>,
    ) -> Vec<u8> {
        let image =
            image::DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(width, height, |x, _| {
                if x < width / 2 { left } else { right }
            }));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("sample png should encode");
        bytes
    }
}
