use std::{cmp::Ordering, collections::BTreeSet, fmt};

use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;

use crate::{
    domain::{
        Citation, CitationReviewStatus, ContentAnchor, ContentSource, DocumentLocator,
        RegionLocator,
    },
    errors::{AppError, AppErrorCode, AppResult},
    learning::{ContextSource, TypedContextSegment},
};

use super::{
    budget::InputBudget,
    citations::{CitationSeed, assign_request_local_citations},
    search::{SearchHit, search_book},
};

pub const HISTORY_PLACEHOLDER: &str = "[Earlier same-conversation history omitted; no generated summary is available in this request.]";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextSourceKind {
    Selection,
    Region,
    Neighbor,
    Heading,
    Definition,
    TextbookSearch,
    UserNote,
    HistorySummary,
    Directory,
}

impl ContextSourceKind {
    pub const fn is_mandatory(self) -> bool {
        matches!(self, Self::Selection | Self::Region)
    }

    const fn packing_order(self) -> u8 {
        match self {
            Self::Selection | Self::Region => 0,
            Self::Neighbor => 1,
            Self::Heading | Self::Definition => 2,
            Self::TextbookSearch => 3,
            Self::UserNote => 4,
            Self::HistorySummary => 5,
            Self::Directory => 6,
        }
    }
}

#[derive(Clone, PartialEq)]
pub struct ContextCandidate {
    pub stable_id: String,
    pub book_id: Uuid,
    pub section_id: Option<Uuid>,
    pub source_kind: ContextSourceKind,
    pub source: ContextSource,
    pub same_section: bool,
    pub relevance_micros: u32,
    pub ordinal: u32,
    pub text: String,
    pub locator_label: String,
    pub locator: Option<DocumentLocator>,
    pub review_status: CitationReviewStatus,
    pub provenance_key: String,
    pub citation_seed: Option<CitationSeed>,
}

impl ContextCandidate {
    pub fn history_placeholder(book_id: Uuid, section_id: Option<Uuid>) -> Self {
        Self {
            stable_id: "same-conversation-history-placeholder".to_owned(),
            book_id,
            section_id,
            source_kind: ContextSourceKind::HistorySummary,
            source: ContextSource::HistorySummary,
            same_section: true,
            relevance_micros: 0,
            ordinal: u32::MAX,
            text: HISTORY_PLACEHOLDER.to_owned(),
            locator_label: "same conversation history placeholder".to_owned(),
            locator: None,
            review_status: CitationReviewStatus::NotRequired,
            provenance_key: "same-conversation-history-placeholder".to_owned(),
            citation_seed: None,
        }
    }
}

impl fmt::Debug for ContextCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextCandidate")
            .field("stable_id", &"<redacted>")
            .field("book_id", &"<redacted>")
            .field("section_id", &self.section_id.map(|_| "<redacted>"))
            .field("source_kind", &self.source_kind)
            .field("source", &self.source)
            .field("same_section", &self.same_section)
            .field("relevance_micros", &self.relevance_micros)
            .field("ordinal", &self.ordinal)
            .field(
                "text",
                &format_args!("<redacted:{} scalars>", self.text.chars().count()),
            )
            .field("locator_label", &self.locator_label)
            .field("locator", &self.locator.as_ref().map(|_| "<redacted>"))
            .field("review_status", &self.review_status)
            .field("provenance_key", &"<redacted>")
            .field("citation_seed", &self.citation_seed)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct ContextSegment {
    pub stable_id: String,
    pub book_id: Uuid,
    pub source_kind: ContextSourceKind,
    pub source: ContextSource,
    pub locator_label: String,
    pub review_status: CitationReviewStatus,
    pub content: String,
    pub(crate) citation_seed: Option<CitationSeed>,
    pub citation: Option<Citation>,
}

impl ContextSegment {
    pub fn to_typed(&self) -> TypedContextSegment {
        TypedContextSegment {
            book_id: self.book_id,
            source: self.source,
            locator: self.locator_label.clone(),
            review_status: self.review_status,
            citation: self.citation.clone(),
            content: self.content.clone(),
        }
    }
}

