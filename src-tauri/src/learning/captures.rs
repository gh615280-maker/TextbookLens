use std::fmt;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    domain::ImageLimits,
    errors::{AppError, AppErrorCode, AppResult},
};

pub const REGION_CAPTURE_SCHEMA_VERSION: u16 = 1;
const APPLICATION_MAX_ENCODED_BYTES: u64 = 4 * 1024 * 1024;
const APPLICATION_MAX_DIMENSION: u32 = 4_096;
const APPLICATION_MAX_DECODED_PIXELS: u64 = 8_847_360;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionCaptureLimits {
    pub max_encoded_bytes: u64,
    pub max_dimension: u32,
    pub max_decoded_pixels: u64,
}

impl RegionCaptureLimits {
    pub fn from_provider(limits: ImageLimits) -> AppResult<Self> {
        if limits.max_images == 0
            || limits.max_encoded_bytes_each == 0
            || limits.max_total_encoded_bytes == 0
            || limits.max_dimension_px == 0
            || limits.max_decoded_pixels_each == 0
        {
            return Err(AppError::unsupported_provider_capability());
        }
        Ok(Self {
            max_encoded_bytes: limits
                .max_encoded_bytes_each
                .min(limits.max_total_encoded_bytes)
                .min(APPLICATION_MAX_ENCODED_BYTES),
            max_dimension: limits.max_dimension_px.min(APPLICATION_MAX_DIMENSION),
            max_decoded_pixels: limits
                .max_decoded_pixels_each
                .min(APPLICATION_MAX_DECODED_PIXELS),
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegionCaptureMetadata {
    pub preparation_id: Uuid,
    pub operation_token: Uuid,
    pub book_id: Uuid,
    pub provider_profile_id: Uuid,
    pub model_id: String,
    pub anchor_content_sha256: String,
    pub capture_sha256: String,
    pub schema_version: u16,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub decoded_pixel_count: u64,
    pub encoded_byte_length: u64,
}

impl fmt::Debug for RegionCaptureMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegionCaptureMetadata")
            .field("preparation_id", &"<redacted>")
            .field("operation_token", &"<redacted>")
            .field("book_id", &"<redacted>")
            .field("provider_profile_id", &"<redacted>")
            .field("model_id", &"<redacted>")
            .field("anchor_content_sha256", &"<redacted>")
            .field("capture_sha256", &"<redacted>")
            .field("schema_version", &self.schema_version)
            .field("mime_type", &self.mime_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("decoded_pixel_count", &self.decoded_pixel_count)
            .field("encoded_byte_length", &self.encoded_byte_length)
            .finish()
    }
}

impl Drop for RegionCaptureMetadata {
    fn drop(&mut self) {
        self.model_id.zeroize();
        self.anchor_content_sha256.zeroize();
        self.capture_sha256.zeroize();
    }
}

pub(crate) struct ExpectedRegionCapture<'a> {
    pub preparation_id: Uuid,
    pub operation_token: Uuid,
    pub book_id: Uuid,
    pub provider_profile_id: Uuid,
    pub model_id: &'a str,
    pub anchor_content_sha256: &'a str,
    pub limits: RegionCaptureLimits,
}

impl fmt::Debug for ExpectedRegionCapture<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExpectedRegionCapture")
            .field("preparation_id", &"<redacted>")
            .field("operation_token", &"<redacted>")
            .field("book_id", &"<redacted>")
            .field("provider_profile_id", &"<redacted>")
            .field("model_id", &"<redacted>")
            .field("anchor_content_sha256", &"<redacted>")
            .field("limits", &self.limits)
            .finish()
    }
}

pub(crate) struct OwnedCaptureBytes {
    bytes: Zeroizing<Vec<u8>>,
    #[cfg(test)]
    zeroize_probe: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl OwnedCaptureBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
            #[cfg(test)]
            zeroize_probe: None,
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    #[cfg(test)]
    pub(crate) fn with_zeroize_probe(
        bytes: Vec<u8>,
        probe: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            bytes: Zeroizing::new(bytes),
            zeroize_probe: Some(probe),
        }
    }
}

