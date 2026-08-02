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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
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
#[serde(rename_all = "snake_case")]
#[ts(export_to = "document.ts")]
pub enum BlockKind {
    Heading,
    Paragraph,
    ListItem,
    Table,
    FigureCaption,
    Code,
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
