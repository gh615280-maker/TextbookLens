use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "document.ts")]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormalizedRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Result<Self, &'static str> {
        let values = [x, y, width, height];
        if values.iter().any(|value| !(0.0..=1.0).contains(value)) {
            return Err("normalized rectangle values must be in [0, 1]");
        }
        if x + width > 1.0 || y + height > 1.0 {
            return Err("normalized rectangle must remain in bounds");
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(
    tag = "format",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "document.ts")]
pub enum DocumentLocator {
    Pdf {
        start_page: u32,
        end_page: u32,
        rects_by_page: Option<BTreeMap<u32, Vec<NormalizedRect>>>,
    },
    Epub {
        cfi: String,
        section_id: Uuid,
    },
    Docx {
        start_block_id: Uuid,
        start_offset: u32,
        end_block_id: Uuid,
        end_offset: u32,
    },
}

#[derive(Deserialize)]
#[serde(
    tag = "format",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum SerializedDocumentLocator {
    Pdf {
        start_page: u32,
        end_page: u32,
        rects_by_page: Option<BTreeMap<String, Vec<NormalizedRect>>>,
    },
    Epub {
        cfi: String,
        section_id: Uuid,
    },
    Docx {
        start_block_id: Uuid,
        start_offset: u32,
        end_block_id: Uuid,
        end_offset: u32,
    },
}

impl<'de> Deserialize<'de> for DocumentLocator {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match SerializedDocumentLocator::deserialize(deserializer)? {
            SerializedDocumentLocator::Pdf {
                start_page,
                end_page,
                rects_by_page,
            } => Ok(Self::Pdf {
                start_page,
                end_page,
                rects_by_page: rects_by_page
                    .map(|pages| {
                        pages
                            .into_iter()
                            .map(|(page, rects)| {
                                page.parse::<u32>()
                                    .map(|page| (page, rects))
                                    .map_err(serde::de::Error::custom)
                            })
                            .collect()
                    })
                    .transpose()?,
            }),
            SerializedDocumentLocator::Epub { cfi, section_id } => {
                Ok(Self::Epub { cfi, section_id })
            }
            SerializedDocumentLocator::Docx {
                start_block_id,
                start_offset,
                end_block_id,
                end_offset,
            } => Ok(Self::Docx {
                start_block_id,
                start_offset,
                end_block_id,
                end_offset,
            }),
        }
    }
}

impl DocumentLocator {
    pub fn pdf(
        start_page: u32,
        end_page: u32,
        rects_by_page: Option<BTreeMap<u32, Vec<NormalizedRect>>>,
    ) -> Result<Self, &'static str> {
        if start_page == 0 || end_page == 0 || start_page > end_page {
            return Err("PDF pages must be one-based and ordered");
        }
        if let Some(rects) = &rects_by_page
            && rects
                .keys()
                .any(|page| *page == 0 || *page < start_page || *page > end_page)
        {
            return Err("rectangle pages must be within the PDF locator range");
        }
        Ok(Self::Pdf {
            start_page,
            end_page,
            rects_by_page,
        })
    }

    pub fn epub(cfi: String, section_id: Uuid) -> Result<Self, &'static str> {
        if cfi.trim().is_empty() {
            return Err("EPUB CFI must not be empty");
        }
        Ok(Self::Epub { cfi, section_id })
    }

    pub fn docx(
        start_block_id: Uuid,
        start_offset: u32,
        end_block_id: Uuid,
        end_offset: u32,
    ) -> Result<Self, &'static str> {
        if start_offset > end_offset {
            return Err("DOCX offsets must be ordered");
        }
        Ok(Self::Docx {
            start_block_id,
            start_offset,
            end_block_id,
            end_offset,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "document.ts")]
pub struct TextQuote {
    pub exact: String,
    pub prefix: String,
    pub suffix: String,
}

impl TextQuote {
    pub fn new(exact: String, prefix: String, suffix: String) -> Result<Self, &'static str> {
        if exact.is_empty() {
            return Err("exact text quote must not be empty");
        }
        Ok(Self {
            exact,
            prefix,
            suffix,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "document.ts")]
pub struct SelectionAnchor {
    pub locator: DocumentLocator,
    pub quote: TextQuote,
    pub section_id: Option<Uuid>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "format",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "document.ts")]
pub enum RegionLocator {
    Pdf { page: u32 },
    Epub { section_id: Uuid, cfi: String },
    Docx { block_id: Uuid },
}

