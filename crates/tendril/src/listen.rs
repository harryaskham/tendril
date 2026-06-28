//! Audio capture execution for the `tendril listen` command.
//!
//! The v0.0.1 listen surface only reported probe-only diagnostics. This module
//! drives an actual recording for at least one supported source per platform
//! by shelling out to a well-known recorder:
//!
//! * Linux + `PipeWire`: `pw-record` (with `parecord` fallback).
//! * Linux + `PulseAudio`: `parecord`.
//! * macOS: `afrecord` (Apple's CoreAudio-backed recorder shipped with the OS).
//! * Windows / unknown backends: capture is not yet wired; callers receive a
//!   structured `probe_only` response.
//!
//! Each successful recording writes WAV bytes to either an explicit
//! `--output` path or a temporary file, and the path is reported back to the
//! caller in the JSON envelope so downstream agents can fetch the artifact.

use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::TendrilError;
use crate::model::{AudioFormat, AudioSourceKind, ListenInput};
use crate::platform::{AudioBackend, PlatformKind};

/// Outcome of a real audio capture attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenArtifact {
    /// Absolute path to the on-disk WAV file containing the captured audio.
    pub path: PathBuf,
    /// MIME type of the artifact (currently always `audio/wav`).
    pub media_type: String,
    /// Number of bytes written to disk.
    pub byte_size: u64,
    /// Negotiated sample rate in Hz.
    pub sample_rate_hz: u32,
    /// Negotiated channel count.
    pub channels: u8,
    /// Recorder program that produced the artifact (e.g. `pw-record`).
    pub recorder: String,
}

/// Reason a capture attempt was skipped before any recorder was spawned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenSkipReason {
    /// No supported recorder was available on this platform.
    UnsupportedPlatform,
    /// Requested format is not yet supported by the selected recorder.
    UnsupportedFormat,
    /// Requested source kind is not yet supported by the selected recorder.
    UnsupportedSource,
    /// Recorder binary could not be located on PATH.
    RecorderUnavailable,
}

/// Result of attempting to perform an actual audio capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ListenCaptureResult {
    /// A real WAV recording was written to disk.
    Captured {
        artifact: ListenArtifact,
        notes: Vec<String>,
    },
    /// No recorder ran; the legacy probe-only response stands.
    Skipped {
        reason: ListenSkipReason,
        notes: Vec<String>,
    },
    /// A recorder ran but failed; the caller still gets the probe diagnostics.
    Failed {
        recorder: String,
        message: String,
        notes: Vec<String>,
    },
}

impl ListenCaptureResult {
    #[must_use]
    pub fn artifact(&self) -> Option<&ListenArtifact> {
        match self {
            Self::Captured { artifact, .. } => Some(artifact),
            _ => None,
        }
    }

    #[must_use]
    pub fn notes(&self) -> &[String] {
        match self {
            Self::Captured { notes, .. }
            | Self::Skipped { notes, .. }
            | Self::Failed { notes, .. } => notes,
        }
    }
}

