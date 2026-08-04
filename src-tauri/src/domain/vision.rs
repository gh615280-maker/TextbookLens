use std::fmt;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{ProviderKind, UnifiedChatRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "vision.ts")]
pub enum ImageMime {
    Png,
    Jpeg,
    Webp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "vision.ts")]
pub struct VisionAssetMeta {
    pub book_id: Uuid,
    pub capture_id: Uuid,
    pub mime_type: ImageMime,
    pub width: u32,
    pub height: u32,
    pub encoded_bytes: u64,
    pub sha256: String,
}

pub struct VisionAsset {
    pub meta: VisionAssetMeta,
    bytes: Zeroizing<Vec<u8>>,
}

impl VisionAsset {
    pub(crate) fn from_validated(meta: VisionAssetMeta, bytes: Vec<u8>) -> Self {
        Self {
            meta,
            bytes: Zeroizing::new(bytes),
        }
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

impl fmt::Debug for VisionAsset {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VisionAsset")
            .field("meta", &self.meta)
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

pub struct UnifiedVisionRequest {
    pub text: UnifiedChatRequest,
    pub images: Vec<VisionAsset>,
}

impl fmt::Debug for UnifiedVisionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnifiedVisionRequest")
            .field("text", &"[REDACTED]")
            .field("images", &self.images)
            .finish()
    }
}

pub struct StructuredPageRequest {
    pub model: String,
    pub pages: Vec<VisionAsset>,
    pub schema_version: String,
    pub max_output_bytes: u32,
}

impl fmt::Debug for StructuredPageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StructuredPageRequest")
            .field("model", &"[REDACTED]")
            .field("page_count", &self.pages.len())
            .field("schema_version", &self.schema_version)
            .field("max_output_bytes", &self.max_output_bytes)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "vision.ts")]
pub struct ProviderPageAnalysis {
    pub schema_version: String,
    pub pages: Vec<UntrustedPageAnalysis>,
}

/// Rust-only result of a structured page operation. Provider content and an
/// optional remote identifier never cross serialization or Debug boundaries.
pub struct StructuredAnalysisOutcome {
    pub analysis: ProviderPageAnalysis,
    pub cleanup: Option<RemoteCleanupHandle>,
}

impl StructuredAnalysisOutcome {
    pub fn inline(analysis: ProviderPageAnalysis) -> Self {
        Self {
            analysis,
            cleanup: None,
        }
    }
}

impl fmt::Debug for StructuredAnalysisOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StructuredAnalysisOutcome")
            .field("analysis", &"[REDACTED]")
            .field("has_cleanup", &self.cleanup.is_some())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "vision.ts")]
pub struct UntrustedPageAnalysis {
    pub page_number: u32,
    pub blocks: Vec<UntrustedPageBlock>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "vision.ts")]
pub struct UntrustedPageBlock {
    pub ordinal: u32,
    pub kind: PageAnalysisBlockKind,
    pub plain_text: String,
    pub bounds: Option<UntrustedNormalizedRect>,
    pub latex: Option<String>,
    pub table_cells: Option<Vec<UntrustedTableCell>>,
    pub visual_description: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "vision.ts")]
pub enum PageAnalysisBlockKind {
    Title,
    Paragraph,
    List,
    Table,
    Caption,
    Formula,
    Figure,
    Transcript,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "vision.ts")]
pub struct UntrustedNormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export_to = "vision.ts")]
pub struct UntrustedTableCell {
    pub row: u32,
    pub column: u32,
    pub row_span: u32,
    pub column_span: u32,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "vision.ts")]
pub enum RemoteCleanupStatus {
    NotRequired,
    Pending,
    Succeeded,
    Failed,
}

pub struct RemoteCleanupHandle {
    provider: ProviderKind,
    opaque_id: SecretString,
}

impl RemoteCleanupHandle {
    pub fn new(provider: ProviderKind, opaque_id: SecretString) -> Self {
        Self {
            provider,
            opaque_id,
        }
    }

    pub fn provider(&self) -> &ProviderKind {
        &self.provider
    }

    pub fn opaque_id(&self) -> &SecretString {
        &self.opaque_id
    }
}

impl fmt::Debug for RemoteCleanupHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RemoteCleanupHandle")
            .field("provider", &self.provider)
            .field("opaque_id", &"[REDACTED]")
            .finish()
    }
}
