use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::ContentSource;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetrievalProvenance {
    pub source: ContentSource,
    pub quoteable: bool,
    pub page_id: Option<Uuid>,
    pub block_id: Option<Uuid>,
    pub correction_id: Option<Uuid>,
    pub original_source: Option<ContentSource>,
    pub original_value_sha256: Option<String>,
}

impl RetrievalProvenance {
    pub fn local_text() -> Self {
        Self {
            source: ContentSource::LocalText,
            quoteable: true,
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
        Self {
            source,
            quoteable: source != ContentSource::AiDescription,
            page_id: Some(page_id),
            block_id: Some(block_id),
            correction_id,
            original_source: (source == ContentSource::UserCorrected)
                .then_some(ContentSource::AiTranscribed),
            original_value_sha256,
        }
    }

    pub const fn ranking_weight(&self) -> f64 {
        match self.source {
            ContentSource::LocalText => 1.0,
            ContentSource::AiTranscribed => 0.9,
            ContentSource::AiDescription => 0.25,
            ContentSource::UserCorrected => 1.05,
        }
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
}