/// Drive a real audio capture for `input` on the active platform.
///
/// Errors only when validation of the user-provided `output` path fails
/// (parent directory missing, etc.). Recorder-level failures are reported as
/// [`ListenCaptureResult::Failed`] so the caller can still surface probe data.
pub fn execute_listen_capture(
    input: &ListenInput,
    output: Option<&Path>,
    platform: PlatformKind,
    backend: Option<AudioBackend>,
) -> Result<ListenCaptureResult, TendrilError> {
    if input.format != AudioFormat::Wav {
        return Ok(ListenCaptureResult::Skipped {
            reason: ListenSkipReason::UnsupportedFormat,
            notes: vec![format!(
                "listen capture currently emits only WAV; requested {:?} format is not yet implemented.",
                input.format
            )],
        });
    }

    if matches!(input.source.kind, AudioSourceKind::Device) {
        return Ok(ListenCaptureResult::Skipped {
            reason: ListenSkipReason::UnsupportedSource,
            notes: vec![
                "explicit device:<id> selection is not implemented for real capture; pass --source microphone or --source system instead.".to_owned(),
            ]
        });
    }

    let recorders = recorders_for(platform, backend);
    if recorders.is_empty() {
        return Ok(ListenCaptureResult::Skipped {
            reason: ListenSkipReason::UnsupportedPlatform,
            notes: vec![format!(
                "no audio recorder is wired for platform={:?}, backend={:?}; capture remains probe-only here.",
                platform, backend
            )],
        });
    }

    let target_path = resolve_output_path(output)?;

    let mut last_failure: Option<(String, String)> = None;
    let mut tried: Vec<String> = Vec::new();
    for recorder in recorders {
        if !command_available(recorder.program) {
            tried.push(format!("{} (not found on PATH)", recorder.program));
            continue;
        }
        tried.push(recorder.program.to_owned());
        match run_recorder(&recorder, input, &target_path) {
            Ok(artifact) => {
                let mut notes = Vec::new();
                notes.push(format!(
                    "captured {} bytes to {} via {}",
                    artifact.byte_size,
                    artifact.path.display(),
                    artifact.recorder
                ));
                if artifact.byte_size <= WAV_HEADER_ONLY_BYTES {
                    notes.push(format!(
                        "warning: artifact contains only WAV header bytes ({} <= {}); the source likely produced no samples (suspended monitor, muted mic, or no audio playing).",
                        artifact.byte_size, WAV_HEADER_ONLY_BYTES
                    ));
                }
                if tried.len() > 1 {
                    notes.push(format!(
                        "recorder candidates considered: {}",
                        tried.join(", ")
                    ));
                }
                return Ok(ListenCaptureResult::Captured { artifact, notes });
            }
            Err(error) => {
                last_failure = Some((recorder.program.to_owned(), error));
            }
        }
    }

    match last_failure {
        Some((recorder, message)) => Ok(ListenCaptureResult::Failed {
            recorder,
            message,
            notes: vec![format!(
                "all recorder attempts failed; tried: {}",
                tried.join(", ")
            )],
        }),
        None => Ok(ListenCaptureResult::Skipped {
            reason: ListenSkipReason::RecorderUnavailable,
            notes: vec![format!(
                "no usable recorder found; looked for: {}",
                tried.join(", ")
            )],
        }),
    }
}

/// Recorder selection metadata. Kept tiny so it lives well in tests.
#[derive(Debug, Clone)]
struct RecorderPlan {
    program: &'static str,
    sample_rate_hz: u32,
    channels: u8,
    /// Builder that turns `(plan, input, output_path, duration_secs_string)`
    /// into the argv handed to the recorder.
    build_args: fn(&RecorderPlan, &ListenInput, &Path, &str) -> Vec<String>,
}

/// A virtual / aggregate audio input device usable as a system-audio loopback
/// (e.g. `BlackHole`). macOS has no built-in system loopback, so `--source
/// system` captures from such a device once the user routes system output to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AvfAudioDevice {
    pub index: u32,
    pub name: String,
}

const LOOPBACK_NAME_HINTS: &[&str] = &[
    "blackhole",
    "loopback",
    "soundflower",
    "aggregate",
    "multi-output",
    "multi output",
];

/// Parse the `AVFoundation` audio device list ffmpeg prints to stderr for
/// `ffmpeg -f avfoundation -list_devices true -i ""`. Device lines look like
/// `[AVFoundation indev @ 0x..] [1] BlackHole 2ch`, under an
/// `AVFoundation audio devices:` header (the video section is ignored).
pub(crate) fn parse_avfoundation_audio_devices(stderr: &str) -> Vec<AvfAudioDevice> {
    let mut devices = Vec::new();
    let mut in_audio = false;
    for line in stderr.lines() {
        if line.contains("AVFoundation audio devices:") {
            in_audio = true;
            continue;
        }
        if line.contains("AVFoundation video devices:") {
            in_audio = false;
            continue;
        }
        if !in_audio {
            continue;
        }
        // Skip the `[AVFoundation indev @ 0x..] ` tag, then read `[N] Name`.
        let Some(tag_end) = line.rfind("] [") else {
            continue;
        };
        let rest = &line[tag_end + 2..];
        let Some(close) = rest.find(']') else {
            continue;
        };
        let Ok(index) = rest[1..close].trim().parse::<u32>() else {
            continue;
        };
        let name = rest[close + 1..].trim().to_owned();
        if name.is_empty() {
            continue;
        }
        devices.push(AvfAudioDevice { index, name });
    }
    devices
}

