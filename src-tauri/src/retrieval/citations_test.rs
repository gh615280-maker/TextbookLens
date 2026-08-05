use uuid::Uuid;

use crate::{
    domain::{Citation, CitationReviewStatus, ContentSource, DocumentLocator},
    errors::AppErrorCode,
    learning::ContextSource,
    retrieval::{
        budget::InputBudget,
        citations::CitationSeed,
        context::{ContextCandidate, ContextSourceKind, pack_context},
    },
};

fn candidate(
    book_id: Uuid,
    stable_id: &str,
    source_kind: ContextSourceKind,
    source: ContextSource,
) -> ContextCandidate {
    let content_source = source.content_source();
    let locator = DocumentLocator::pdf(42, 42, None).unwrap();
    let review_status = match source {
        ContextSource::AiTranscribed => CitationReviewStatus::NeedsReview,
        ContextSource::UserCorrected => CitationReviewStatus::UserCorrected,
        _ => CitationReviewStatus::NotRequired,
    };
    let citation_seed = content_source
        .filter(|source| *source != ContentSource::AiDescription)
        .map(|source| {
            CitationSeed::new(
                book_id,
                Some(Uuid::nil()),
                locator.clone(),
                "page 42".to_owned(),
                source,
                review_status,
            )
            .unwrap()
        });
    ContextCandidate {
        stable_id: stable_id.to_owned(),
        book_id,
        section_id: Some(Uuid::nil()),
        source_kind,
        source,
        same_section: true,
        relevance_micros: 100,
        ordinal: 0,
        text: format!("content for {stable_id}"),
        locator_label: "page 42".to_owned(),
        locator: content_source.map(|_| locator),
        review_status,
        provenance_key: format!("scope:{stable_id}"),
        citation_seed,
    }
}

fn budget(usable_input: u32) -> InputBudget {
    InputBudget {
        effective_window: usable_input.saturating_add(4_608),
        output_reserve: 4_096,
        protocol_reserve: 512,
        usable_input,
    }
}

fn estimate(segments: &[crate::retrieval::context::ContextSegment]) -> u64 {
    64 + segments
        .iter()
        .map(|segment| {
            segment.content.chars().count()
                + segment
                    .citation
                    .as_ref()
                    .map_or(0, |citation| citation.id.chars().count())
                + 8
        })
        .map(|count| u64::try_from(count).unwrap_or(u64::MAX))
        .sum::<u64>()
}

#[test]
fn citations_are_assigned_only_after_final_packing_without_id_gaps() {
    let book_id = Uuid::new_v4();
    let mandatory = candidate(
        book_id,
        "mandatory",
        ContextSourceKind::Selection,
        ContextSource::LocalText,
    );
    let first = candidate(
        book_id,
        "first",
        ContextSourceKind::Neighbor,
        ContextSource::AiTranscribed,
    );
    let second = candidate(
        book_id,
        "second",
        ContextSourceKind::TextbookSearch,
        ContextSource::UserCorrected,
    );
    let dropped = candidate(
        book_id,
        "dropped",
        ContextSourceKind::TextbookSearch,
        ContextSource::LocalText,
    );
    let fit_three = estimate(&[
        finalized(&mandatory, 1),
        finalized(&first, 2),
        finalized(&second, 3),
    ]);
    let packed = pack_context(
        book_id,
        budget(u32::try_from(fit_three).unwrap()),
        vec![dropped, second, mandatory, first],
        |segments| Ok(estimate(segments)),
    )
    .unwrap();
    assert_eq!(
        packed
            .citations
            .iter()
            .map(|citation| citation.id.as_str())
            .collect::<Vec<_>>(),
        ["TL-C1", "TL-C2", "TL-C3"]
    );
    assert_eq!(packed.omitted_segment_count, 1);
    assert_eq!(
        packed.citations[1].review_status,
        CitationReviewStatus::NeedsReview
    );
    assert_eq!(packed.citations[1].source, ContentSource::AiTranscribed);
    assert_eq!(packed.citations[1].label, "page 42");
    assert!(packed.citations.iter().all(|citation| citation.quoteable));
}

fn finalized(
    candidate: &ContextCandidate,
    ordinal: u32,
) -> crate::retrieval::context::ContextSegment {
    let citation = candidate.citation_seed.as_ref().map(|seed| {
        Citation::new(
            format!("TL-C{ordinal}"),
            seed.label.clone(),
            seed.book_id,
            seed.section_id,
            seed.locator.clone(),
            seed.source,
            seed.review_status,
        )
        .unwrap()
    });
    crate::retrieval::context::ContextSegment {
        stable_id: candidate.stable_id.clone(),
        book_id: candidate.book_id,
        source_kind: candidate.source_kind,
        source: candidate.source,
        locator_label: candidate.locator_label.clone(),
        review_status: candidate.review_status,
        content: candidate.text.clone(),
        citation_seed: candidate.citation_seed.clone(),
        citation,
    }
}

#[test]
fn descriptions_notes_and_history_can_never_create_quote_citations() {
    let book_id = Uuid::new_v4();
    let locator = DocumentLocator::pdf(1, 1, None).unwrap();
    let error = CitationSeed::new(
        book_id,
        None,
        locator,
        "page 1".to_owned(),
        ContentSource::AiDescription,
        CitationReviewStatus::Indexed,
    )
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidInput);

    for source in [
        ContextSource::AiDescription,
        ContextSource::UserNote,
        ContextSource::HistorySummary,
    ] {
        assert!(!source.quoteable());
    }

    let invalid: Result<Citation, _> = serde_json::from_value(serde_json::json!({
        "id": "TL-C1",
        "label": "page 1",
        "bookId": book_id,
        "sectionId": null,
        "locator": {"format":"pdf", "startPage":1, "endPage":1, "rectsByPage":null},
        "source": "ai_description",
        "reviewStatus": "indexed",
        "quoteable": true
    }));
    assert!(invalid.is_err());
}

#[test]
fn citation_serialization_and_debug_never_include_content_or_internal_provider_fields() {
    let book_id = Uuid::new_v4();
    let citation = Citation::new(
        "TL-C1".to_owned(),
        "page 42".to_owned(),
        book_id,
        None,
        DocumentLocator::pdf(42, 42, None).unwrap(),
        ContentSource::LocalText,
        CitationReviewStatus::NotRequired,
    )
    .unwrap();
    let serialized = serde_json::to_string(&citation).unwrap();
    assert!(serialized.contains(&book_id.to_string()));
    for forbidden in [
        "storedPath",
        "body",
        "providerBody",
        "credential",
        "attemptId",
        "runId",
        "apiKey",
    ] {
        assert!(!serialized.contains(forbidden));
    }
    let debug = format!("{citation:?}");
    assert!(!debug.contains(&book_id.to_string()));
    assert!(!debug.contains("start_page"));
    assert!(debug.contains("<redacted>"));
}
