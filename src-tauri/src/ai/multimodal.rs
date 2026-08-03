use std::{collections::BTreeSet, fmt::Write};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    domain::{
        ImageLimits, ImageMime, StructuredPageRequest, UnifiedVisionRequest, VisionAsset,
        VisionAssetMeta,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

pub fn stage_vision_asset(
    book_id: Uuid,
    capture_id: Uuid,
    declared_mime: ImageMime,
    declared_width: u32,
    declared_height: u32,
    bytes: Vec<u8>,
    limits: ImageLimits,
) -> AppResult<VisionAsset> {
    let encoded_bytes = u64::try_from(bytes.len()).map_err(|_| invalid_input())?;
    if encoded_bytes == 0 || encoded_bytes > limits.max_encoded_bytes_each {
        return Err(invalid_input());
    }

    let (detected_mime, width, height) = detect_image(&bytes).ok_or_else(invalid_input)?;
    if detected_mime != declared_mime
        || width == 0
        || height == 0
        || width != declared_width
        || height != declared_height
    {
        return Err(invalid_input());
    }
    validate_dimensions(width, height, limits)?;

    let meta = VisionAssetMeta {
        book_id,
        capture_id,
        mime_type: detected_mime,
        width,
        height,
        encoded_bytes,
        sha256: sha256_hex(&bytes),
    };
    Ok(VisionAsset::from_validated(meta, bytes))
}

pub fn validate_vision_request(
    expected_book_id: Uuid,
    request: &UnifiedVisionRequest,
    limits: ImageLimits,
) -> AppResult<()> {
    validate_assets(expected_book_id, &request.images, limits)
}

pub fn validate_structured_page_request(
    expected_book_id: Uuid,
    request: &StructuredPageRequest,
    limits: ImageLimits,
) -> AppResult<()> {
    validate_assets(expected_book_id, &request.pages, limits)
}

fn validate_assets(
    expected_book_id: Uuid,
    assets: &[VisionAsset],
    limits: ImageLimits,
) -> AppResult<()> {
    if assets.is_empty() || assets.len() > usize::from(limits.max_images) {
        return Err(invalid_input());
    }

    let mut total_encoded_bytes = 0_u64;
    let mut capture_ids = BTreeSet::new();
    let mut content_hashes = BTreeSet::new();
    for asset in assets {
        let actual_bytes = u64::try_from(asset.bytes().len()).map_err(|_| invalid_input())?;
        total_encoded_bytes = total_encoded_bytes
            .checked_add(actual_bytes)
            .ok_or_else(invalid_input)?;
        if asset.meta.book_id != expected_book_id
            || asset.meta.encoded_bytes != actual_bytes
            || actual_bytes == 0
            || actual_bytes > limits.max_encoded_bytes_each
            || !capture_ids.insert(asset.meta.capture_id)
            || !content_hashes.insert(asset.meta.sha256.as_str())
            || asset.meta.sha256 != sha256_hex(asset.bytes())
        {
            return Err(invalid_input());
        }

        let (mime_type, width, height) = detect_image(asset.bytes()).ok_or_else(invalid_input)?;
        if mime_type != asset.meta.mime_type
            || width != asset.meta.width
            || height != asset.meta.height
        {
            return Err(invalid_input());
        }
        validate_dimensions(width, height, limits)?;
    }

    if total_encoded_bytes > limits.max_total_encoded_bytes {
        return Err(invalid_input());
    }
    Ok(())
}

fn validate_dimensions(width: u32, height: u32, limits: ImageLimits) -> AppResult<()> {
    let decoded_pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(invalid_input)?;
    if width == 0
        || height == 0
        || width > limits.max_dimension_px
        || height > limits.max_dimension_px
        || decoded_pixels > limits.max_decoded_pixels_each
    {
        return Err(invalid_input());
    }
    Ok(())
}

fn detect_image(bytes: &[u8]) -> Option<(ImageMime, u32, u32)> {
    detect_png(bytes)
        .or_else(|| detect_jpeg(bytes))
        .or_else(|| detect_webp(bytes))
}

fn detect_png(bytes: &[u8]) -> Option<(ImageMime, u32, u32)> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24
        || &bytes[..8] != SIGNATURE
        || u32::from_be_bytes(bytes[8..12].try_into().ok()?) != 13
        || &bytes[12..16] != b"IHDR"
    {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((ImageMime::Png, width, height))
}

fn detect_jpeg(bytes: &[u8]) -> Option<(ImageMime, u32, u32)> {
    if bytes.len() < 4 || bytes[..2] != [0xff, 0xd8] {
        return None;
    }

    let mut cursor = 2_usize;
    while cursor < bytes.len() {
        if bytes[cursor] != 0xff {
            return None;
        }
        while cursor < bytes.len() && bytes[cursor] == 0xff {
            cursor += 1;
        }
        let marker = *bytes.get(cursor)?;
        cursor += 1;

        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            continue;
        }

        let segment_length = usize::from(u16::from_be_bytes([
            *bytes.get(cursor)?,
            *bytes.get(cursor + 1)?,
        ]));
        if segment_length < 2 || cursor.checked_add(segment_length)? > bytes.len() {
            return None;
        }
        if is_start_of_frame(marker) {
            if segment_length < 7 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([
                *bytes.get(cursor + 3)?,
                *bytes.get(cursor + 4)?,
            ]));
            let width = u32::from(u16::from_be_bytes([
                *bytes.get(cursor + 5)?,
                *bytes.get(cursor + 6)?,
            ]));
            return Some((ImageMime::Jpeg, width, height));
        }
        cursor += segment_length;
    }
    None
}

fn is_start_of_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

fn detect_webp(bytes: &[u8]) -> Option<(ImageMime, u32, u32)> {
    if bytes.len() < 21 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let chunk_length = usize::try_from(u32::from_le_bytes(bytes[16..20].try_into().ok()?)).ok()?;
    if 20_usize.checked_add(chunk_length)? > bytes.len() {
        return None;
    }

    let (width, height) = match &bytes[12..16] {
        b"VP8X" if chunk_length >= 10 => (
            read_le_u24(bytes.get(24..27)?)?.checked_add(1)?,
            read_le_u24(bytes.get(27..30)?)?.checked_add(1)?,
        ),
        b"VP8 " if chunk_length >= 10 && bytes.get(23..26)? == [0x9d, 0x01, 0x2a] => (
            u32::from(u16::from_le_bytes(bytes.get(26..28)?.try_into().ok()?)) & 0x3fff,
            u32::from(u16::from_le_bytes(bytes.get(28..30)?.try_into().ok()?)) & 0x3fff,
        ),
        b"VP8L" if chunk_length >= 5 && bytes[20] == 0x2f => {
            let packed = u32::from_le_bytes(bytes.get(21..25)?.try_into().ok()?);
            ((packed & 0x3fff) + 1, ((packed >> 14) & 0x3fff) + 1)
        }
        _ => return None,
    };
    Some((ImageMime::Webp, width, height))
}

fn read_le_u24(bytes: &[u8]) -> Option<u32> {
    Some(
        u32::from(bytes.first().copied()?)
            | (u32::from(*bytes.get(1)?) << 8)
            | (u32::from(*bytes.get(2)?) << 16),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}