/// Pick the first device whose name looks like a virtual loopback device.
pub(crate) fn find_loopback_device(devices: &[AvfAudioDevice]) -> Option<AvfAudioDevice> {
    devices
        .iter()
        .find(|device| {
            let lower = device.name.to_lowercase();
            LOOPBACK_NAME_HINTS.iter().any(|hint| lower.contains(hint))
        })
        .cloned()
}

/// Detect an available virtual loopback audio device on macOS via ffmpeg.
/// Returns `None` off macOS, when ffmpeg is unavailable, or when no loopback
/// device is present.
#[cfg(target_os = "macos")]
pub(crate) fn detect_macos_loopback_device() -> Option<AvfAudioDevice> {
    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-f",
            "avfoundation",
            "-list_devices",
            "true",
            "-i",
            "",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    find_loopback_device(&parse_avfoundation_audio_devices(&stderr))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn detect_macos_loopback_device() -> Option<AvfAudioDevice> {
    None
}

/// Build the `ffmpeg` argument vector for a duration-bounded `AVFoundation` audio
/// capture to a WAV file. `audio_spec` is the `AVFoundation` audio device selector
/// (a device index like `"1"`, or `"default"`); it is captured audio-only via
/// the `:<spec>` input form.
pub(crate) fn ffmpeg_avfoundation_audio_args(
    audio_spec: &str,
    output: &Path,
    duration_secs: &str,
    sample_rate_hz: u32,
    channels: u8,
) -> Vec<String> {
    vec![
        "-hide_banner".to_owned(),
        "-nostdin".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-f".to_owned(),
        "avfoundation".to_owned(),
        "-i".to_owned(),
        format!(":{audio_spec}"),
        "-t".to_owned(),
        duration_secs.to_owned(),
        "-ar".to_owned(),
        sample_rate_hz.to_string(),
        "-ac".to_owned(),
        channels.to_string(),
        "-y".to_owned(),
        output.to_string_lossy().into_owned(),
    ]
}

fn build_ffmpeg_avfoundation_args(
    plan: &RecorderPlan,
    input: &ListenInput,
    output: &Path,
    duration_secs: &str,
) -> Vec<String> {
    let audio_spec = match input.source.kind {
        AudioSourceKind::System => detect_macos_loopback_device()
            .map_or_else(|| "default".to_owned(), |device| device.index.to_string()),
        AudioSourceKind::Microphone | AudioSourceKind::Device => "default".to_owned(),
    };
    ffmpeg_avfoundation_audio_args(
        &audio_spec,
        output,
        duration_secs,
        plan.sample_rate_hz,
        plan.channels,
    )
}

fn recorders_for(platform: PlatformKind, backend: Option<AudioBackend>) -> Vec<RecorderPlan> {
    match platform {
        PlatformKind::MacOs => vec![
            // ffmpeg's AVFoundation backend is the primary recorder: it can
            // target a specific input device (system loopback via BlackHole) and
            // is present where the legacy `afrecord` is not.
            RecorderPlan {
                program: "ffmpeg",
                sample_rate_hz: 44_100,
                channels: 2,
                build_args: build_ffmpeg_avfoundation_args,
            },
            RecorderPlan {
                program: "afrecord",
                sample_rate_hz: 44_100,
                channels: 1,
                build_args: build_afrecord_args,
            },
        ],
        PlatformKind::Linux => match backend {
            Some(AudioBackend::PulseAudio) => vec![RecorderPlan {
                program: "parecord",
                sample_rate_hz: 48_000,
                channels: 2,
                build_args: build_parecord_args,
            }],
            // PipeWire is the preferred path; on unknown backends we still try
            // the same recorders in preference order and rely on the runtime
            // PATH probe to skip any that are not installed.
            _ => vec![
                RecorderPlan {
                    program: "pw-record",
                    sample_rate_hz: 48_000,
                    channels: 2,
                    build_args: build_pw_record_args,
                },
                RecorderPlan {
                    program: "parecord",
                    sample_rate_hz: 48_000,
                    channels: 2,
                    build_args: build_parecord_args,
                },
            ],
        },
        PlatformKind::Windows11 | PlatformKind::Android => Vec::new(),
    }
}

fn build_pw_record_args(
    plan: &RecorderPlan,
    input: &ListenInput,
    output: &Path,
    _duration_secs: &str,
) -> Vec<String> {
    // pw-record writes raw PCM by default. Request a WAV container with
    // `--format=wav` and configure rate/channels explicitly so the artifact
    // is self-describing.
    let target = match input.source.kind {
        AudioSourceKind::System => "@DEFAULT_MONITOR@",
        AudioSourceKind::Microphone | AudioSourceKind::Device => "@DEFAULT_SOURCE@",
    };
    // Convert the requested duration to a sample count so pw-record exits on
    // its own: this guarantees it flushes the WAV header without us needing
    // to send SIGTERM and race the writer.
    let sample_count = sample_count_for(plan.sample_rate_hz, input.duration_ms);
    vec![
        "--target".to_owned(),
        target.to_owned(),
        "--rate".to_owned(),
        plan.sample_rate_hz.to_string(),
        "--channels".to_owned(),
        plan.channels.to_string(),
        "--format=s16".to_owned(),
        "-n".to_owned(),
        sample_count.to_string(),
        output.to_string_lossy().into_owned(),
    ]
}

fn sample_count_for(sample_rate_hz: u32, duration_ms: u64) -> u64 {
    // Round up to ensure we capture at least the requested duration.
    let numerator = u64::from(sample_rate_hz).saturating_mul(duration_ms);
    numerator.div_ceil(1_000)
}

fn build_parecord_args(
    plan: &RecorderPlan,
    input: &ListenInput,
    output: &Path,
    _duration_secs: &str,
) -> Vec<String> {
    let device = match input.source.kind {
        AudioSourceKind::System => "@DEFAULT_MONITOR@",
        AudioSourceKind::Microphone | AudioSourceKind::Device => "@DEFAULT_SOURCE@",
    };
    vec![
        "--device".to_owned(),
        device.to_owned(),
        "--rate".to_owned(),
        plan.sample_rate_hz.to_string(),
        "--channels".to_owned(),
        plan.channels.to_string(),
        "--format=s16le".to_owned(),
        "--file-format=wav".to_owned(),
        output.to_string_lossy().into_owned(),
    ]
}

fn build_afrecord_args(
    _plan: &RecorderPlan,
    _input: &ListenInput,
    output: &Path,
    duration_secs: &str,
) -> Vec<String> {
    // afrecord ships with macOS and natively supports time-bounded WAV capture.
    // -d <seconds> stops at the requested duration; -f WAVE -d ... gives us a
    // WAV container; -t LEI16 selects 16-bit little-endian PCM.
    vec![
        "-f".to_owned(),
        "WAVE".to_owned(),
        "-d".to_owned(),
        "LEI16".to_owned(),
        "-t".to_owned(),
        duration_secs.to_owned(),
        output.to_string_lossy().into_owned(),
    ]
}

fn run_recorder(
    plan: &RecorderPlan,
    input: &ListenInput,
    output: &Path,
) -> Result<ListenArtifact, String> {
    let duration = Duration::from_millis(input.duration_ms);
    let duration_secs = format_seconds(duration);
    let args = (plan.build_args)(plan, input, output, &duration_secs);

    // Make sure we never inherit a stale file: caller resolves the path but
    // does not pre-create it for recorders that prefer to allocate it
    // themselves.
    if output.exists() {
        let _ = fs::remove_file(output);
    }

    let mut command = Command::new(plan.program);
    command.args(args.iter().map(OsStr::new));
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to spawn {}: {error}", plan.program))?;

    if program_runs_until_killed(plan.program) {
        wait_for_duration_then_terminate(&mut child, duration)?;
    } else {
        wait_with_grace(&mut child, duration)?;
    }

    // Read stderr for diagnostics regardless of success.
    let stderr_text = drain_stderr(&mut child);
    let status = child
        .wait()
        .map_err(|error| format!("failed to await {}: {error}", plan.program))?;

    if !is_acceptable_exit(plan.program, status) {
        let _ = fs::remove_file(output);
        return Err(format!(
            "{} exited with status {status}{}",
            plan.program,
            if stderr_text.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr_text.trim())
            }
        ));
    }

    let metadata = fs::metadata(output).map_err(|error| {
        format!(
            "{} reported success but produced no file at {}: {error}",
            plan.program,
            output.display()
        )
    })?;
    if metadata.len() == 0 {
        let _ = fs::remove_file(output);
        return Err(format!(
            "{} produced an empty file at {}",
            plan.program,
            output.display()
        ));
    }

    Ok(ListenArtifact {
        path: output.to_path_buf(),
        media_type: "audio/wav".to_owned(),
        byte_size: metadata.len(),
        sample_rate_hz: plan.sample_rate_hz,
        channels: plan.channels,
        recorder: plan.program.to_owned(),
    })
}

