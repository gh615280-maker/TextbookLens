use std::fmt;

use uuid::Uuid;

use crate::{
    domain::{Citation, CitationReviewStatus, ContentSource, DocumentLocator},
    errors::{AppError, AppErrorCode, AppResult},
};

use super::context::ContextSegment;

#[derive(Clone, PartialEq)]
pub struct CitationSeed {
    pub book_id: Uuid,
    pub section_id: Option<Uuid>,
    pub locator: DocumentLocator,
    pub label: String,
    pub source: ContentSource,
    pub review_status: CitationReviewStatus,
}

impl CitationSeed {
    pub fn new(
        book_id: Uuid,
        section_id: Option<Uuid>,
        locator: DocumentLocator,
        label: String,
        source: ContentSource,
        review_status: CitationReviewStatus,
    ) -> AppResult<Self> {
        if source == ContentSource::AiDescription
            || label.trim().is_empty()
            || label.chars().count() > 256
            || label
                .chars()
                .any(|value| value.is_control() && value != '\t')
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        let review_status_matches_source = match source {
            ContentSource::LocalText => review_status == CitationReviewStatus::NotRequired,
            ContentSource::AiTranscribed => matches!(
                review_status,
                CitationReviewStatus::Indexed | CitationReviewStatus::NeedsReview
            ),
            ContentSource::UserCorrected => review_status == CitationReviewStatus::UserCorrected,
            ContentSource::AiDescription => false,
        };
        if !review_status_matches_source {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        Ok(Self {
            book_id,
            section_id,
            locator,
            label,
            source,
            review_status,
        })
    }

    fn finalize(&self, ordinal: u32) -> AppResult<Citation> {
        Citation::new(
            request_local_citation_id(ordinal)?,
            self.label.clone(),
            self.book_id,
            self.section_id,
            self.locator.clone(),
            self.source,
            self.review_status,
        )
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))
    }
}

impl fmt::Debug for CitationSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CitationSeed")
            .field("book_id", &"<redacted>")
            .field("section_id", &self.section_id.map(|_| "<redacted>"))
            .field("locator", &"<redacted>")
            .field("label", &self.label)
            .field("source", &self.source)
            .field("review_status", &self.review_status)
            .finish()
    }
}

pub fn assign_request_local_citations(segments: &mut [ContextSegment]) -> AppResult<Vec<Citation>> {
    let mut citations = Vec::new();
    for segment in segments.iter_mut() {
        segment.citation = None;
        let Some(seed) = &segment.citation_seed else {
            continue;
        };
        let ordinal = u32::try_from(citations.len())
            .ok()
            .and_then(|count| count.checked_add(1))
            .ok_or_else(|| AppError::new(AppErrorCode::ContextTooLarge))?;
        let citation = seed.finalize(ordinal)?;
        segment.citation = Some(citation.clone());
        citations.push(citation);
    }
    Ok(citations)
}

pub fn request_local_citation_id(ordinal: u32) -> AppResult<String> {
    if ordinal == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(format!("TL-C{ordinal}"))
}
