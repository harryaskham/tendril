use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use image::codecs::jpeg::JpegEncoder;
use image::{
    DynamicImage, GenericImageView, ImageFormat as RasterImageFormat, imageops::FilterType,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::config::ImageFormat;
use crate::error::TendrilError;
use crate::model::{Bounds, CaptureInput, CaptureOutput, CoordinateTransform, TargetSelector};
use crate::platform::{
    CaptureArtifact, CaptureRequest as PlatformCaptureRequest, CaptureTargetKind, PlatformAdapter,
    TargetDescriptor as PlatformTargetDescriptor, TargetDiscoveryRequest,
};

pub(crate) fn execute_capture(
    input: &CaptureInput,
    adapter: &dyn PlatformAdapter,
) -> Result<CaptureOutput, TendrilError> {
    let target = resolve_target(input, adapter)?;
    ensure_capture_supported(&target)?;
    adapter.capture_support(target.kind)?;

    let artifact = adapter.capture(&PlatformCaptureRequest {
        target: target.kind,
        target_id: target.id.clone(),
    })?;

    build_capture_output(input, &target, artifact, &adapter.info())
}

fn resolve_target(
    input: &CaptureInput,
    adapter: &dyn PlatformAdapter,
) -> Result<PlatformTargetDescriptor, TendrilError> {
    let inventory = adapter.discover_targets(&TargetDiscoveryRequest)?;
    inventory
        .targets
        .into_iter()
        .find(|target| {
            target.id == input.target.id() && matches_target_kind(&input.target, target.kind)
        })
        .ok_or_else(|| {
            TendrilError::target_not_found(
                match input.target.kind() {
                    crate::model::TargetKind::Window => "window",
                    crate::model::TargetKind::Display => "display",
                    crate::model::TargetKind::AudioSource => "audio_source",
                },
                input.target.id(),
            )
        })
}

fn matches_target_kind(target: &TargetSelector, platform_kind: CaptureTargetKind) -> bool {
    matches!(
        (target, platform_kind),
        (TargetSelector::Window { .. }, CaptureTargetKind::Window)
            | (TargetSelector::Display { .. }, CaptureTargetKind::Display)
    )
}

fn ensure_capture_supported(target: &PlatformTargetDescriptor) -> Result<(), TendrilError> {
    if target.capture_supported {
        Ok(())
    } else {
        Err(TendrilError::unsupported_capability(
            "capture_not_supported_for_target",
            format!("target `{}` does not support capture", target.id),
            Some(serde_json::json!({
                "target_id": target.id,
                "target_kind": target.kind,
            })),
        ))
    }
}

fn build_capture_output(
    input: &CaptureInput,
    target: &PlatformTargetDescriptor,
    artifact: CaptureArtifact,
    adapter: &crate::platform::AdapterInfo,
) -> Result<CaptureOutput, TendrilError> {
    let decoded = image::load_from_memory(&artifact.image_bytes).map_err(|error| {
        TendrilError::execution_failure(
            "capture_decode_failed",
            format!("captured image could not be decoded: {error}"),
            None,
        )
    })?;

    let (original_width, original_height) = decoded.dimensions();
    let (output_width, output_height) = resized_dimensions(
        original_width,
        original_height,
        input.max_width,
        input.max_height,
    );

    let rendered = if output_width == original_width && output_height == original_height {
        decoded
    } else {
        decoded.resize_exact(output_width, output_height, FilterType::Lanczos3)
    };

    let image_bytes = encode_image(&rendered, input.format, input.compression)?;

    Ok(CaptureOutput {
        adapter: adapter.clone(),
        target: input.target.clone(),
        original_bounds: Bounds {
            x: target.bounds.x,
            y: target.bounds.y,
            width: original_width,
            height: original_height,
        },
        output_bounds: Bounds {
            x: target.bounds.x,
            y: target.bounds.y,
            width: output_width,
            height: output_height,
        },
        source_to_output: CoordinateTransform {
            x_numerator: output_width,
            x_denominator: original_width,
            y_numerator: output_height,
            y_denominator: original_height,
        },
        output_to_source: CoordinateTransform {
            x_numerator: original_width,
            x_denominator: output_width,
            y_numerator: original_height,
            y_denominator: output_height,
        },
        resized: output_width != original_width || output_height != original_height,
        format: input.format,
        compression: input.compression,
        media_type: media_type_for_format(input.format).to_owned(),
        image_base64: BASE64.encode(image_bytes),
        captured_at: artifact.captured_at,
    })
}

fn encode_image(
    image: &DynamicImage,
    format: ImageFormat,
    compression: u8,
) -> Result<Vec<u8>, TendrilError> {
    match format {
        ImageFormat::Png => {
            let mut buffer = Vec::new();
            image
                .write_to(&mut Cursor::new(&mut buffer), RasterImageFormat::Png)
                .map_err(|error| {
                    TendrilError::execution_failure(
                        "capture_encode_failed",
                        format!("failed to encode png capture: {error}"),
                        None,
                    )
                })?;
            Ok(buffer)
        }
        ImageFormat::Jpeg => {
            let mut buffer = Vec::new();
            let mut encoder = JpegEncoder::new_with_quality(&mut buffer, compression.max(1));
            encoder.encode_image(image).map_err(|error| {
                TendrilError::execution_failure(
                    "capture_encode_failed",
                    format!("failed to encode jpeg capture: {error}"),
                    None,
                )
            })?;
            Ok(buffer)
        }
    }
}

pub(crate) fn resized_dimensions(
    original_width: u32,
    original_height: u32,
    max_width: Option<u32>,
    max_height: Option<u32>,
) -> (u32, u32) {
    let mut scale_numerator = 1_u64;
    let mut scale_denominator = 1_u64;

    if let Some(width) = max_width {
        let candidate_numerator = u64::from(width);
        let candidate_denominator = u64::from(original_width);
        if candidate_numerator < candidate_denominator
            && candidate_numerator.saturating_mul(scale_denominator)
                < scale_numerator.saturating_mul(candidate_denominator)
        {
            scale_numerator = candidate_numerator;
            scale_denominator = candidate_denominator;
        }
    }

    if let Some(height) = max_height {
        let candidate_numerator = u64::from(height);
        let candidate_denominator = u64::from(original_height);
        if candidate_numerator < candidate_denominator
            && candidate_numerator.saturating_mul(scale_denominator)
                < scale_numerator.saturating_mul(candidate_denominator)
        {
            scale_numerator = candidate_numerator;
            scale_denominator = candidate_denominator;
        }
    }

    if scale_numerator >= scale_denominator {
        return (original_width, original_height);
    }

    let width = rounded_ratio(
        u64::from(original_width),
        scale_numerator,
        scale_denominator,
    );
    let height = rounded_ratio(
        u64::from(original_height),
        scale_numerator,
        scale_denominator,
    );

    (
        u32::try_from(width.max(1)).expect("resized width should fit in u32"),
        u32::try_from(height.max(1)).expect("resized height should fit in u32"),
    )
}

fn rounded_ratio(value: u64, numerator: u64, denominator: u64) -> u64 {
    value
        .saturating_mul(numerator)
        .saturating_add(denominator / 2)
        / denominator
}

fn media_type_for_format(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "image/png",
        ImageFormat::Jpeg => "image/jpeg",
    }
}