impl fmt::Debug for ContextSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContextSegment")
            .field("stable_id", &"<redacted>")
            .field("book_id", &"<redacted>")
            .field("source_kind", &self.source_kind)
            .field("source", &self.source)
            .field("locator_label", &self.locator_label)
            .field("review_status", &self.review_status)
            .field(
                "content",
                &format_args!("<redacted:{} scalars>", self.content.chars().count()),
            )
            .field("citation", &self.citation)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct PackedContext {
    pub segments: Vec<ContextSegment>,
    pub citations: Vec<Citation>,
    pub estimated_input_tokens: u32,
    pub omitted_segment_count: u32,
}

impl PackedContext {
    pub fn typed_segments(&self) -> Vec<TypedContextSegment> {
        self.segments.iter().map(ContextSegment::to_typed).collect()
    }
}

impl fmt::Debug for PackedContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PackedContext")
            .field("segment_count", &self.segments.len())
            .field("citation_count", &self.citations.len())
            .field("estimated_input_tokens", &self.estimated_input_tokens)
            .field("omitted_segment_count", &self.omitted_segment_count)
            .finish()
    }
}

pub fn pack_context<F>(
    book_id: Uuid,
    budget: InputBudget,
    candidates: Vec<ContextCandidate>,
    mut estimate_complete_request: F,
) -> AppResult<PackedContext>
where
    F: FnMut(&[ContextSegment]) -> AppResult<u64>,
{
    validate_candidate_values(book_id, &candidates)?;
    let mandatory_count = candidates
        .iter()
        .filter(|candidate| candidate.source_kind.is_mandatory())
        .count();
    if mandatory_count != 1 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let original_count = candidates.len();
    let ordered = deduplicate_candidates(candidates);
    let mandatory_count = ordered
        .iter()
        .take_while(|candidate| candidate.source_kind.is_mandatory())
        .count();
    if mandatory_count == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }

    let (mandatory_segments, _) = finalize_segments(&ordered[..mandatory_count])?;
    let mandatory_tokens = estimate_complete_request(&mandatory_segments)?;
    budget.require_fits(mandatory_tokens)?;

    for candidate_count in (mandatory_count..=ordered.len()).rev() {
        let (segments, citations) = finalize_segments(&ordered[..candidate_count])?;
        let estimated = estimate_complete_request(&segments)?;
        if let Ok(estimated_input_tokens) = budget.require_fits(estimated) {
            let omitted_segment_count =
                u32::try_from(original_count.saturating_sub(candidate_count))
                    .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?;
            return Ok(PackedContext {
                segments,
                citations,
                estimated_input_tokens,
                omitted_segment_count,
            });
        }
    }

    Err(AppError::new(AppErrorCode::ContextTooLarge))
}