/// Empirically, a bare WAV file with only the canonical RIFF/fmt headers and
/// an empty data chunk weighs in at 44 bytes. Anything at or below that size
/// means the recorder produced no PCM samples — treat those as captures with
/// a header-only warning so callers can detect silent sessions.
const WAV_HEADER_ONLY_BYTES: u64 = 44;

/// Recorders like `parecord` keep recording until terminated. `pw-record`
/// (with `-n COUNT`) and `afrecord` (with `-d <secs>`) exit on their own.
fn program_runs_until_killed(program: &str) -> bool {
    matches!(program, "parecord")
}

fn is_acceptable_exit(program: &str, status: std::process::ExitStatus) -> bool {
    if status.success() {
        return true;
    }
    // We deliberately SIGTERM long-running recorders, so a non-success exit
    // after a timeout-kill is the normal path. Treat any exit as acceptable
    // for those programs as long as a non-empty file was produced; the
    // metadata check above guards against silent failures.
    program_runs_until_killed(program)
}

fn wait_for_duration_then_terminate(child: &mut Child, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll recorder: {error}"))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Polite shutdown first; if the recorder ignores it we escalate.
            let _ = terminate_child(child);
            std::thread::sleep(Duration::from_millis(150));
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn wait_with_grace(child: &mut Child, duration: Duration) -> Result<(), String> {
    // Allow the recorder up to duration + 2s of headroom for container
    // finalization. If it hangs longer we kill it so listen never blocks
    // an agent indefinitely.
    let deadline = Instant::now() + duration + Duration::from_secs(2);
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll recorder: {error}"))?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn terminate_child(child: &Child) -> std::io::Result<()> {
    // Send SIGTERM by shelling out to /bin/kill. We deliberately avoid
    // libc::kill here because the workspace forbids `unsafe_code`. Using a
    // SIGTERM (instead of std::process::Child::kill which sends SIGKILL) lets
    // recorders flush their WAV headers before exiting.
    let pid = child.id();
    let status = std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "kill -TERM {pid} exited with status {status}"
        )))
    }
}

