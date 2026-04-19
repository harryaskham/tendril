use tendril::platform::{
    AdapterContext, AudioBackend, AudioCapabilityProbe, AudioSourceKind, CaptureAdapter,
    CaptureTargetKind, DesktopSession, InputControlAdapter, LinuxAdapter, MacOsAdapter,
    PermissionState, PlatformAdapter, PlatformAdapterError, PlatformKind, WindowsAdapter,
};

#[test]
fn adapters_expose_stateless_permissions_and_capability_contracts() {
    let adapters: Vec<(PlatformKind, Box<dyn PlatformAdapter>)> = vec![
        (
            PlatformKind::MacOs,
            Box::new(MacOsAdapter::new(AdapterContext::macos())),
        ),
        (
            PlatformKind::Linux,
            Box::new(LinuxAdapter::new(AdapterContext::linux(
                DesktopSession::X11,
                Some(AudioBackend::PipeWire),
            ))),
        ),
        (
            PlatformKind::Windows11,
            Box::new(WindowsAdapter::new(AdapterContext::windows11())),
        ),
    ];

    for (platform, adapter) in adapters {
        let info = adapter.info();
        assert_eq!(info.platform, platform);
        assert!(info.stateless);

        let permissions = adapter.permissions();
        assert!(
            permissions.len() >= 3,
            "expected explicit permissions for {platform:?}"
        );
        assert!(
            permissions
                .iter()
                .all(|permission| !permission.summary.trim().is_empty())
        );
        assert!(permissions.iter().all(|permission| {
            matches!(
                permission.state,
                PermissionState::Granted
                    | PermissionState::NotRequired
                    | PermissionState::Unknown
                    | PermissionState::Denied
            )
        }));

        let discovery = adapter
            .target_discovery_support()
            .expect("target discovery support should be described");
        assert!(!discovery.notes.is_empty() || !discovery.permissions.is_empty());

        let window_capture = adapter
            .capture_support(CaptureTargetKind::Window)
            .expect("window capture support should be described");
        assert_eq!(
            window_capture.capability,
            tendril::platform::Capability::WindowCapture
        );

        let display_capture = adapter
            .capture_support(CaptureTargetKind::Display)
            .expect("display capture support should be described");
        assert_eq!(
            display_capture.capability,
            tendril::platform::Capability::DisplayCapture
        );

        let input_support = adapter
            .input_support()
            .expect("input support should be described");
        assert_eq!(
            input_support.capability,
            tendril::platform::Capability::InputControl
        );

        let microphone = adapter
            .probe_audio_capture(&tendril::platform::AudioProbeRequest {
                source: AudioSourceKind::Microphone,
                duration_hint_ms: Some(250),
            })
            .expect("microphone support should be described");
        assert!(!microphone.supported_sample_rates_hz.is_empty());
        assert!(!microphone.supported_channel_counts.is_empty());
    }
}

#[test]
fn unsupported_platform_paths_surface_structured_capability_errors() {
    let linux_wayland = LinuxAdapter::new(AdapterContext::linux(
        DesktopSession::Wayland,
        Some(AudioBackend::PipeWire),
    ));

    let capture_support = linux_wayland
        .capture_support(CaptureTargetKind::Window)
        .expect("wayland capture support should describe compositor-aware backends");
    assert_eq!(
        capture_support.capability,
        tendril::platform::Capability::WindowCapture
    );
    assert!(
        capture_support
            .notes
            .iter()
            .any(|note| note.contains("grim") || note.contains("Wayland"))
    );

    // Wayland input support now succeeds when ydotool/wtype are present
    // and otherwise returns an actionable UnsupportedFeature diagnostic that
    // names both helper tools (bd-408572).
    match linux_wayland.input_support() {
        Ok(support) => {
            assert_eq!(
                support.capability,
                tendril::platform::Capability::InputControl
            );
            assert!(
                support
                    .notes
                    .iter()
                    .any(|note| note.contains("ydotool") || note.contains("wtype")),
                "supported branch should describe the helper tool that backs Wayland input"
            );
        }
        Err(PlatformAdapterError::UnsupportedCapability(capability)) => {
            assert_eq!(
                capability.capability,
                tendril::platform::Capability::InputControl
            );
            assert_eq!(capability.platform, PlatformKind::Linux);
            assert_eq!(
                capability.reason,
                tendril::platform::CapabilityErrorReason::UnsupportedFeature
            );
            assert!(
                capability.message.contains("ydotool") && capability.message.contains("wtype"),
                "missing-backend diagnostic should name both Wayland helpers: {}",
                capability.message
            );
            assert!(
                capability
                    .suggested_action
                    .as_deref()
                    .is_some_and(|message| message.contains("ydotoold")),
                "missing-backend diagnostic should mention the ydotoold daemon"
            );
        }
        Err(other) => panic!("unexpected error: {other:?}"),
    }

    let macos = MacOsAdapter::new(AdapterContext::macos());
    let loopback_error = macos
        .probe_audio_capture(&tendril::platform::AudioProbeRequest {
            source: AudioSourceKind::SystemLoopback,
            duration_hint_ms: Some(250),
        })
        .expect_err("macos loopback should remain unsupported in v0.0.1");

    match loopback_error {
        PlatformAdapterError::UnsupportedCapability(capability) => {
            assert_eq!(
                capability.capability,
                tendril::platform::Capability::AudioLoopbackCapture
            );
            assert_eq!(capability.platform, PlatformKind::MacOs);
            assert!(capability.suggested_action.is_some());
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[allow(dead_code)]
fn assert_unsupported_session(
    error: PlatformAdapterError,
    expected_capability: tendril::platform::Capability,
) {
    match error {
        PlatformAdapterError::UnsupportedCapability(capability) => {
            assert_eq!(capability.capability, expected_capability);
            assert_eq!(capability.platform, PlatformKind::Linux);
            assert_eq!(
                capability.reason,
                tendril::platform::CapabilityErrorReason::UnsupportedSession
            );
            assert!(capability.suggested_action.is_some());
        }
        other => panic!("unexpected error: {other:?}"),
    }
}