pub(crate) fn render_capture_human(output: &CaptureOutput) -> String {
    format!(
        "capture target: {:?} {}\nplatform: {:?} / {:?}\noriginal: {}x{}\noutput: {}x{}\nresized: {}\nsource_to_output: {}/{} x {}/{}\noutput_to_source: {}/{} x {}/{}\nformat: {:?}\ncompression: {}\nmedia_type: {}\nimage_base64_bytes: {}\ncaptured_at: {}\n",
        output.target.kind(),
        output.target.id(),
        output.adapter.platform,
        output.adapter.session,
        output.original_bounds.width,
        output.original_bounds.height,
        output.output_bounds.width,
        output.output_bounds.height,
        output.resized,
        output.source_to_output.x_numerator,
        output.source_to_output.x_denominator,
        output.source_to_output.y_numerator,
        output.source_to_output.y_denominator,
        output.output_to_source.x_numerator,
        output.output_to_source.x_denominator,
        output.output_to_source.y_numerator,
        output.output_to_source.y_denominator,
        output.format,
        output.compression,
        output.media_type,
        output.image_base64.len(),
        output.captured_at,
    )
}

pub(crate) fn current_timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

pub(crate) fn unique_temp_path(extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    let extension = extension.trim_start_matches('.');
    std::env::temp_dir().join(format!("tendril-capture-{nanos}.{extension}"))
}