/// Packs textbook-level context while keeping bounded directory metadata and an explicit
/// history-summary marker ahead of optional query-ranked retrieval snippets.
///
/// Unlike selection packing, textbook-level questions have no selected-text mandatory segment.
/// Directory and history-summary entries are all-or-error after their independent upstream caps;
/// optional retrieval entries are removed from lowest relevance first.
pub fn pack_book_context<F>(
    book_id: Uuid,
    budget: InputBudget,
    candidates: Vec<ContextCandidate>,
    mut estimate_complete_request: F,
) -> AppResult<PackedContext>
where
    F: FnMut(&[ContextSegment]) -> AppResult<u64>,
{
    validate_candidate_values(book_id, &candidates)?;
    if candidates.iter().any(|candidate| {
        candidate.source_kind.is_mandatory()
            || !matches!(
                candidate.source_kind,
                ContextSourceKind::TextbookSearch
                    | ContextSourceKind::HistorySummary
                    | ContextSourceKind::Directory
            )
    }) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }

    let original_count = candidates.len();
    let deduplicated = deduplicate_candidates(candidates);
    let mut required = deduplicated
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.source_kind,
                ContextSourceKind::Directory | ContextSourceKind::HistorySummary
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    required.sort_by(|left, right| {
        left.source_kind
            .packing_order()
            .cmp(&right.source_kind.packing_order())
            .then_with(|| left.ordinal.cmp(&right.ordinal))
            .then_with(|| left.stable_id.cmp(&right.stable_id))
    });
    let mut optional = deduplicated
        .into_iter()
        .filter(|candidate| candidate.source_kind == ContextSourceKind::TextbookSearch)
        .collect::<Vec<_>>();
    optional.sort_by(compare_candidates);

    let (required_segments, _) = finalize_segments(&required)?;
    budget.require_fits(estimate_complete_request(&required_segments)?)?;
    for optional_count in (0..=optional.len()).rev() {
        let mut retained = required.clone();
        retained.extend(optional[..optional_count].iter().cloned());
        let (segments, citations) = finalize_segments(&retained)?;
        let estimated = estimate_complete_request(&segments)?;
        if let Ok(estimated_input_tokens) = budget.require_fits(estimated) {
            return Ok(PackedContext {
                omitted_segment_count: u32::try_from(original_count.saturating_sub(retained.len()))
                    .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?,
                segments,
                citations,
                estimated_input_tokens,
            });
        }
    }
    Err(AppError::new(AppErrorCode::ContextTooLarge))
}

fn validate_candidate_values(book_id: Uuid, candidates: &[ContextCandidate]) -> AppResult<()> {
    for candidate in candidates {
        if candidate.book_id != book_id
            || candidate.stable_id.trim().is_empty()
            || candidate.text.is_empty()
            || candidate.locator_label.trim().is_empty()
            || candidate.provenance_key.trim().is_empty()
            || contains_disallowed_control(&candidate.text)
            || contains_disallowed_control(&candidate.locator_label)
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if candidate.source_kind == ContextSourceKind::HistorySummary
            && (candidate.source != ContextSource::HistorySummary
                || candidate.text != HISTORY_PLACEHOLDER)
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if candidate.source_kind == ContextSourceKind::Directory
            && (candidate.source != ContextSource::Directory
                || candidate.locator.is_some()
                || candidate.citation_seed.is_some())
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if candidate.source == ContextSource::Directory
            && candidate.source_kind != ContextSourceKind::Directory
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if matches!(
            candidate.source,
            ContextSource::UserNote
                | ContextSource::HistorySummary
                | ContextSource::AiDescription
                | ContextSource::Directory
        ) && candidate.citation_seed.is_some()
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        if let Some(seed) = &candidate.citation_seed
            && (seed.book_id != book_id
                || candidate.locator.as_ref() != Some(&seed.locator)
                || candidate.source.content_source() != Some(seed.source))
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
    }
    Ok(())
}

fn contains_disallowed_control(value: &str) -> bool {
    value
        .chars()
        .any(|code_point| code_point.is_control() && code_point != '\n' && code_point != '\t')
}

fn finalize_segments(
    candidates: &[ContextCandidate],
) -> AppResult<(Vec<ContextSegment>, Vec<Citation>)> {
    let mut segments = candidates
        .iter()
        .map(|candidate| ContextSegment {
            stable_id: candidate.stable_id.clone(),
            book_id: candidate.book_id,
            source_kind: candidate.source_kind,
            source: candidate.source,
            locator_label: candidate.locator_label.clone(),
            review_status: candidate.review_status,
            content: candidate.text.clone(),
            citation_seed: candidate.citation_seed.clone(),
            citation: None,
        })
        .collect::<Vec<_>>();
    let citations = assign_request_local_citations(&mut segments)?;
    Ok((segments, citations))
}

fn deduplicate_candidates(mut candidates: Vec<ContextCandidate>) -> Vec<ContextCandidate> {
    candidates.sort_by(compare_candidates);
    let mut retained = Vec::<ContextCandidate>::new();
    for candidate in candidates {
        let duplicate = retained.iter().any(|prior| {
            same_provenance_scope(prior, &candidate)
                && (prior.stable_id == candidate.stable_id
                    || overlap_percent(&candidate.text, &prior.text) >= 80)
        });
        if !duplicate {
            retained.push(candidate);
        }
    }
    retained
}