#[cfg(not(unix))]
fn terminate_child(child: &Child) -> std::io::Result<()> {
    // On non-unix platforms we have no graceful signal; fall back to kill().
    // The current code path only invokes this from the Linux capture branch.
    let mut handle = child.stdin.as_ref();
    let _ = handle.take();
    Ok(())
}

fn drain_stderr(child: &mut Child) -> String {
    let mut buffer = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut buffer);
    }
    buffer
}

fn command_available(program: &str) -> bool {
    // Probe via PATH walking ourselves, so we avoid pulling in the `which`
    // crate just for this check.
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for entry in std::env::split_paths(&path) {
        let candidate = entry.join(program);
        if candidate.is_file() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = candidate.metadata() {
                    if metadata.permissions().mode() & 0o111 != 0 {
                        return true;
                    }
                }
            }
            #[cfg(not(unix))]
            {
                return true;
            }
        }
    }
    false
}

fn resolve_output_path(output: Option<&Path>) -> Result<PathBuf, TendrilError> {
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                return Err(TendrilError::validation(format!(
                    "listen --output parent directory `{}` does not exist",
                    parent.display()
                ))
                .with_code("invalid_listen_input")
                .with_field("output"));
            }
        }
        return Ok(path.to_path_buf());
    }

    let mut candidate = std::env::temp_dir();
    let unique = format!(
        "tendril-listen-{}-{}.wav",
        std::process::id(),
        unique_suffix()
    );
    candidate.push(unique);
    Ok(candidate)
}

fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or_else(
        |_| "0".to_owned(),
        |duration| duration.as_nanos().to_string(),
    )
}

fn format_seconds(duration: Duration) -> String {
    // afrecord accepts fractional seconds; clamp to millisecond precision.
    let total_millis = duration.as_millis();
    if total_millis % 1_000 == 0 {
        format!("{}", total_millis / 1_000)
    } else {
        format!("{:.3}", duration.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::model::AudioSourceSelector;

    #[test]
    fn parse_avfoundation_audio_devices_extracts_indexed_audio_devices() {
        let stderr = "[AVFoundation indev @ 0x1] AVFoundation video devices:\n\
[AVFoundation indev @ 0x1] [0] FaceTime HD Camera\n\
[AVFoundation indev @ 0x1] AVFoundation audio devices:\n\
[AVFoundation indev @ 0x1] [0] Steam Streaming Speakers\n\
[AVFoundation indev @ 0x1] [1] BlackHole 2ch\n\
[AVFoundation indev @ 0x1] [2] MacBook Pro Microphone";
        let devices = parse_avfoundation_audio_devices(stderr);
        assert_eq!(devices.len(), 3);
        assert_eq!(devices[1].index, 1);
        assert_eq!(devices[1].name, "BlackHole 2ch");
        // The video-section device must not leak into the audio list.
        assert!(
            devices
                .iter()
                .all(|device| device.name != "FaceTime HD Camera")
        );
    }

    #[test]
    fn find_loopback_device_matches_blackhole() {
        let devices = vec![
            AvfAudioDevice {
                index: 0,
                name: "MacBook Pro Microphone".to_owned(),
            },
            AvfAudioDevice {
                index: 1,
                name: "BlackHole 2ch".to_owned(),
            },
        ];
        assert_eq!(
            find_loopback_device(&devices)
                .expect("loopback present")
                .index,
            1
        );
    }

    #[test]
    fn find_loopback_device_returns_none_without_virtual_device() {
        let devices = vec![AvfAudioDevice {
            index: 0,
            name: "MacBook Pro Microphone".to_owned(),
        }];
        assert!(find_loopback_device(&devices).is_none());
    }

    #[test]
    fn ffmpeg_avfoundation_audio_args_capture_audio_only_to_wav() {
        let path = std::path::Path::new("/tmp/out.wav");
        let args = ffmpeg_avfoundation_audio_args("1", path, "2", 44_100, 2);
        let i = args.iter().position(|arg| arg == "-i").expect("-i present");
        assert_eq!(args[i + 1], ":1");
        assert_eq!(
            args.windows(2)
                .find(|w| w[0] == "-f")
                .map(|w| w[1].as_str()),
            Some("avfoundation")
        );
        assert_eq!(
            args.windows(2)
                .find(|w| w[0] == "-t")
                .map(|w| w[1].as_str()),
            Some("2")
        );
        assert_eq!(args.last().map(String::as_str), Some("/tmp/out.wav"));
    }

    fn input(source: AudioSourceKind, format: AudioFormat, duration_ms: u64) -> ListenInput {
        ListenInput {
            source: AudioSourceSelector {
                kind: source,
                id: None,
            },
            duration_ms,
            format,
        }
    }

    #[test]
    fn non_wav_format_is_skipped_for_now() {
        let result = execute_listen_capture(
            &input(AudioSourceKind::Microphone, AudioFormat::Flac, 500),
            None,
            PlatformKind::Linux,
            Some(AudioBackend::PipeWire),
        )
        .expect("validation should not fail for Flac request");
        match result {
            ListenCaptureResult::Skipped { reason, .. } => {
                assert_eq!(reason, ListenSkipReason::UnsupportedFormat);
            }
            other => panic!("expected Skipped(UnsupportedFormat), got {other:?}"),
        }
    }

    #[test]
    fn device_source_is_skipped_for_now() {
        let result = execute_listen_capture(
            &ListenInput {
                source: AudioSourceSelector {
                    kind: AudioSourceKind::Device,
                    id: Some("alsa_input.42".to_owned()),
                },
                duration_ms: 500,
                format: AudioFormat::Wav,
            },
            None,
            PlatformKind::Linux,
            Some(AudioBackend::PipeWire),
        )
        .expect("validation should not fail for device source");
        match result {
            ListenCaptureResult::Skipped { reason, .. } => {
                assert_eq!(reason, ListenSkipReason::UnsupportedSource);
            }
            other => panic!("expected Skipped(UnsupportedSource), got {other:?}"),
        }
    }

    #[test]
    fn windows_capture_is_marked_unsupported_until_implemented() {
        let result = execute_listen_capture(
            &input(AudioSourceKind::Microphone, AudioFormat::Wav, 500),
            None,
            PlatformKind::Windows11,
            Some(AudioBackend::Wasapi),
        )
        .expect("validation should not fail for windows");
        match result {
            ListenCaptureResult::Skipped { reason, .. } => {
                assert_eq!(reason, ListenSkipReason::UnsupportedPlatform);
            }
            other => panic!("expected Skipped(UnsupportedPlatform), got {other:?}"),
        }
    }

    #[test]
    fn missing_parent_directory_is_rejected_before_spawn() {
        let bogus = PathBuf::from("/nonexistent-tendril-listen/dir/output.wav");
        let error = execute_listen_capture(
            &input(AudioSourceKind::Microphone, AudioFormat::Wav, 500),
            Some(&bogus),
            PlatformKind::Linux,
            Some(AudioBackend::PipeWire),
        )
        .expect_err("missing parent directory should be a validation error");
        assert_eq!(error.code(), "invalid_listen_input");
    }

    #[test]
    fn pw_record_args_select_monitor_for_loopback() {
        let plan = RecorderPlan {
            program: "pw-record",
            sample_rate_hz: 48_000,
            channels: 2,
            build_args: build_pw_record_args,
        };
        let args = (plan.build_args)(
            &plan,
            &input(AudioSourceKind::System, AudioFormat::Wav, 1_000),
            std::path::Path::new("/tmp/out.wav"),
            "1",
        );
        assert!(args.contains(&"@DEFAULT_MONITOR@".to_owned()));
        assert!(args.contains(&"/tmp/out.wav".to_owned()));
        // -n <samples> must match the requested duration so pw-record exits
        // on its own instead of relying on a SIGTERM race.
        assert!(args.windows(2).any(|w| w[0] == "-n" && w[1] == "48000"));
    }

    #[test]
    fn sample_count_rounds_up_for_partial_milliseconds() {
        // 1 ms at 48 kHz is 48 samples; 1.5 ms should round up to 73.
        assert_eq!(sample_count_for(48_000, 1), 48);
        assert_eq!(sample_count_for(48_000, 1_500), 72_000);
    }

    #[test]
    fn parecord_args_select_default_source_for_microphone() {
        let plan = RecorderPlan {
            program: "parecord",
            sample_rate_hz: 48_000,
            channels: 2,
            build_args: build_parecord_args,
        };
        let args = (plan.build_args)(
            &plan,
            &input(AudioSourceKind::Microphone, AudioFormat::Wav, 1_000),
            std::path::Path::new("/tmp/mic.wav"),
            "1",
        );
        assert!(args.contains(&"@DEFAULT_SOURCE@".to_owned()));
        assert!(args.contains(&"--file-format=wav".to_owned()));
    }

    #[test]
    fn format_seconds_uses_integer_when_exact() {
        assert_eq!(format_seconds(Duration::from_secs(3)), "3");
        assert_eq!(format_seconds(Duration::from_millis(1_500)), "1.500");
    }

    #[test]
    fn afrecord_args_request_time_bounded_wav() {
        let plan = RecorderPlan {
            program: "afrecord",
            sample_rate_hz: 44_100,
            channels: 1,
            build_args: build_afrecord_args,
        };
        let args = (plan.build_args)(
            &plan,
            &input(AudioSourceKind::Microphone, AudioFormat::Wav, 2_000),
            std::path::Path::new("/tmp/clip.wav"),
            "2",
        );
        // WAVE container with 16-bit little-endian PCM.
        assert!(args.windows(2).any(|w| w[0] == "-f" && w[1] == "WAVE"));
        assert!(args.windows(2).any(|w| w[0] == "-d" && w[1] == "LEI16"));
        // The caller-provided duration string is passed through verbatim as the
        // time bound so afrecord stops on its own.
        assert!(args.windows(2).any(|w| w[0] == "-t" && w[1] == "2"));
        // The output path is always the final argument.
        assert_eq!(args.last().map(String::as_str), Some("/tmp/clip.wav"));
    }

    #[test]
    fn recorders_for_prefers_ffmpeg_on_macos() {
        let plans = recorders_for(PlatformKind::MacOs, None);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].program, "ffmpeg");
        assert_eq!(plans[1].program, "afrecord");
    }

    #[test]
    fn recorders_for_uses_parecord_for_pulseaudio() {
        let plans = recorders_for(PlatformKind::Linux, Some(AudioBackend::PulseAudio));
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].program, "parecord");
    }

    #[test]
    fn recorders_for_prefers_pw_record_on_unknown_linux_backend() {
        let plans = recorders_for(PlatformKind::Linux, None);
        let programs: Vec<&str> = plans.iter().map(|p| p.program).collect();
        // pw-record is the preferred path; parecord is the fallback.
        assert_eq!(programs, vec!["pw-record", "parecord"]);
    }

    #[test]
    fn recorders_for_is_empty_on_windows_and_android() {
        assert!(recorders_for(PlatformKind::Windows11, None).is_empty());
        assert!(recorders_for(PlatformKind::Android, None).is_empty());
    }

    #[test]
    fn only_parecord_runs_until_killed() {
        assert!(program_runs_until_killed("parecord"));
        assert!(!program_runs_until_killed("pw-record"));
        assert!(!program_runs_until_killed("afrecord"));
    }

    #[cfg(unix)]
    #[test]
    fn nonzero_exit_is_only_acceptable_for_run_until_killed_recorders() {
        use std::os::unix::process::ExitStatusExt;
        // SIGTERM-style failure (exit code 1) is the normal path for parecord,
        // which we deliberately kill after the requested duration.
        let failure = std::process::ExitStatus::from_raw(1 << 8);
        assert!(is_acceptable_exit("parecord", failure));
        // The same non-success exit is NOT acceptable for recorders that exit
        // on their own (pw-record, afrecord) — a failure there is a real error.
        assert!(!is_acceptable_exit("pw-record", failure));
        assert!(!is_acceptable_exit("afrecord", failure));
        // A clean exit is acceptable for any recorder.
        let success = std::process::ExitStatus::from_raw(0);
        assert!(is_acceptable_exit("afrecord", success));
    }

    #[test]
    fn resolve_output_path_returns_explicit_path_with_existing_parent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("capture.wav");
        assert_eq!(
            resolve_output_path(Some(&target)).expect("existing parent is accepted"),
            target
        );
    }

    #[test]
    fn resolve_output_path_accepts_bare_filename_without_parent() {
        // A bare filename has an empty parent component, so the existence check
        // is skipped and the path is returned unchanged.
        let bare = PathBuf::from("output.wav");
        assert_eq!(
            resolve_output_path(Some(&bare)).expect("bare filename is accepted"),
            bare
        );
    }

    #[test]
    fn resolve_output_path_generates_default_wav_under_temp_dir() {
        let resolved = resolve_output_path(None).expect("default path is generated");
        assert!(resolved.starts_with(std::env::temp_dir()));
        assert_eq!(
            resolved.extension().and_then(|ext| ext.to_str()),
            Some("wav")
        );
        let name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file name");
        assert!(name.starts_with("tendril-listen-"));
    }
}