impl fmt::Debug for OwnedCaptureBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedCaptureBytes")
            .field("byte_length", &self.bytes.len())
            .finish()
    }
}

impl Drop for OwnedCaptureBytes {
    fn drop(&mut self) {
        self.bytes.zeroize();
        #[cfg(test)]
        if let Some(probe) = &self.zeroize_probe {
            use std::sync::atomic::Ordering;
            probe.store(self.bytes.iter().all(|value| *value == 0), Ordering::SeqCst);
        }
    }
}

pub struct StagedRegionCapture {
    mime_type: &'static str,
    width: u32,
    height: u32,
    decoded_pixel_count: u64,
    bytes: OwnedCaptureBytes,
}

impl StagedRegionCapture {
    pub fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn decoded_pixel_count(&self) -> u64 {
        self.decoded_pixel_count
    }

    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

impl fmt::Debug for StagedRegionCapture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedRegionCapture")
            .field("mime_type", &self.mime_type)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("decoded_pixel_count", &self.decoded_pixel_count)
            .field("byte_length", &self.bytes.len())
            .finish()
    }
}

pub(crate) fn validate_and_stage_region_capture(
    expected: ExpectedRegionCapture<'_>,
    metadata: &RegionCaptureMetadata,
    bytes: OwnedCaptureBytes,
) -> AppResult<StagedRegionCapture> {
    let encoded_length = u64::try_from(bytes.len()).map_err(|_| invalid_input())?;
    let decoded_pixels = u64::from(metadata.width)
        .checked_mul(u64::from(metadata.height))
        .ok_or_else(invalid_input)?;
    let detected_dimensions = png_dimensions(bytes.as_slice());
    if metadata.preparation_id != expected.preparation_id
        || metadata.operation_token != expected.operation_token
        || metadata.book_id != expected.book_id
        || metadata.provider_profile_id != expected.provider_profile_id
        || metadata.model_id != expected.model_id
        || metadata.anchor_content_sha256 != expected.anchor_content_sha256
        || metadata.schema_version != REGION_CAPTURE_SCHEMA_VERSION
        || metadata.mime_type != "image/png"
        || detected_dimensions != Some((metadata.width, metadata.height))
        || metadata.width == 0
        || metadata.height == 0
        || metadata.width > expected.limits.max_dimension
        || metadata.height > expected.limits.max_dimension
        || decoded_pixels != metadata.decoded_pixel_count
        || decoded_pixels > expected.limits.max_decoded_pixels
        || encoded_length == 0
        || encoded_length != metadata.encoded_byte_length
        || encoded_length > expected.limits.max_encoded_bytes
        || !valid_sha256(&metadata.anchor_content_sha256)
        || !valid_sha256(&metadata.capture_sha256)
        || !sha256_matches(bytes.as_slice(), &metadata.capture_sha256)
    {
        return Err(invalid_input());
    }

    Ok(StagedRegionCapture {
        mime_type: "image/png",
        width: metadata.width,
        height: metadata.height,
        decoded_pixel_count: metadata.decoded_pixel_count,
        bytes,
    })
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24
        || &bytes[..8] != SIGNATURE
        || u32::from_be_bytes(bytes[8..12].try_into().ok()?) != 13
        || &bytes[12..16] != b"IHDR"
    {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_matches(bytes: &[u8], expected_hex: &str) -> bool {
    let Some(mut expected) = decode_sha256(expected_hex) else {
        return false;
    };
    let mut actual: [u8; 32] = Sha256::digest(bytes).into();
    let difference = actual
        .iter()
        .zip(expected.iter())
        .fold(0_u8, |accumulator, (left, right)| {
            accumulator | (left ^ right)
        });
    actual.zeroize();
    expected.zeroize();
    difference == 0
}

fn decode_sha256(value: &str) -> Option<[u8; 32]> {
    if !valid_sha256(value) {
        return None;
    }
    let mut result = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        result[index] = (decode_hex_nibble(chunk[0])? << 4) | decode_hex_nibble(chunk[1])?;
    }
    Some(result)
}

const fn decode_hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}