fn compare_candidates(left: &ContextCandidate, right: &ContextCandidate) -> Ordering {
    left.source_kind
        .is_mandatory()
        .cmp(&right.source_kind.is_mandatory())
        .reverse()
        .then_with(|| {
            left.source_kind
                .packing_order()
                .cmp(&right.source_kind.packing_order())
        })
        .then_with(|| right.same_section.cmp(&left.same_section))
        .then_with(|| right.relevance_micros.cmp(&left.relevance_micros))
        .then_with(|| left.ordinal.cmp(&right.ordinal))
        .then_with(|| source_order(left.source).cmp(&source_order(right.source)))
        .then_with(|| left.provenance_key.cmp(&right.provenance_key))
        .then_with(|| left.stable_id.cmp(&right.stable_id))
}

const fn source_order(source: ContextSource) -> u8 {
    match source {
        ContextSource::UserCorrected => 0,
        ContextSource::LocalText => 1,
        ContextSource::AiTranscribed => 2,
        ContextSource::AiDescription => 3,
        ContextSource::UserNote => 4,
        ContextSource::HistorySummary => 5,
        ContextSource::Directory => 6,
    }
}

fn same_provenance_scope(left: &ContextCandidate, right: &ContextCandidate) -> bool {
    left.source == right.source
        && left.provenance_key == right.provenance_key
        && match (&left.locator, &right.locator) {
            (Some(left), Some(right)) => locators_overlap(left, right),
            (None, None) => left.locator_label == right.locator_label,
            _ => false,
        }
}

fn locators_overlap(left: &DocumentLocator, right: &DocumentLocator) -> bool {
    match (left, right) {
        (
            DocumentLocator::Pdf {
                start_page: left_start,
                end_page: left_end,
                ..
            },
            DocumentLocator::Pdf {
                start_page: right_start,
                end_page: right_end,
                ..
            },
        ) => left_start <= right_end && right_start <= left_end,
        (
            DocumentLocator::Epub {
                cfi: left_cfi,
                section_id: left_section,
            },
            DocumentLocator::Epub {
                cfi: right_cfi,
                section_id: right_section,
            },
        ) => left_section == right_section && left_cfi == right_cfi,
        (
            DocumentLocator::Docx {
                start_block_id: left_start,
                end_block_id: left_end,
                ..
            },
            DocumentLocator::Docx {
                start_block_id: right_start,
                end_block_id: right_end,
                ..
            },
        ) => {
            left_start == right_start
                || left_start == right_end
                || left_end == right_start
                || left_end == right_end
        }
        _ => false,
    }
}

fn overlap_percent(candidate: &str, retained: &str) -> u32 {
    let candidate = canonical_comparison_text(candidate);
    let retained = canonical_comparison_text(retained);
    if candidate.is_empty() || retained.is_empty() {
        return 0;
    }
    if candidate == retained {
        return 100;
    }
    let candidate_shingles = scalar_shingles(&candidate);
    let retained_shingles = scalar_shingles(&retained);
    if candidate_shingles.is_empty() {
        return u32::from(retained.contains(&candidate)) * 100;
    }
    let overlap = candidate_shingles.intersection(&retained_shingles).count();
    u32::try_from(overlap.saturating_mul(100) / candidate_shingles.len()).unwrap_or(100)
}

fn canonical_comparison_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn scalar_shingles(value: &str) -> BTreeSet<Vec<char>> {
    const WIDTH: usize = 5;
    let scalars = value.chars().collect::<Vec<_>>();
    if scalars.len() < WIDTH {
        return BTreeSet::new();
    }
    scalars.windows(WIDTH).map(<[char]>::to_vec).collect()
}

