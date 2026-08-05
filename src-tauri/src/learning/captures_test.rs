use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{domain::ImageLimits, errors::AppErrorCode};

use super::captures::{
    ExpectedRegionCapture, OwnedCaptureBytes, REGION_CAPTURE_SCHEMA_VERSION, RegionCaptureLimits,
    RegionCaptureMetadata, validate_and_stage_region_capture,
};

#[test]
fn captures_validate_all_bindings_dimensions_bytes_and_hashes() {
    let ids = CaptureIds::new();
    let bytes = png(2, 2, b"bounded synthetic pixels");
    let capture_hash = sha256_hex(&bytes);
    let anchor_hash = "a".repeat(64);
    let staged = validate_and_stage_region_capture(
        expected(&ids, &anchor_hash),
        &metadata(&ids, &anchor_hash, &capture_hash, bytes.len()),
        OwnedCaptureBytes::new(bytes.clone()),
    )
    .expect("valid bounded capture");
    assert_eq!(staged.mime_type(), "image/png");
    assert_eq!((staged.width(), staged.height()), (2, 2));
    assert_eq!(staged.decoded_pixel_count(), 4);
    assert_eq!(staged.bytes(), bytes);

    let cases = [
        CaptureMutation::Preparation,
        CaptureMutation::Operation,
        CaptureMutation::Book,
        CaptureMutation::Profile,
        CaptureMutation::Model,
        CaptureMutation::AnchorHash,
        CaptureMutation::CaptureHash,
        CaptureMutation::Schema,
        CaptureMutation::Mime,
        CaptureMutation::Width,
        CaptureMutation::Height,
        CaptureMutation::Pixels,
        CaptureMutation::Length,
    ];
    for mutation in cases {
        let mut candidate = metadata(&ids, &anchor_hash, &capture_hash, bytes.len());
        mutation.apply(&mut candidate);
        let error = validate_and_stage_region_capture(
            expected(&ids, &anchor_hash),
            &candidate,
            OwnedCaptureBytes::new(bytes.clone()),
        )
        .expect_err("mutated capture must fail closed");
        assert_eq!(error.code, AppErrorCode::InvalidInput, "{mutation:?}");
    }

    for invalid_bytes in [
        b"not a PNG".to_vec(),
        png(3, 2, b"declared dimensions do not match"),
    ] {
        let invalid_hash = sha256_hex(&invalid_bytes);
        let error = validate_and_stage_region_capture(
            expected(&ids, &anchor_hash),
            &metadata(&ids, &anchor_hash, &invalid_hash, invalid_bytes.len()),
            OwnedCaptureBytes::new(invalid_bytes),
        )
        .expect_err("capture bytes must have a matching PNG header");
        assert_eq!(error.code, AppErrorCode::InvalidInput);
    }
}

#[test]
fn captures_zeroize_owned_bytes_on_error_and_drop() {
    let ids = CaptureIds::new();
    let bytes = png(2, 2, b"ZEROIZE_CAPTURE_SENTINEL");
    let anchor_hash = "b".repeat(64);
    let invalid_hash = "c".repeat(64);
    let error_probe = Arc::new(AtomicBool::new(false));
    let error = validate_and_stage_region_capture(
        expected(&ids, &anchor_hash),
        &metadata(&ids, &anchor_hash, &invalid_hash, bytes.len()),
        OwnedCaptureBytes::with_zeroize_probe(bytes.clone(), error_probe.clone()),
    )
    .expect_err("hash mismatch");
    assert_eq!(error.code, AppErrorCode::InvalidInput);
    assert!(error_probe.load(Ordering::SeqCst));

    let drop_probe = Arc::new(AtomicBool::new(false));
    let staged = validate_and_stage_region_capture(
        expected(&ids, &anchor_hash),
        &metadata(&ids, &anchor_hash, &sha256_hex(&bytes), bytes.len()),
        OwnedCaptureBytes::with_zeroize_probe(bytes, drop_probe.clone()),
    )
    .expect("valid capture");
    assert!(!drop_probe.load(Ordering::SeqCst));
    drop(staged);
    assert!(drop_probe.load(Ordering::SeqCst));
}