impl RegionLocator {
    pub fn pdf(page: u32) -> Result<Self, &'static str> {
        if page == 0 {
            return Err("PDF region page must be one-based");
        }
        Ok(Self::Pdf { page })
    }

    pub fn epub(section_id: Uuid, cfi: String) -> Result<Self, &'static str> {
        if cfi.trim().is_empty() {
            return Err("EPUB region CFI or element locator must not be empty");
        }
        Ok(Self::Epub { section_id, cfi })
    }

    pub fn docx(block_id: Uuid) -> Self {
        Self::Docx { block_id }
    }

    fn is_valid(&self) -> bool {
        match self {
            Self::Pdf { page } => *page > 0,
            Self::Epub { cfi, .. } => !cfi.trim().is_empty(),
            Self::Docx { .. } => true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "document.ts")]
pub struct RegionAnchor {
    pub locator: RegionLocator,
    pub rect: NormalizedRect,
    pub content_sha256: String,
    pub text_fallback: Option<TextQuote>,
}

impl RegionAnchor {
    pub const MAX_FALLBACK_EXACT_CHARS: usize = 4096;
    pub const MAX_FALLBACK_CONTEXT_CHARS: usize = 64;

    pub fn new(
        locator: RegionLocator,
        rect: NormalizedRect,
        content_sha256: String,
        text_fallback: Option<TextQuote>,
    ) -> Result<Self, &'static str> {
        if !locator.is_valid() {
            return Err("region locator is invalid");
        }
        let values = [rect.x, rect.y, rect.width, rect.height];
        if values.iter().any(|value| !value.is_finite())
            || rect.x < 0.0
            || rect.y < 0.0
            || rect.width <= 0.0
            || rect.height <= 0.0
            || rect.x + rect.width > 1.0
            || rect.y + rect.height > 1.0
        {
            return Err("region rectangle must be finite, normalized, and positive");
        }
        if content_sha256.len() != 64
            || !content_sha256
                .bytes()
                .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
        {
            return Err("region content hash must be a lowercase SHA-256 hex digest");
        }
        if let Some(fallback) = &text_fallback
            && (fallback.exact.is_empty()
                || fallback.exact.chars().count() > Self::MAX_FALLBACK_EXACT_CHARS
                || fallback.prefix.chars().count() > Self::MAX_FALLBACK_CONTEXT_CHARS
                || fallback.suffix.chars().count() > Self::MAX_FALLBACK_CONTEXT_CHARS)
        {
            return Err("region text fallback is invalid or exceeds its bounds");
        }
        Ok(Self {
            locator,
            rect,
            content_sha256,
            text_fallback,
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SerializedRegionAnchor {
    locator: RegionLocator,
    rect: NormalizedRect,
    content_sha256: String,
    text_fallback: Option<TextQuote>,
}

impl<'de> Deserialize<'de> for RegionAnchor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = SerializedRegionAnchor::deserialize(deserializer)?;
        Self::new(
            value.locator,
            value.rect,
            value.content_sha256,
            value.text_fallback,
        )
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export_to = "document.ts")]
pub enum ContentAnchor {
    Text { selection: SelectionAnchor },
    Region { region: RegionAnchor },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TaggedContentAnchor {
    Text { selection: SelectionAnchor },
    Region { region: RegionAnchor },
}

impl<'de> Deserialize<'de> for ContentAnchor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value
            .as_object()
            .is_some_and(|object| object.contains_key("kind"))
        {
            return Ok(
                match serde_json::from_value::<TaggedContentAnchor>(value)
                    .map_err(serde::de::Error::custom)?
                {
                    TaggedContentAnchor::Text { selection } => Self::Text { selection },
                    TaggedContentAnchor::Region { region } => Self::Region { region },
                },
            );
        }
        serde_json::from_value::<SelectionAnchor>(value)
            .map(|selection| Self::Text { selection })
            .map_err(serde::de::Error::custom)
    }
}

impl From<SelectionAnchor> for ContentAnchor {
    fn from(selection: SelectionAnchor) -> Self {
        Self::Text { selection }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "document.ts")]
pub enum BlockKind {
    Heading,
    Paragraph,
    List,
    Table,
    Caption,
    Equation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "document.ts")]
pub struct NormalizedBookInput {
    pub title: String,
    pub author: Option<String>,
    pub language: Option<String>,
    pub sections: Vec<NormalizedSectionInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "document.ts")]
pub struct NormalizedSectionInput {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub ordinal: u32,
    pub title: String,
    pub locator: DocumentLocator,
    pub blocks: Vec<NormalizedBlockInput>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "document.ts")]
pub struct NormalizedBlockInput {
    pub id: Uuid,
    pub ordinal: u32,
    pub kind: BlockKind,
    pub plain_text: String,
    pub locator: DocumentLocator,
}

pub fn stable_section_id(book_id: Uuid, ordinal: u32) -> Uuid {
    Uuid::new_v5(&book_id, format!("section:{ordinal}").as_bytes())
}

pub fn stable_block_id(book_id: Uuid, section_ordinal: u32, block_ordinal: u32) -> Uuid {
    Uuid::new_v5(
        &book_id,
        format!("block:{section_ordinal}:{block_ordinal}").as_bytes(),
    )
}