pub struct SelectionContextQuery {
    pub book_id: Uuid,
    pub section_id: Uuid,
    pub anchor: ContentAnchor,
    pub selected_text: String,
    pub query_text: String,
}

impl fmt::Debug for SelectionContextQuery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectionContextQuery")
            .field("book_id", &"<redacted>")
            .field("section_id", &"<redacted>")
            .field("anchor", &"<redacted>")
            .field(
                "selected_text",
                &format_args!("<redacted:{} scalars>", self.selected_text.chars().count()),
            )
            .field(
                "query_text",
                &format_args!("<redacted:{} scalars>", self.query_text.chars().count()),
            )
            .finish()
    }
}

pub async fn retrieve_selection_context(
    pool: &SqlitePool,
    query: &SelectionContextQuery,
) -> AppResult<Vec<ContextCandidate>> {
    if query.selected_text.trim().is_empty()
        || query.query_text.chars().count() > 16_384
        || contains_disallowed_control(&query.selected_text)
        || contains_disallowed_control(&query.query_text)
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let book = sqlx::query("SELECT format, import_status FROM books WHERE id = ?")
        .bind(query.book_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if book.try_get::<String, _>("import_status")? != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let book_format: String = book.try_get("format")?;
    let section = sqlx::query(
        "SELECT id, parent_id, ordinal, title, locator_json FROM sections WHERE id = ? AND book_id = ?",
    )
    .bind(query.section_id.to_string())
    .bind(query.book_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::InvalidInput))?;

    let (active_locator, active_kind, center_ordinal) =
        validate_anchor_ownership(pool, query, &book_format).await?;
    let section_title: String = section.try_get("title")?;
    let active_label = locator_label(&active_locator, Some(&section_title));
    let active_seed = CitationSeed::new(
        query.book_id,
        Some(query.section_id),
        active_locator.clone(),
        active_label.clone(),
        ContentSource::LocalText,
        CitationReviewStatus::NotRequired,
    )?;
    let mut candidates = vec![ContextCandidate {
        stable_id: "active-content".to_owned(),
        book_id: query.book_id,
        section_id: Some(query.section_id),
        source_kind: active_kind,
        source: ContextSource::LocalText,
        same_section: true,
        relevance_micros: u32::MAX,
        ordinal: 0,
        text: query.selected_text.clone(),
        locator_label: active_label,
        locator: Some(active_locator),
        review_status: CitationReviewStatus::NotRequired,
        provenance_key: format!("local-section:{}", query.section_id),
        citation_seed: Some(active_seed),
    }];

    if let Some(center_ordinal) = center_ordinal {
        candidates.extend(load_neighbors(pool, query, center_ordinal).await?);
    }
    candidates.extend(load_heading_lineage(pool, query, &section).await?);
    candidates.extend(load_definitions(pool, query).await?);

    let search_query = if query.query_text.trim().is_empty() {
        query.selected_text.as_str()
    } else {
        query.query_text.trim()
    };
    for hit in search_book(pool, query.book_id, search_query, 40).await? {
        candidates.push(candidate_from_search_hit(query, hit)?);
    }
    Ok(candidates)
}