#[test]
fn captures_debug_and_deserialization_never_emit_sensitive_values() {
    let ids = CaptureIds::new();
    let anchor_hash = "d".repeat(64);
    let capture_hash = "e".repeat(64);
    let metadata = metadata(&ids, &anchor_hash, &capture_hash, 4);
    let debug = format!("{metadata:?}");
    let sentinels = [
        ids.model.clone(),
        anchor_hash.clone(),
        capture_hash.clone(),
        ids.preparation.to_string(),
        ids.operation.to_string(),
        ids.profile.to_string(),
    ];
    for sentinel in sentinels {
        assert!(!debug.contains(&sentinel));
    }

    let unsafe_json = serde_json::json!({
        "preparationId": ids.preparation,
        "operationToken": ids.operation,
        "bookId": ids.book,
        "providerProfileId": ids.profile,
        "modelId": ids.model,
        "anchorContentSha256": anchor_hash,
        "captureSha256": capture_hash,
        "schemaVersion": 1,
        "mimeType": "image/png",
        "width": 1,
        "height": 1,
        "decodedPixelCount": 1,
        "encodedByteLength": 4,
        "unexpectedBody": "TEXTBOOK_SENTINEL"
    });
    assert!(serde_json::from_value::<RegionCaptureMetadata>(unsafe_json).is_err());
}

#[derive(Debug)]
enum CaptureMutation {
    Preparation,
    Operation,
    Book,
    Profile,
    Model,
    AnchorHash,
    CaptureHash,
    Schema,
    Mime,
    Width,
    Height,
    Pixels,
    Length,
}

impl CaptureMutation {
    fn apply(&self, metadata: &mut RegionCaptureMetadata) {
        match self {
            Self::Preparation => metadata.preparation_id = Uuid::new_v4(),
            Self::Operation => metadata.operation_token = Uuid::new_v4(),
            Self::Book => metadata.book_id = Uuid::new_v4(),
            Self::Profile => metadata.provider_profile_id = Uuid::new_v4(),
            Self::Model => metadata.model_id = "substituted-model".to_owned(),
            Self::AnchorHash => metadata.anchor_content_sha256 = "f".repeat(64),
            Self::CaptureHash => metadata.capture_sha256 = "0".repeat(64),
            Self::Schema => metadata.schema_version = 2,
            Self::Mime => metadata.mime_type = "image/jpeg".to_owned(),
            Self::Width => metadata.width = 0,
            Self::Height => metadata.height = 4_097,
            Self::Pixels => metadata.decoded_pixel_count = 5,
            Self::Length => metadata.encoded_byte_length += 1,
        }
    }
}

struct CaptureIds {
    preparation: Uuid,
    operation: Uuid,
    book: Uuid,
    profile: Uuid,
    model: String,
}

impl CaptureIds {
    fn new() -> Self {
        Self {
            preparation: Uuid::new_v4(),
            operation: Uuid::new_v4(),
            book: Uuid::new_v4(),
            profile: Uuid::new_v4(),
            model: "verified-synthetic-model".to_owned(),
        }
    }
}

fn expected<'a>(ids: &'a CaptureIds, anchor_hash: &'a str) -> ExpectedRegionCapture<'a> {
    ExpectedRegionCapture {
        preparation_id: ids.preparation,
        operation_token: ids.operation,
        book_id: ids.book,
        provider_profile_id: ids.profile,
        model_id: &ids.model,
        anchor_content_sha256: anchor_hash,
        limits: RegionCaptureLimits::from_provider(ImageLimits {
            max_images: 1,
            max_encoded_bytes_each: 1024,
            max_total_encoded_bytes: 1024,
            max_dimension_px: 32,
            max_decoded_pixels_each: 1024,
        })
        .expect("test limits"),
    }
}

fn metadata(
    ids: &CaptureIds,
    anchor_hash: &str,
    capture_hash: &str,
    byte_length: usize,
) -> RegionCaptureMetadata {
    RegionCaptureMetadata {
        preparation_id: ids.preparation,
        operation_token: ids.operation,
        book_id: ids.book,
        provider_profile_id: ids.profile,
        model_id: ids.model.clone(),
        anchor_content_sha256: anchor_hash.to_owned(),
        capture_sha256: capture_hash.to_owned(),
        schema_version: REGION_CAPTURE_SCHEMA_VERSION,
        mime_type: "image/png".to_owned(),
        width: 2,
        height: 2,
        decoded_pixel_count: 4,
        encoded_byte_length: u64::try_from(byte_length).unwrap(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|value| format!("{value:02x}"))
        .collect()
}

fn png(width: u32, height: u32, suffix: &[u8]) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(suffix);
    bytes
}
