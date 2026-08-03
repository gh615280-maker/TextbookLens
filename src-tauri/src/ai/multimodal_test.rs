use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use secrecy::SecretString;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{
    error::AiError,
    multimodal::{stage_vision_asset, validate_vision_request},
    provider::{AiProvider, ProviderStream},
};
use crate::domain::{
    ImageLimits, ImageMime, ProviderKind, RemoteCleanupHandle, StructuredPageRequest,
    UnifiedChatRequest, UnifiedVisionRequest, ValidationResult,
};

#[test]
fn image_staging_accepts_only_matching_bounded_png_jpeg_and_webp_headers() {
    let book_id = Uuid::new_v4();
    let limits = limits(4);
    let cases = [
        (ImageMime::Png, png(640, 480, b"png"), 640, 480),
        (ImageMime::Jpeg, jpeg(320, 240), 320, 240),
        (ImageMime::Webp, webp(1920, 1080), 1920, 1080),
    ];

    for (mime_type, bytes, width, height) in cases {
        let asset = stage_vision_asset(
            book_id,
            Uuid::new_v4(),
            mime_type,
            width,
            height,
            bytes,
            limits,
        )
        .unwrap();
        assert_eq!(asset.meta.mime_type, mime_type);
        assert_eq!(asset.meta.width, width);
        assert_eq!(asset.meta.height, height);
        assert_eq!(asset.meta.sha256.len(), 64);
    }
}

#[test]
fn image_staging_rejects_mime_spoof_magic_mismatch_and_declared_dimension_mismatch() {
    let book_id = Uuid::new_v4();
    let capture_id = Uuid::new_v4();
    let limits = limits(4);

    assert!(
        stage_vision_asset(
            book_id,
            capture_id,
            ImageMime::Jpeg,
            32,
            16,
            png(32, 16, b""),
            limits,
        )
        .is_err()
    );
    assert!(
        stage_vision_asset(
            book_id,
            capture_id,
            ImageMime::Png,
            32,
            16,
            b"not-an-image".to_vec(),
            limits,
        )
        .is_err()
    );
    assert!(
        stage_vision_asset(
            book_id,
            capture_id,
            ImageMime::Png,
            31,
            16,
            png(32, 16, b""),
            limits,
        )
        .is_err()
    );
}

#[test]
fn image_staging_rejects_oversize_pixel_bomb_and_zero_dimensions() {
    let book_id = Uuid::new_v4();
    let bytes = png(64, 64, &[0; 64]);
    let mut size_limits = limits(4);
    size_limits.max_encoded_bytes_each = u64::try_from(bytes.len() - 1).unwrap();
    assert!(
        stage_vision_asset(
            book_id,
            Uuid::new_v4(),
            ImageMime::Png,
            64,
            64,
            bytes,
            size_limits,
        )
        .is_err()
    );

    assert!(
        stage_vision_asset(
            book_id,
            Uuid::new_v4(),
            ImageMime::Png,
            4_096,
            4_096,
            png(4_096, 4_096, b""),
            limits(4),
        )
        .is_err()
    );
    assert!(
        stage_vision_asset(
            book_id,
            Uuid::new_v4(),
            ImageMime::Png,
            0,
            1,
            png(0, 1, b""),
            limits(4),
        )
        .is_err()
    );
}

#[test]
fn request_validation_rejects_duplicate_capture_hash_wrong_book_and_count() {
    let expected_book = Uuid::new_v4();
    let other_book = Uuid::new_v4();
    let duplicate_capture = Uuid::new_v4();
    let limits = limits(2);

    let request = vision_request(vec![
        asset(
            expected_book,
            duplicate_capture,
            ImageMime::Png,
            png(10, 10, b"a"),
            10,
            10,
            limits,
        ),
        asset(
            expected_book,
            duplicate_capture,
            ImageMime::Jpeg,
            jpeg(10, 10),
            10,
            10,
            limits,
        ),
    ]);
    assert!(validate_vision_request(expected_book, &request, limits).is_err());

    let same_bytes = png(10, 10, b"same");
    let request = vision_request(vec![
        asset(
            expected_book,
            Uuid::new_v4(),
            ImageMime::Png,
            same_bytes.clone(),
            10,
            10,
            limits,
        ),
        asset(
            expected_book,
            Uuid::new_v4(),
            ImageMime::Png,
            same_bytes,
            10,
            10,
            limits,
        ),
    ]);
    assert!(validate_vision_request(expected_book, &request, limits).is_err());

    let request = vision_request(vec![asset(
        other_book,
        Uuid::new_v4(),
        ImageMime::Png,
        png(10, 10, b"other-book"),
        10,
        10,
        limits,
    )]);
    assert!(validate_vision_request(expected_book, &request, limits).is_err());

    let request = vision_request(
        (0_u8..3)
            .map(|suffix| {
                asset(
                    expected_book,
                    Uuid::new_v4(),
                    ImageMime::Png,
                    png(10, 10, &[suffix]),
                    10,
                    10,
                    limits,
                )
            })
            .collect(),
    );
    assert!(validate_vision_request(expected_book, &request, limits).is_err());
}