async fn validate_anchor_ownership(
    pool: &SqlitePool,
    query: &SelectionContextQuery,
    book_format: &str,
) -> AppResult<(DocumentLocator, ContextSourceKind, Option<u32>)> {
    match &query.anchor {
        ContentAnchor::Text { selection } => {
            if selection.quote.exact != query.selected_text
                || selection
                    .section_id
                    .is_some_and(|section_id| section_id != query.section_id)
            {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
            validate_document_locator(pool, query, book_format, &selection.locator).await?;
            let center = find_center_ordinal(pool, query, Some(&selection.locator), None).await?;
            Ok((
                selection.locator.clone(),
                ContextSourceKind::Selection,
                center,
            ))
        }
        ContentAnchor::Region { region } => {
            if region
                .text_fallback
                .as_ref()
                .is_some_and(|fallback| fallback.exact != query.selected_text)
            {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
            let (locator, block_id) = match &region.locator {
                RegionLocator::Pdf { page } if book_format == "pdf" => (
                    DocumentLocator::pdf(*page, *page, None)
                        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?,
                    None,
                ),
                RegionLocator::Epub { section_id, cfi }
                    if book_format == "epub" && *section_id == query.section_id =>
                {
                    (
                        DocumentLocator::epub(cfi.clone(), *section_id)
                            .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?,
                        None,
                    )
                }
                RegionLocator::Docx { block_id } if book_format == "docx" => (
                    DocumentLocator::docx(*block_id, 0, *block_id, 0)
                        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?,
                    Some(*block_id),
                ),
                _ => return Err(AppError::new(AppErrorCode::InvalidInput)),
            };
            validate_document_locator(pool, query, book_format, &locator).await?;
            let center = find_center_ordinal(pool, query, Some(&locator), block_id).await?;
            Ok((locator, ContextSourceKind::Region, center))
        }
    }
}

async fn validate_document_locator(
    pool: &SqlitePool,
    query: &SelectionContextQuery,
    book_format: &str,
    locator: &DocumentLocator,
) -> AppResult<()> {
    let valid = match locator {
        DocumentLocator::Pdf { .. } => book_format == "pdf",
        DocumentLocator::Epub { section_id, .. } => {
            if book_format != "epub" || *section_id != query.section_id {
                false
            } else {
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM sections WHERE id = ? AND book_id = ?",
                )
                .bind(section_id.to_string())
                .bind(query.book_id.to_string())
                .fetch_one(pool)
                .await?
                    == 1
            }
        }
        DocumentLocator::Docx {
            start_block_id,
            end_block_id,
            ..
        } => {
            if book_format != "docx" {
                false
            } else {
                let expected = if start_block_id == end_block_id { 1 } else { 2 };
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(DISTINCT id) FROM blocks WHERE book_id = ? AND section_id = ? AND id IN (?, ?)",
                )
                .bind(query.book_id.to_string())
                .bind(query.section_id.to_string())
                .bind(start_block_id.to_string())
                .bind(end_block_id.to_string())
                .fetch_one(pool)
                .await?
                    == expected
            }
        }
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::InvalidInput))
    }
}

async fn find_center_ordinal(
    pool: &SqlitePool,
    query: &SelectionContextQuery,
    locator: Option<&DocumentLocator>,
    block_id: Option<Uuid>,
) -> AppResult<Option<u32>> {
    if let Some(block_id) = block_id {
        let ordinal = sqlx::query_scalar::<_, i64>(
            "SELECT ordinal FROM blocks WHERE id = ? AND book_id = ? AND section_id = ?",
        )
        .bind(block_id.to_string())
        .bind(query.book_id.to_string())
        .bind(query.section_id.to_string())
        .fetch_optional(pool)
        .await?;
        return ordinal.map(to_u32).transpose();
    }
    let locator_json = locator
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    let row = sqlx::query(
        "SELECT ordinal FROM blocks WHERE book_id = ? AND section_id = ? AND (instr(plain_text, ?) > 0 OR locator_json = ?) ORDER BY CASE WHEN instr(plain_text, ?) > 0 THEN 0 ELSE 1 END, ordinal, id LIMIT 1",
    )
    .bind(query.book_id.to_string())
    .bind(query.section_id.to_string())
    .bind(&query.selected_text)
    .bind(locator_json)
    .bind(&query.selected_text)
    .fetch_optional(pool)
    .await?;
    row.map(|row| to_u32(row.try_get::<i64, _>("ordinal")?))
        .transpose()
}

async fn load_neighbors(
    pool: &SqlitePool,
    query: &SelectionContextQuery,
    center_ordinal: u32,
) -> AppResult<Vec<ContextCandidate>> {
    let start = center_ordinal.saturating_sub(2);
    let end = center_ordinal.saturating_add(2);
    let rows = sqlx::query(
        "SELECT id, ordinal, plain_text, locator_json FROM blocks WHERE book_id = ? AND section_id = ? AND ordinal BETWEEN ? AND ? AND ordinal != ? ORDER BY ordinal, id",
    )
    .bind(query.book_id.to_string())
    .bind(query.section_id.to_string())
    .bind(i64::from(start))
    .bind(i64::from(end))
    .bind(i64::from(center_ordinal))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| local_candidate(query, row, ContextSourceKind::Neighbor, 900_000))
        .collect()
}

