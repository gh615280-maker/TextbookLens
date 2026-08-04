use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::{
    ai::structured::PAGE_ANALYSIS_SCHEMA_VERSION,
    domain::{
        ContentSource, IndexPageBlockKind, IndexReviewReason, IndexTableCellDto, NormalizedRect,
        PageAnalysisBlockKind, ProviderPageAnalysis, UntrustedPageAnalysis, UntrustedPageBlock,
        UntrustedTableCell,
    },
};

pub const MAX_VALIDATION_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_BLOCKS_PER_PAGE: usize = 2_000;
const MAX_PLAIN_TEXT_CODE_POINTS: usize = 65_536;
const MAX_PLAIN_TEXT_BYTES: usize = MAX_PLAIN_TEXT_CODE_POINTS * 4;
const MAX_LATEX_CODE_POINTS: usize = 16_384;
const MAX_LATEX_BYTES: usize = MAX_LATEX_CODE_POINTS * 4;
const MAX_DESCRIPTION_CODE_POINTS: usize = 32_768;
const MAX_DESCRIPTION_BYTES: usize = MAX_DESCRIPTION_CODE_POINTS * 4;
const MAX_TABLE_CELLS: usize = 4_096;
const MAX_TABLE_AXIS: u32 = 512;
const MAX_CELL_CODE_POINTS: usize = 16_384;
const MAX_CELL_BYTES: usize = MAX_CELL_CODE_POINTS * 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestedPageValidation {
    pub page_number: u32,
    pub local_text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BatchValidationFailure {
    InvalidSchemaVersion,
    InvalidResponseSize,
    EmptyRequest,
    DuplicateRequestedPage,
    MissingOrUnexpectedPage,
    DuplicateResponsePage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageValidationFailure {
    NoVisibleContent,
    TooManyBlocks,
    InvalidOrdinal,
    InvalidText,
    InvalidLatex,
    InvalidTable,
    InvalidBounds,
    InvalidKindContent,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ValidatedPage {
    pub page_number: u32,
    pub blocks: Vec<ValidatedBlock>,
    pub review_reason: Option<IndexReviewReason>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ValidatedBlock {
    pub ordinal: u32,
    pub kind: IndexPageBlockKind,
    pub plain_text: Option<String>,
    pub latex: Option<String>,
    pub table_cells: Option<Vec<IndexTableCellDto>>,
    pub visual_description: Option<String>,
    pub bounds: Option<NormalizedRect>,
    pub source: ContentSource,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageValidationResult {
    pub page_number: u32,
    pub result: Result<ValidatedPage, PageValidationFailure>,
}

pub fn validate_batch(
    analysis: &ProviderPageAnalysis,
    response_bytes: usize,
    requested_pages: &[RequestedPageValidation],
) -> Result<Vec<PageValidationResult>, BatchValidationFailure> {
    if analysis.schema_version != PAGE_ANALYSIS_SCHEMA_VERSION {
        return Err(BatchValidationFailure::InvalidSchemaVersion);
    }
    if response_bytes == 0 || response_bytes > MAX_VALIDATION_RESPONSE_BYTES {
        return Err(BatchValidationFailure::InvalidResponseSize);
    }
    if requested_pages.is_empty() {
        return Err(BatchValidationFailure::EmptyRequest);
    }

    let mut requested = BTreeMap::new();
    for page in requested_pages {
        if page.page_number == 0
            || requested
                .insert(page.page_number, page.local_text.as_deref())
                .is_some()
        {
            return Err(BatchValidationFailure::DuplicateRequestedPage);
        }
    }
    if analysis.pages.len() != requested.len() {
        return Err(BatchValidationFailure::MissingOrUnexpectedPage);
    }

    let mut response = BTreeMap::new();
    for page in &analysis.pages {
        if response.insert(page.page_number, page).is_some() {
            return Err(BatchValidationFailure::DuplicateResponsePage);
        }
    }
    if response.keys().copied().collect::<BTreeSet<_>>()
        != requested.keys().copied().collect::<BTreeSet<_>>()
    {
        return Err(BatchValidationFailure::MissingOrUnexpectedPage);
    }

    Ok(requested
        .into_iter()
        .map(|(page_number, local_text)| PageValidationResult {
            page_number,
            result: validate_page(response[&page_number], local_text),
        })
        .collect())
}

fn validate_page(
    page: &UntrustedPageAnalysis,
    local_text: Option<&str>,
) -> Result<ValidatedPage, PageValidationFailure> {
    if page.blocks.is_empty() {
        return Err(PageValidationFailure::NoVisibleContent);
    }
    if page.blocks.len() > MAX_BLOCKS_PER_PAGE {
        return Err(PageValidationFailure::TooManyBlocks);
    }

    let mut blocks = Vec::with_capacity(page.blocks.len());
    let mut malformed_table = false;
    let mut malformed_latex = false;
    for (expected_ordinal, block) in page.blocks.iter().enumerate() {
        if usize::try_from(block.ordinal).ok() != Some(expected_ordinal) {
            return Err(PageValidationFailure::InvalidOrdinal);
        }
        let (validated, table_issue, latex_issue) = validate_block(block)?;
        malformed_table |= table_issue;
        malformed_latex |= latex_issue;
        blocks.push(validated);
    }

    let review_reason = if malformed_table {
        Some(IndexReviewReason::MalformedTable)
    } else if malformed_latex {
        Some(IndexReviewReason::MalformedLatex)
    } else if has_severe_overlap(&blocks) {
        Some(IndexReviewReason::SevereOverlap)
    } else if has_severe_repetition(&blocks) || appears_truncated(&blocks) {
        Some(IndexReviewReason::IncompleteContent)
    } else if local_text.is_some_and(|text| contradicts_local_text(&blocks, text)) {
        Some(IndexReviewReason::TextContradiction)
    } else {
        None
    };

    Ok(ValidatedPage {
        page_number: page.page_number,
        blocks,
        review_reason,
    })
}

fn validate_block(
    block: &UntrustedPageBlock,
) -> Result<(ValidatedBlock, bool, bool), PageValidationFailure> {
    let kind = match block.kind {
        PageAnalysisBlockKind::Title => IndexPageBlockKind::Title,
        PageAnalysisBlockKind::Paragraph => IndexPageBlockKind::Paragraph,
        PageAnalysisBlockKind::List => IndexPageBlockKind::List,
        PageAnalysisBlockKind::Table => IndexPageBlockKind::Table,
        PageAnalysisBlockKind::Caption => IndexPageBlockKind::Caption,
        PageAnalysisBlockKind::Formula => IndexPageBlockKind::Formula,
        PageAnalysisBlockKind::Figure => IndexPageBlockKind::Figure,
        PageAnalysisBlockKind::Transcript => IndexPageBlockKind::Transcript,
    };
    let plain_text = optional_bounded_text(
        &block.plain_text,
        MAX_PLAIN_TEXT_CODE_POINTS,
        MAX_PLAIN_TEXT_BYTES,
    )?;
    let visual_description = block
        .visual_description
        .as_deref()
        .map(|value| {
            required_bounded_text(value, MAX_DESCRIPTION_CODE_POINTS, MAX_DESCRIPTION_BYTES)
        })
        .transpose()?;
    let latex = block
        .latex
        .as_deref()
        .map(required_bounded_latex)
        .transpose()?;
    let latex_issue = latex.as_deref().is_some_and(latex_is_malformed);
    let (table_cells, table_issue) = validate_table_cells(block.table_cells.as_deref())?;

    if kind != IndexPageBlockKind::Table && table_cells.is_some() {
        return Err(PageValidationFailure::InvalidKindContent);
    }
    if kind == IndexPageBlockKind::Table && plain_text.is_none() && table_cells.is_none() {
        return Err(PageValidationFailure::InvalidKindContent);
    }
    if kind == IndexPageBlockKind::Figure && plain_text.is_none() && visual_description.is_none() {
        return Err(PageValidationFailure::InvalidKindContent);
    }
    if plain_text.is_none()
        && latex.is_none()
        && table_cells.is_none()
        && visual_description.is_none()
    {
        return Err(PageValidationFailure::NoVisibleContent);
    }

    let bounds = block
        .bounds
        .map(|bounds| {
            if !bounds.x.is_finite()
                || !bounds.y.is_finite()
                || !bounds.width.is_finite()
                || !bounds.height.is_finite()
                || bounds.width <= 0.0
                || bounds.height <= 0.0
            {
                return Err(PageValidationFailure::InvalidBounds);
            }
            NormalizedRect::new(bounds.x, bounds.y, bounds.width, bounds.height)
                .map_err(|_| PageValidationFailure::InvalidBounds)
        })
        .transpose()?;
    let source = if plain_text.is_none() && latex.is_none() && table_cells.is_none() {
        ContentSource::AiDescription
    } else {
        ContentSource::AiTranscribed
    };

    Ok((
        ValidatedBlock {
            ordinal: block.ordinal,
            kind,
            plain_text,
            latex,
            table_cells,
            visual_description,
            bounds,
            source,
        },
        table_issue,
        latex_issue,
    ))
}

fn optional_bounded_text(
    value: &str,
    max_code_points: usize,
    max_bytes: usize,
) -> Result<Option<String>, PageValidationFailure> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    required_bounded_text(value, max_code_points, max_bytes).map(Some)
}

fn required_bounded_text(
    value: &str,
    max_code_points: usize,
    max_bytes: usize,
) -> Result<String, PageValidationFailure> {
    let code_points = value.chars().count();
    if value.trim().is_empty()
        || value.len() > max_bytes
        || code_points > max_code_points
        || value.contains(['\0', '\r'])
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(PageValidationFailure::InvalidText);
    }
    let suspicious = value
        .chars()
        .filter(|character| *character == '\u{fffd}')
        .count();
    if code_points >= 20 && suspicious.saturating_mul(20) > code_points {
        return Err(PageValidationFailure::InvalidText);
    }
    Ok(value.to_owned())
}

fn required_bounded_latex(value: &str) -> Result<String, PageValidationFailure> {
    if value.trim().is_empty()
        || value.len() > MAX_LATEX_BYTES
        || value.chars().count() > MAX_LATEX_CODE_POINTS
        || value.contains(['\0', '\r'])
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Err(PageValidationFailure::InvalidLatex);
    }
    Ok(value.to_owned())
}

fn latex_is_malformed(value: &str) -> bool {
    let mut depth = 0_i32;
    let mut escaped = false;
    let mut dollar_count = 0_u32;
    for character in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        match character {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            '$' => dollar_count += 1,
            _ => {}
        }
    }
    escaped || depth != 0 || !dollar_count.is_multiple_of(2)
}

fn validate_table_cells(
    cells: Option<&[UntrustedTableCell]>,
) -> Result<(Option<Vec<IndexTableCellDto>>, bool), PageValidationFailure> {
    let Some(cells) = cells else {
        return Ok((None, false));
    };
    if cells.is_empty() || cells.len() > MAX_TABLE_CELLS {
        return Err(PageValidationFailure::InvalidTable);
    }

    let mut occupied = HashSet::new();
    let mut malformed = false;
    let mut result = Vec::with_capacity(cells.len());
    for cell in cells {
        let text = required_bounded_text(cell.text.as_str(), MAX_CELL_CODE_POINTS, MAX_CELL_BYTES)
            .map_err(|_| PageValidationFailure::InvalidTable)?;
        let row_end = cell.row.checked_add(cell.row_span);
        let column_end = cell.column.checked_add(cell.column_span);
        if cell.row_span == 0
            || cell.column_span == 0
            || row_end.is_none_or(|end| end > MAX_TABLE_AXIS)
            || column_end.is_none_or(|end| end > MAX_TABLE_AXIS)
        {
            return Err(PageValidationFailure::InvalidTable);
        }
        for row in cell.row..row_end.expect("checked above") {
            for column in cell.column..column_end.expect("checked above") {
                if !occupied.insert((row, column)) {
                    malformed = true;
                }
            }
        }
        result.push(IndexTableCellDto {
            row: cell.row,
            column: cell.column,
            row_span: cell.row_span,
            column_span: cell.column_span,
            text,
        });
    }
    let row_zero = result.iter().any(|cell| cell.row == 0);
    let column_zero = result.iter().any(|cell| cell.column == 0);
    malformed |= !row_zero || !column_zero;
    Ok((Some(result), malformed))
}

fn has_severe_overlap(blocks: &[ValidatedBlock]) -> bool {
    for (index, left) in blocks.iter().enumerate() {
        let Some(left_bounds) = &left.bounds else {
            continue;
        };
        for right in &blocks[index + 1..] {
            let Some(right_bounds) = &right.bounds else {
                continue;
            };
            let left_area = left_bounds.width * left_bounds.height;
            let right_area = right_bounds.width * right_bounds.height;
            let intersection_width = (left_bounds.x + left_bounds.width)
                .min(right_bounds.x + right_bounds.width)
                - left_bounds.x.max(right_bounds.x);
            let intersection_height = (left_bounds.y + left_bounds.height)
                .min(right_bounds.y + right_bounds.height)
                - left_bounds.y.max(right_bounds.y);
            if intersection_width <= 0.0 || intersection_height <= 0.0 {
                continue;
            }
            let ratio = intersection_width * intersection_height / left_area.min(right_area);
            if ratio >= 0.85 {
                return true;
            }
        }
    }
    false
}

fn has_severe_repetition(blocks: &[ValidatedBlock]) -> bool {
    let mut occurrences = HashMap::<String, usize>::new();
    for text in blocks
        .iter()
        .filter_map(|block| block.plain_text.as_deref())
    {
        let normalized = normalize_for_comparison(text);
        if normalized.chars().count() >= 12 {
            let count = occurrences.entry(normalized).or_default();
            *count += 1;
            if *count >= 3 {
                return true;
            }
        }
    }
    false
}

fn appears_truncated(blocks: &[ValidatedBlock]) -> bool {
    blocks
        .last()
        .and_then(|block| block.plain_text.as_deref())
        .is_some_and(|text| {
            let trimmed = text.trim_end();
            trimmed.ends_with("<truncated>") || trimmed.ends_with('\u{fffd}')
        })
}

fn contradicts_local_text(blocks: &[ValidatedBlock], local_text: &str) -> bool {
    let local = normalize_for_comparison(local_text);
    let candidate = normalize_for_comparison(
        &blocks
            .iter()
            .filter_map(|block| block.plain_text.as_deref())
            .collect::<Vec<_>>()
            .join(" "),
    );
    if local.chars().count() < 32 || candidate.chars().count() < 32 {
        return false;
    }
    let local_set = local.chars().collect::<HashSet<_>>();
    let candidate_set = candidate.chars().collect::<HashSet<_>>();
    let intersection = local_set.intersection(&candidate_set).count();
    let smaller = local_set.len().min(candidate_set.len());
    smaller > 0 && intersection.saturating_mul(5) < smaller
}

fn normalize_for_comparison(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
