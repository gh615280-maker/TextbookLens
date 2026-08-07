use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{CitationReviewStatus, ContentSource};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalProvenance {
    pub source: ContentSource,
    pub quoteable: bool,
    #[serde(default)]
    pub review_status: CitationReviewStatus,
    #[serde(skip_serializing)]
    pub page_id: Option<Uuid>,
    #[serde(skip_serializing)]
    pub block_id: Option<Uuid>,
    #[serde(skip_serializing)]
    pub correction_id: Option<Uuid>,
    pub original_source: Option<ContentSource>,
    #[serde(skip_serializing)]
    pub original_value_sha256: Option<String>,
}

impl RetrievalProvenance {
    pub fn local_text() -> Self {
        Self {
            source: ContentSource::LocalText,
            quoteable: true,
            review_status: CitationReviewStatus::NotRequired,
            page_id: None,
            block_id: None,
            correction_id: None,
            original_source: None,
            original_value_sha256: None,
        }
    }

    pub fn file_extracted() -> Self {
        Self {
            source: ContentSource::LocalText,
            quoteable: true,
            review_status: CitationReviewStatus::NotRequired,
            page_id: None,
            block_id: None,
            correction_id: None,
            original_source: None,
            original_value_sha256: None,
        }
    }

    pub fn indexed(
        source: ContentSource,
        page_id: Uuid,
        block_id: Uuid,
        correction_id: Option<Uuid>,
        original_value_sha256: Option<String>,
    ) -> Self {
        let review_status = if source == ContentSource::UserCorrected {
            CitationReviewStatus::UserCorrected
        } else {
            CitationReviewStatus::Indexed
        };
        Self::indexed_with_review(
            source,
            page_id,
            block_id,
            correction_id,
            original_value_sha256,
            review_status,
        )
    }

    pub fn indexed_with_review(
        source: ContentSource,
        page_id: Uuid,
        block_id: Uuid,
        correction_id: Option<Uuid>,
        original_value_sha256: Option<String>,
        review_status: CitationReviewStatus,
    ) -> Self {
        Self {
            source,
            quoteable: source != ContentSource::AiDescription,
            review_status,
            page_id: Some(page_id),
            block_id: Some(block_id),
            correction_id,
            original_source: (source == ContentSource::UserCorrected)
                .then_some(ContentSource::AiTranscribed),
            original_value_sha256,
        }
    }

    pub const fn ranking_weight(&self) -> u32 {
        match self.source {
            ContentSource::LocalText => 1_000_000,
            ContentSource::AiTranscribed => 900_000,
            ContentSource::AiDescription => 250_000,
            ContentSource::UserCorrected => 1_050_000,
        }
    }

    pub fn stable_scope_key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}",
            self.source.as_str(),
            self.page_id.map_or_else(String::new, |id| id.to_string()),
            self.block_id.map_or_else(String::new, |id| id.to_string()),
            self.correction_id
                .map_or_else(String::new, |id| id.to_string()),
            self.original_value_sha256.as_deref().unwrap_or_default()
        )
    }
}

impl fmt::Debug for RetrievalProvenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetrievalProvenance")
            .field("source", &self.source)
            .field("quoteable", &self.quoteable)
            .field("review_status", &self.review_status)
            .field("page_id", &self.page_id.map(|_| "<redacted>"))
            .field("block_id", &self.block_id.map(|_| "<redacted>"))
            .field("correction_id", &self.correction_id.map(|_| "<redacted>"))
            .field("original_source", &self.original_source)
            .field(
                "original_value_sha256",
                &self.original_value_sha256.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptions_are_lower_weight_and_never_quoteable() {
        let page_id = Uuid::new_v4();
        let block_id = Uuid::new_v4();
        let description = RetrievalProvenance::indexed(
            ContentSource::AiDescription,
            page_id,
            block_id,
            None,
            None,
        );
        let transcription = RetrievalProvenance::indexed(
            ContentSource::AiTranscribed,
            page_id,
            block_id,
            None,
            None,
        );
        assert!(!description.quoteable);
        assert!(transcription.quoteable);
        assert!(description.ranking_weight() < transcription.ranking_weight());
    }

    #[test]
    fn corrected_provenance_retains_the_original_audit_link() {
        let correction_id = Uuid::new_v4();
        let provenance = RetrievalProvenance::indexed(
            ContentSource::UserCorrected,
            Uuid::new_v4(),
            Uuid::new_v4(),
            Some(correction_id),
            Some("a".repeat(64)),
        );
        assert_eq!(provenance.correction_id, Some(correction_id));
        assert_eq!(
            provenance.original_source,
            Some(ContentSource::AiTranscribed)
        );
        assert_eq!(provenance.original_value_sha256, Some("a".repeat(64)));
    }

    #[test]
    fn audit_identifiers_are_available_locally_but_redacted_from_dto_and_debug() {
        let page_id = Uuid::new_v4();
        let block_id = Uuid::new_v4();
        let correction_id = Uuid::new_v4();
        let hash = "f".repeat(64);
        let provenance = RetrievalProvenance::indexed(
            ContentSource::UserCorrected,
            page_id,
            block_id,
            Some(correction_id),
            Some(hash.clone()),
        );
        assert_eq!(provenance.page_id, Some(page_id));
        assert_eq!(provenance.block_id, Some(block_id));
        let serialized = serde_json::to_string(&provenance).unwrap();
        let debug = format!("{provenance:?}");
        for forbidden in [
            page_id.to_string(),
            block_id.to_string(),
            correction_id.to_string(),
            hash,
        ] {
            assert!(!serialized.contains(&forbidden));
            assert!(!debug.contains(&forbidden));
        }
        assert!(serialized.contains("user_corrected"));
        assert!(debug.contains("<redacted>"));
    }
}