async fn load_heading_lineage(
    pool: &SqlitePool,
    query: &SelectionContextQuery,
    initial: &SqliteRow,
) -> AppResult<Vec<ContextCandidate>> {
    let mut candidates = Vec::new();
    let mut current = Some((
        parse_uuid(initial.try_get::<String, _>("id")?)?,
        initial.try_get::<Option<String>, _>("parent_id")?,
        to_u32(initial.try_get::<i64, _>("ordinal")?)?,
        initial.try_get::<String, _>("title")?,
        initial.try_get::<String, _>("locator_json")?,
    ));
    let mut visited = BTreeSet::new();
    for depth in 0..32_u32 {
        let Some((section_id, parent_id, ordinal, title, locator_json)) = current.take() else {
            break;
        };
        if !visited.insert(section_id) {
            return Err(AppError::new(AppErrorCode::DatabaseError));
        }
        let locator = parse_locator_json(&locator_json)?;
        candidates.push(ContextCandidate {
            stable_id: format!("heading:{section_id}"),
            book_id: query.book_id,
            section_id: Some(section_id),
            source_kind: ContextSourceKind::Heading,
            source: ContextSource::LocalText,
            same_section: section_id == query.section_id,
            relevance_micros: 800_000_u32.saturating_sub(depth),
            ordinal,
            text: title.clone(),
            locator_label: locator_label(&locator, Some(&title)),
            locator: Some(locator),
            review_status: CitationReviewStatus::NotRequired,
            provenance_key: format!("local-section:{section_id}"),
            citation_seed: None,
        });
        let Some(parent_id) = parent_id else {
            break;
        };
        let parent_uuid = parse_uuid(parent_id)?;
        let row = sqlx::query(
            "SELECT id, parent_id, ordinal, title, locator_json FROM sections WHERE id = ? AND book_id = ?",
        )
        .bind(parent_uuid.to_string())
        .bind(query.book_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
        current = Some((
            parse_uuid(row.try_get::<String, _>("id")?)?,
            row.try_get("parent_id")?,
            to_u32(row.try_get::<i64, _>("ordinal")?)?,
            row.try_get("title")?,
            row.try_get("locator_json")?,
        ));
    }
    Ok(candidates)
}

async fn load_definitions(
    pool: &SqlitePool,
    query: &SelectionContextQuery,
) -> AppResult<Vec<ContextCandidate>> {
    let rows = sqlx::query(
        "SELECT id, ordinal, plain_text, locator_json FROM blocks WHERE book_id = ? AND section_id = ? AND (lower(ltrim(plain_text)) LIKE 'definition%' OR ltrim(plain_text) LIKE '定义%') ORDER BY ordinal, id LIMIT 8",
    )
    .bind(query.book_id.to_string())
    .bind(query.section_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| local_candidate(query, row, ContextSourceKind::Definition, 700_000))
        .collect()
}

fn local_candidate(
    query: &SelectionContextQuery,
    row: SqliteRow,
    source_kind: ContextSourceKind,
    relevance_micros: u32,
) -> AppResult<ContextCandidate> {
    let stable_id: String = row.try_get("id")?;
    let ordinal = to_u32(row.try_get::<i64, _>("ordinal")?)?;
    let text: String = row.try_get("plain_text")?;
    let locator = parse_locator_json(&row.try_get::<String, _>("locator_json")?)?;
    let label = locator_label(&locator, None);
    let citation_seed = CitationSeed::new(
        query.book_id,
        Some(query.section_id),
        locator.clone(),
        label.clone(),
        ContentSource::LocalText,
        CitationReviewStatus::NotRequired,
    )?;
    Ok(ContextCandidate {
        stable_id,
        book_id: query.book_id,
        section_id: Some(query.section_id),
        source_kind,
        source: ContextSource::LocalText,
        same_section: true,
        relevance_micros,
        ordinal,
        text,
        locator_label: label,
        locator: Some(locator),
        review_status: CitationReviewStatus::NotRequired,
        provenance_key: format!("local-section:{}", query.section_id),
        citation_seed: Some(citation_seed),
    })
}

fn candidate_from_search_hit(
    query: &SelectionContextQuery,
    hit: SearchHit,
) -> AppResult<ContextCandidate> {
    if hit.context_book_id != query.book_id {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    let source = ContextSource::from(hit.provenance.source);
    let provenance_key = hit.context_section_id.map_or_else(
        || hit.provenance.stable_scope_key(),
        |section_id| format!("local-section:{section_id}"),
    );
    let label = locator_label(&hit.locator, hit.section_title.as_deref());
    let citation_seed = if hit.provenance.quoteable {
        Some(CitationSeed::new(
            query.book_id,
            hit.context_section_id,
            hit.locator.clone(),
            label.clone(),
            hit.provenance.source,
            hit.provenance.review_status,
        )?)
    } else {
        None
    };
    Ok(ContextCandidate {
        stable_id: hit.context_stable_id,
        book_id: query.book_id,
        section_id: hit.context_section_id,
        source_kind: ContextSourceKind::TextbookSearch,
        source,
        same_section: hit.context_section_id == Some(query.section_id),
        relevance_micros: hit.relevance_micros,
        ordinal: hit.context_ordinal,
        text: hit.context_text,
        locator_label: label,
        locator: Some(hit.locator),
        review_status: hit.provenance.review_status,
        provenance_key,
        citation_seed,
    })
}

pub(crate) fn book_candidate_from_search_hit(
    book_id: Uuid,
    hit: SearchHit,
) -> AppResult<ContextCandidate> {
    if hit.context_book_id != book_id {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    let source = ContextSource::from(hit.provenance.source);
    let provenance_key = hit.context_section_id.map_or_else(
        || hit.provenance.stable_scope_key(),
        |section_id| format!("local-section:{section_id}"),
    );
    let label = locator_label(&hit.locator, hit.section_title.as_deref());
    let citation_seed = if hit.provenance.quoteable {
        Some(CitationSeed::new(
            book_id,
            hit.context_section_id,
            hit.locator.clone(),
            label.clone(),
            hit.provenance.source,
            hit.provenance.review_status,
        )?)
    } else {
        None
    };
    Ok(ContextCandidate {
        stable_id: hit.context_stable_id,
        book_id,
        section_id: hit.context_section_id,
        source_kind: ContextSourceKind::TextbookSearch,
        source,
        same_section: false,
        relevance_micros: hit.relevance_micros,
        ordinal: hit.context_ordinal,
        text: hit.context_text,
        locator_label: label,
        locator: Some(hit.locator),
        review_status: hit.provenance.review_status,
        provenance_key,
        citation_seed,
    })
}

pub fn locator_label(locator: &DocumentLocator, section_title: Option<&str>) -> String {
    if let Some(title) = section_title.filter(|title| !title.trim().is_empty()) {
        return title.trim().chars().take(256).collect();
    }
    match locator {
        DocumentLocator::Pdf {
            start_page,
            end_page,
            ..
        } if start_page == end_page => format!("page {start_page}"),
        DocumentLocator::Pdf {
            start_page,
            end_page,
            ..
        } => format!("pages {start_page}-{end_page}"),
        DocumentLocator::Epub { .. } => "EPUB passage".to_owned(),
        DocumentLocator::Docx { .. } => "DOCX passage".to_owned(),
    }
}

fn parse_locator_json(value: &str) -> AppResult<DocumentLocator> {
    serde_json::from_str(value).map_err(AppError::database)
}

fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

fn to_u32(value: i64) -> AppResult<u32> {
    u32::try_from(value).map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}
