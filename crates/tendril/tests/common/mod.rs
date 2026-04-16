use std::io::Write;
use std::process::{Command, Stdio};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::{DynamicImage, ImageBuffer, Rgba};
use serde_json::{Value, json};
use tempfile::TempDir;

pub struct CliHarness {
    _tempdir: TempDir,
    config_dir: std::path::PathBuf,
    target_fixture: String,
    capture_fixture: String,
    input_fixture: String,
}

impl CliHarness {
    pub fn new() -> Self {
        let tempdir = tempfile::tempdir().expect("tempdir should be created");
        let config_dir = tempdir.path().to_path_buf();
        std::fs::write(
            config_dir.join("config.yaml"),
            r"
capture:
  format: jpeg
  compression: 72
  max_width: 2
logging:
  level: error
",
        )
        .expect("config fixture should be writable");

        Self {
            _tempdir: tempdir,
            config_dir,
            target_fixture: sample_target_fixture().to_string(),
            capture_fixture: sample_capture_fixture().to_string(),
            input_fixture: sample_input_fixture().to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn cli_json(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().expect("cli should run");
        assert!(
            output.status.success(),
            "cli failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("cli should emit valid json")
    }

    #[allow(dead_code)]
    pub fn mcp_round_trip(&self, requests: &[Value]) -> Vec<Value> {
        let mut child = self
            .command()
            .args(["mcp", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("mcp stdio should start");

        {
            let mut stdin = child.stdin.take().expect("stdin should be piped");
            for request in requests {
                let frame = frame_request(request);
                stdin
                    .write_all(&frame)
                    .expect("request frame should be written");
            }
        }

        let output = child
            .wait_with_output()
            .expect("mcp stdio should exit cleanly");
        assert!(
            output.status.success(),
            "mcp stdio failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        parse_framed_responses(&output.stdout)
    }

    pub fn apply_env<'a>(&self, command: &'a mut Command) -> &'a mut Command {
        command
            .env("TENDRIL_CONFIG_DIR", &self.config_dir)
            .env("TENDRIL_TARGET_FIXTURE_JSON", &self.target_fixture)
            .env("TENDRIL_CAPTURE_FIXTURE_JSON", &self.capture_fixture)
            .env("TENDRIL_INPUT_FIXTURE_JSON", &self.input_fixture)
            .env("XDG_SESSION_TYPE", "x11")
            .env("DISPLAY", ":99")
            .env_remove("WAYLAND_DISPLAY")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tendril"));
        self.apply_env(&mut command);
        command
    }
}

#[allow(dead_code)]
pub fn frame_request(value: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(value).expect("request should serialize");
    let mut message = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    message.extend(body);
    message
}

#[allow(dead_code)]
pub fn parse_framed_responses(mut bytes: &[u8]) -> Vec<Value> {
    let mut responses = Vec::new();

    while !bytes.is_empty() {
        let header_end = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("framed response should contain a header terminator");
        let (header, remainder) = bytes.split_at(header_end + 4);
        let header_str = std::str::from_utf8(header).expect("header should be valid utf-8");
        let length = header_str
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .expect("response should include Content-Length")
            .trim()
            .parse::<usize>()
            .expect("content length should parse");
        let (body, rest) = remainder.split_at(length);
        responses.push(serde_json::from_slice(body).expect("response body should be json"));
        bytes = rest;
    }

    responses
}

fn sample_target_fixture() -> Value {
    json!({
        "targets": [
            {
                "id": "window-1",
                "title": "Fixture Window",
                "kind": "window",
                "name": "FixtureApp",
                "bounds": {
                    "x": 10,
                    "y": 20,
                    "width": 4,
                    "height": 2
                },
                "scale_factor": {
                    "numerator": 1,
                    "denominator": 1
                },
                "capture_supported": true,
                "input_supported": true,
                "app_name": "FixtureApp",
                "process_id": 4242
            },
            {
                "id": "1",
                "title": null,
                "kind": "display",
                "name": "Fixture Display",
                "bounds": {
                    "x": 0,
                    "y": 0,
                    "width": 8,
                    "height": 4
                },
                "scale_factor": {
                    "numerator": 1,
                    "denominator": 1
                },
                "capture_supported": true,
                "input_supported": true,
                "app_name": null,
                "process_id": null
            }
        ]
    })
}

fn sample_capture_fixture() -> Value {
    json!({
        "media_type": "image/png",
        "image_base64": BASE64.encode(sample_png(4, 2)),
        "captured_at": "2026-04-09T18:00:00Z"
    })
}

fn sample_input_fixture() -> Value {
    json!({
        "focus_required": true,
        "focus_transferred": true,
        "focused_target": "window-1",
        "notes": ["fixture input executed"]
    })
}

fn sample_png(width: u32, height: u32) -> Vec<u8> {
    let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
        width,
        height,
        Rgba([0, 255, 0, 255]),
    ));
    let mut encoded = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut encoded),
            image::ImageFormat::Png,
        )
        .expect("sample image should encode");
    encoded
}
