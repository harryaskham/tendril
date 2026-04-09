use std::env;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use mcp_cli::ErrorCategory;

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
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::UnsupportedCapability(_) => ErrorCategory::UnsupportedCapability,
            Self::MissingPermission { .. } => ErrorCategory::MissingPermission,
            Self::AdapterFailure { .. } => ErrorCategory::PlatformAdapterFailure,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDiscoveryRequest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetDescriptor {
    pub id: String,
    pub title: Option<String>,
    pub kind: CaptureTargetKind,
    pub capture_supported: bool,
    pub input_supported: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetInventory {
    pub targets: Vec<TargetDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRequest {
    pub target: CaptureTargetKind,
    pub target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureArtifact {
    pub target_id: String,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRequest {
    pub target_id: String,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputOutcome {
    pub action_count: usize,
    pub focus_transferred: bool,
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

    fn execute_input(&self, _request: &InputRequest) -> Result<InputOutcome, PlatformAdapterError>;
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
}

#[derive(Debug, Clone)]
pub struct MacOsAdapter {
    context: AdapterContext,
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
        PermissionStatus::unknown(
            PermissionKind::ScreenCapture,
            "macOS capture requires Screen Recording consent for the invoking terminal or binary.",
            "Grant Screen Recording in System Settings > Privacy & Security > Screen Recording, then rerun tendril.",
        )
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
        _request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            self.platform(),
            "target discovery is not implemented in this bead; use capability probes only",
        ))
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

    fn capture(&self, _request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        Err(PlatformAdapterError::missing_permission(
            Capability::DisplayCapture,
            PermissionKind::ScreenCapture,
            self.platform(),
            "Capture execution is gated on explicit Screen Recording consent.",
            "Grant Screen Recording access before invoking capture commands.",
        ))
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

    fn execute_input(&self, _request: &InputRequest) -> Result<InputOutcome, PlatformAdapterError> {
        Err(PlatformAdapterError::missing_permission(
            Capability::InputControl,
            PermissionKind::Accessibility,
            self.platform(),
            "Input injection requires explicit Accessibility consent.",
            "Grant Accessibility access before invoking tendril run.",
        ))
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
        _request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            self.platform(),
            "target discovery is not implemented in this bead; use capability probes only",
        ))
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
            DesktopSession::Wayland => Err(PlatformAdapterError::unsupported(
                capability,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "The Linux adapter spine does not yet provide a compositor-portable Wayland capture backend.",
                Some("Use an X11 session or extend the adapter with a portal/compositor backend."),
            )),
            _ => Err(PlatformAdapterError::unsupported(
                capability,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Capture requires a detected X11 or Wayland desktop session.",
                Some("Run Tendril from a graphical login session and retry."),
            )),
        }
    }

    fn capture(&self, _request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            self.platform(),
            "capture execution is not implemented in this bead; use capability probes only",
        ))
    }
}

impl InputControlAdapter for LinuxAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        match self.context.session {
            DesktopSession::X11 => Ok(FeatureSupport::available(Capability::InputControl)
                .with_notes(vec![
                    "Input injection remains stateless and target-scoped on X11.".to_owned(),
                ])),
            DesktopSession::Wayland => Err(PlatformAdapterError::unsupported(
                Capability::InputControl,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Wayland input injection is compositor-specific and is not exposed by the generic Linux adapter spine.",
                Some(
                    "Use an X11 session or implement a compositor-specific backend with explicit permissions.",
                ),
            )),
            _ => Err(PlatformAdapterError::unsupported(
                Capability::InputControl,
                self.platform(),
                CapabilityErrorReason::UnsupportedSession,
                "Input control requires a detected graphical desktop session.",
                Some("Run Tendril inside X11 or a supported Wayland compositor session."),
            )),
        }
    }

    fn execute_input(&self, _request: &InputRequest) -> Result<InputOutcome, PlatformAdapterError> {
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::InputControl,
            self.platform(),
            "input execution is not implemented in this bead; use capability probes only",
        ))
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
        _request: &TargetDiscoveryRequest,
    ) -> Result<TargetInventory, PlatformAdapterError> {
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::TargetDiscovery,
            self.platform(),
            "target discovery is not implemented in this bead; use capability probes only",
        ))
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

    fn capture(&self, _request: &CaptureRequest) -> Result<CaptureArtifact, PlatformAdapterError> {
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::Capture,
            self.platform(),
            "capture execution is not implemented in this bead; use capability probes only",
        ))
    }
}

impl InputControlAdapter for WindowsAdapter {
    fn input_support(&self) -> Result<FeatureSupport, PlatformAdapterError> {
        Ok(FeatureSupport::available(Capability::InputControl).with_notes(vec![
            "The adapter may still need to report focus transfer when a target cannot receive background input."
                .to_owned(),
        ]))
    }

    fn execute_input(&self, _request: &InputRequest) -> Result<InputOutcome, PlatformAdapterError> {
        Err(PlatformAdapterError::adapter_failure(
            AdapterOperation::InputControl,
            self.platform(),
            "input execution is not implemented in this bead; use capability probes only",
        ))
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

fn detect_linux_audio_backend(
    pipewire_runtime_dir: Option<&std::ffi::OsStr>,
    pulse_server: Option<&std::ffi::OsStr>,
    pulse_runtime_path: Option<&std::ffi::OsStr>,
) -> Option<AudioBackend> {
    if pipewire_runtime_dir.is_some() {
        Some(AudioBackend::PipeWire)
    } else if pulse_server.is_some() || pulse_runtime_path.is_some() {
        Some(AudioBackend::PulseAudio)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterContext, AudioBackend, AudioCapabilityProbe, AudioSourceKind, CaptureAdapter,
        CaptureTargetKind, DesktopSession, InputControlAdapter, LinuxAdapter, MacOsAdapter,
        PermissionAdapter, PermissionKind, PermissionState, PlatformAdapter, PlatformAdapterError,
        PlatformKind, WindowsAdapter, detect_linux_audio_backend, detect_linux_session,
    };
    use crate::TendrilError;
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
        );

        assert_eq!(detected, Some(AudioBackend::PipeWire));
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
    fn linux_wayland_input_probe_is_actionable() {
        let adapter = LinuxAdapter::new(AdapterContext::linux(
            DesktopSession::Wayland,
            Some(AudioBackend::PipeWire),
        ));
        let error = adapter
            .input_support()
            .expect_err("wayland input should not be generically supported yet");

        match error {
            PlatformAdapterError::UnsupportedCapability(capability) => {
                assert_eq!(capability.capability, super::Capability::InputControl);
                assert_eq!(
                    capability.reason,
                    super::CapabilityErrorReason::UnsupportedSession
                );
                assert!(
                    capability
                        .suggested_action
                        .as_deref()
                        .is_some_and(|message| message.contains("X11"))
                );
            }
            other => panic!("unexpected error: {other:?}"),
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
    fn adapter_permissions_are_explicit_and_stateless() {
        let adapter = WindowsAdapter::new(AdapterContext::windows11());
        let permissions = adapter.permissions();

        assert_eq!(adapter.info().platform, PlatformKind::Windows11);
        assert!(adapter.info().stateless);
        assert_eq!(permissions.len(), 3);
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
}