pub(crate) fn read_and_remove_temp_capture(path: &PathBuf) -> Result<Vec<u8>, TendrilError> {
    let bytes = fs::read(path).map_err(|error| {
        TendrilError::execution_failure(
            "capture_read_failed",
            format!(
                "failed to read capture artifact `{}`: {error}",
                path.display()
            ),
            None,
        )
    })?;
    let _ = fs::remove_file(path);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::{DynamicImage, ImageBuffer, Rgba};
    use proptest::prelude::*;

    use super::{build_capture_output, current_timestamp, resized_dimensions};
    use crate::config::ImageFormat;
    use crate::model::{CaptureInput, TargetSelector};
    use crate::platform::{
        AdapterContext, AdapterInfo, CaptureArtifact, CaptureTargetKind, TargetDescriptor,
    };

    #[test]
    fn resized_dimensions_preserve_aspect_ratio() {
        assert_eq!(
            resized_dimensions(1920, 1080, Some(1000), Some(1000)),
            (1000, 563)
        );
        assert_eq!(
            resized_dimensions(1920, 1080, Some(3840), None),
            (1920, 1080)
        );
        assert_eq!(resized_dimensions(1600, 1200, None, Some(600)), (800, 600));
    }

    #[test]
    fn capture_output_contains_inverse_mapping_metadata_after_resize() {
        let artifact = CaptureArtifact {
            target_id: "window-1".to_owned(),
            media_type: "image/png".to_owned(),
            image_bytes: sample_png(400, 200),
            captured_at: current_timestamp(),
        };
        let target = TargetDescriptor {
            id: "window-1".to_owned(),
            title: Some("Example".to_owned()),
            kind: CaptureTargetKind::Window,
            name: "Example".to_owned(),
            bounds: crate::model::Bounds {
                x: 10,
                y: 20,
                width: 400,
                height: 200,
            },
            scale_factor: crate::model::ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: Some("app".to_owned()),
            process_id: Some(1),
        };
        let input = CaptureInput {
            target: TargetSelector::Window {
                id: "window-1".to_owned(),
            },
            max_width: Some(100),
            max_height: None,
            format: ImageFormat::Png,
            compression: 85,
        };

        let output = build_capture_output(
            &input,
            &target,
            artifact,
            &AdapterInfo::from_context(&AdapterContext::windows11()),
        )
        .expect("capture output should build");

        assert!(output.resized);
        assert_eq!(output.original_bounds.width, 400);
        assert_eq!(output.output_bounds.width, 100);
        assert_eq!(output.output_bounds.height, 50);
        assert_eq!(output.source_to_output.x_numerator, 100);
        assert_eq!(output.source_to_output.x_denominator, 400);
        assert_eq!(output.output_to_source.x_numerator, 400);
        assert_eq!(output.output_to_source.x_denominator, 100);
        assert_eq!(output.media_type, "image/png");
        assert!(!output.image_base64.is_empty());
        assert!(output.captured_at.contains('T'));
    }

    #[test]
    fn capture_output_keeps_identity_mapping_without_resize() {
        let artifact = CaptureArtifact {
            target_id: "1".to_owned(),
            media_type: "image/png".to_owned(),
            image_bytes: sample_png(64, 32),
            captured_at: current_timestamp(),
        };
        let target = TargetDescriptor {
            id: "1".to_owned(),
            title: None,
            kind: CaptureTargetKind::Display,
            name: "Display".to_owned(),
            bounds: crate::model::Bounds {
                x: 0,
                y: 0,
                width: 64,
                height: 32,
            },
            scale_factor: crate::model::ScaleFactor::identity(),
            capture_supported: true,
            input_supported: true,
            app_name: None,
            process_id: None,
        };
        let input = CaptureInput {
            target: TargetSelector::Display {
                id: "1".to_owned(),
            },
            max_width: None,
            max_height: None,
            format: ImageFormat::Jpeg,
            compression: 80,
        };

        let output = build_capture_output(
            &input,
            &target,
            artifact,
            &AdapterInfo::from_context(&AdapterContext::windows11()),
        )
        .expect("capture output should build");

        assert!(!output.resized);
        assert_eq!(output.original_bounds.width, output.output_bounds.width);
        assert_eq!(output.output_to_source.x_numerator, 64);
        assert_eq!(output.output_to_source.x_denominator, 64);
        assert_eq!(output.media_type, "image/jpeg");
    }

    proptest! {
        #[test]
        fn resized_dimensions_never_exceed_requested_constraints(
            width in 1u32..5000,
            height in 1u32..5000,
            max_width in proptest::option::of(1u32..5000),
            max_height in proptest::option::of(1u32..5000),
        ) {
            let (resized_width, resized_height) = resized_dimensions(width, height, max_width, max_height);

            prop_assert!(resized_width >= 1);
            prop_assert!(resized_height >= 1);
            prop_assert!(resized_width <= width);
            prop_assert!(resized_height <= height);
            if let Some(limit) = max_width {
                prop_assert!(resized_width <= limit.max(1).min(width));
            }
            if let Some(limit) = max_height {
                prop_assert!(resized_height <= limit.max(1).min(height));
            }
        }
    }

    fn sample_png(width: u32, height: u32) -> Vec<u8> {
        let image = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(
            width,
            height,
            Rgba([32, 64, 96, 255]),
        ));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .expect("sample png should encode");
        bytes
    }
}