#[tokio::test]
async fn text_only_provider_defaults_reject_visual_operations_without_network_or_echoes() {
    let provider = TextOnlyProvider::default();
    let book_id = Uuid::new_v4();
    let limits = limits(2);
    let credential_sentinel = "fixture-credential-must-not-escape";
    let model_sentinel = "fixture-model-must-not-escape";

    let vision_result = provider
        .stream_vision(
            &SecretString::from(credential_sentinel),
            UnifiedVisionRequest {
                text: text_request(model_sentinel),
                images: vec![asset(
                    book_id,
                    Uuid::new_v4(),
                    ImageMime::Png,
                    png(10, 10, b"vision"),
                    10,
                    10,
                    limits,
                )],
            },
            CancellationToken::new(),
        )
        .await;
    let vision_error = match vision_result {
        Err(error) => error,
        Ok(_) => panic!("text-only provider unexpectedly accepted vision"),
    };
    assert_eq!(
        vision_error.stable_code(),
        "UNSUPPORTED_PROVIDER_CAPABILITY"
    );

    let structured_error = provider
        .analyze_pages(
            &SecretString::from(credential_sentinel),
            StructuredPageRequest {
                model: model_sentinel.to_owned(),
                pages: vec![asset(
                    book_id,
                    Uuid::new_v4(),
                    ImageMime::Png,
                    png(10, 10, b"structured"),
                    10,
                    10,
                    limits,
                )],
                schema_version: "textbooklens.page-analysis.v1".to_owned(),
                max_output_bytes: 4_096,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        structured_error.stable_code(),
        "UNSUPPORTED_PROVIDER_CAPABILITY"
    );
    let diagnostics = format!("{vision_error:?} {structured_error:?}");
    assert!(!diagnostics.contains(credential_sentinel));
    assert!(!diagnostics.contains(model_sentinel));
    assert_eq!(provider.network_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn rust_only_debug_boundaries_redact_image_bytes_and_cleanup_ids() {
    let raw_sentinel = b"raw-image-sentinel";
    let cleanup_sentinel = "opaque-cleanup-sentinel";
    let asset = asset(
        Uuid::new_v4(),
        Uuid::new_v4(),
        ImageMime::Png,
        png(10, 10, raw_sentinel),
        10,
        10,
        limits(1),
    );
    let handle =
        RemoteCleanupHandle::new(ProviderKind::OpenAi, SecretString::from(cleanup_sentinel));

    assert!(!format!("{asset:?}").contains(std::str::from_utf8(raw_sentinel).unwrap()));
    assert!(!format!("{handle:?}").contains(cleanup_sentinel));
}

#[derive(Default)]
struct TextOnlyProvider {
    network_calls: AtomicUsize,
}

#[async_trait]
impl AiProvider for TextOnlyProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::OpenAi
    }

    async fn validate(
        &self,
        _credential: &SecretString,
        _model: &str,
    ) -> Result<ValidationResult, AiError> {
        self.network_calls.fetch_add(1, Ordering::SeqCst);
        Err(AiError::provider_unavailable())
    }

    async fn stream_chat(
        &self,
        _credential: &SecretString,
        _request: UnifiedChatRequest,
        _cancel: CancellationToken,
    ) -> Result<ProviderStream, AiError> {
        self.network_calls.fetch_add(1, Ordering::SeqCst);
        Err(AiError::provider_unavailable())
    }
}

fn limits(max_images: u16) -> ImageLimits {
    ImageLimits {
        max_images,
        max_encoded_bytes_each: 1_024,
        max_total_encoded_bytes: 4_096,
        max_dimension_px: 4_096,
        max_decoded_pixels_each: 8_847_360,
    }
}

fn asset(
    book_id: Uuid,
    capture_id: Uuid,
    mime_type: ImageMime,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    limits: ImageLimits,
) -> crate::domain::VisionAsset {
    stage_vision_asset(book_id, capture_id, mime_type, width, height, bytes, limits).unwrap()
}

fn vision_request(images: Vec<crate::domain::VisionAsset>) -> UnifiedVisionRequest {
    UnifiedVisionRequest {
        text: text_request("gpt-5.6"),
        images,
    }
}

fn text_request(model: &str) -> UnifiedChatRequest {
    UnifiedChatRequest {
        model: model.to_owned(),
        system: "system".to_owned(),
        messages: Vec::new(),
        max_output_tokens: 4_096,
        expected_language: None,
    }
}

fn png(width: u32, height: u32, suffix: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(suffix);
    bytes
}

fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x07, 0x08];
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes
}

fn webp(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0_u8; 30];
    bytes[..4].copy_from_slice(b"RIFF");
    bytes[4..8].copy_from_slice(&22_u32.to_le_bytes());
    bytes[8..12].copy_from_slice(b"WEBP");
    bytes[12..16].copy_from_slice(b"VP8X");
    bytes[16..20].copy_from_slice(&10_u32.to_le_bytes());
    write_le_u24(&mut bytes[24..27], width - 1);
    write_le_u24(&mut bytes[27..30], height - 1);
    bytes
}

fn write_le_u24(target: &mut [u8], value: u32) {
    target[0] = value as u8;
    target[1] = (value >> 8) as u8;
    target[2] = (value >> 16) as u8;
}
