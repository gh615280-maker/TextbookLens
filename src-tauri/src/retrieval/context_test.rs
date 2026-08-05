use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    db::{
        Database,
        indexing::{CreateIndexRun, create_index_run},
    },
    domain::{
        CitationReviewStatus, ContentAnchor, ContentSource, DocumentLocator, IndexPageBlockKind,
        IndexQualityReason, NormalizedRect, SelectionAnchor, TextQuote,
    },
    errors::AppErrorCode,
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
    learning::ContextSource,
    retrieval::{
        budget::InputBudget,
        citations::CitationSeed,
        context::{
            ContextCandidate, ContextSourceKind, HISTORY_PLACEHOLDER, SelectionContextQuery,
            pack_context, retrieve_selection_context,
        },
    },
};

fn test_budget(usable_input: u32) -> InputBudget {
    InputBudget {
        effective_window: usable_input.saturating_add(4_608),
        output_reserve: 4_096,
        protocol_reserve: 512,
        usable_input,
    }
}

fn base_candidate(
    book_id: Uuid,
    source_kind: ContextSourceKind,
    stable_id: &str,
    text: &str,
) -> ContextCandidate {
    let locator = DocumentLocator::pdf(1, 1, None).unwrap();
    ContextCandidate {
        stable_id: stable_id.to_owned(),
        book_id,
        section_id: Some(Uuid::nil()),
        source_kind,
        source: ContextSource::LocalText,
        same_section: true,
        relevance_micros: 500_000,
        ordinal: 0,
        text: text.to_owned(),
        locator_label: "page 1".to_owned(),
        locator: Some(locator.clone()),
        review_status: CitationReviewStatus::NotRequired,
        provenance_key: "local-section:synthetic".to_owned(),
        citation_seed: Some(
            CitationSeed::new(
                book_id,
                Some(Uuid::nil()),
                locator,
                "page 1".to_owned(),
                ContentSource::LocalText,
                CitationReviewStatus::NotRequired,
            )
            .unwrap(),
        ),
    }
}

fn estimate(segments: &[crate::retrieval::context::ContextSegment]) -> u64 {
    100_u64.saturating_add(
        segments
            .iter()
            .map(|segment| {
                segment.content.chars().count()
                    + segment.locator_label.chars().count()
                    + segment
                        .citation
                        .as_ref()
                        .map_or(0, |citation| citation.id.chars().count())
                    + 12
            })
            .map(|value| u64::try_from(value).unwrap_or(u64::MAX))
            .sum::<u64>(),
    )
}

#[test]
fn context_optional_drop_order_is_history_then_low_relevance_search_then_earlier_classes() {
    let book_id = Uuid::new_v4();
    let mandatory = base_candidate(
        book_id,
        ContextSourceKind::Selection,
        "mandatory",
        &"选".repeat(20),
    );
    let neighbor = base_candidate(
        book_id,
        ContextSourceKind::Neighbor,
        "neighbor",
        &"邻".repeat(20),
    );
    let heading = base_candidate(
        book_id,
        ContextSourceKind::Heading,
        "heading",
        &"题".repeat(20),
    );
    let mut high = base_candidate(
        book_id,
        ContextSourceKind::TextbookSearch,
        "search-high",
        &"高".repeat(20),
    );
    high.relevance_micros = 900_000;
    high.provenance_key = "local-section:high".to_owned();
    let mut low = base_candidate(
        book_id,
        ContextSourceKind::TextbookSearch,
        "search-low",
        &"低".repeat(20),
    );
    low.relevance_micros = 1;
    low.provenance_key = "local-section:low".to_owned();
    let history = ContextCandidate::history_placeholder(book_id, Some(Uuid::nil()));

    let keep_through_high = estimate(&[
        segment_for_estimate(&mandatory, 1),
        segment_for_estimate(&neighbor, 2),
        segment_for_estimate(&heading, 3),
        segment_for_estimate(&high, 4),
    ]);
    let packed = pack_context(
        book_id,
        test_budget(u32::try_from(keep_through_high).unwrap()),
        vec![history, low, heading, mandatory, high, neighbor],
        |segments| Ok(estimate(segments)),
    )
    .unwrap();

    assert_eq!(
        packed
            .segments
            .iter()
            .map(|segment| segment.stable_id.as_str())
            .collect::<Vec<_>>(),
        ["mandatory", "neighbor", "heading", "search-high"]
    );
    assert_eq!(packed.omitted_segment_count, 2);
    assert!(!packed.segments.iter().any(|segment| {
        segment.source_kind == ContextSourceKind::HistorySummary
            && segment.content != HISTORY_PLACEHOLDER
    }));
}

fn segment_for_estimate(
    candidate: &ContextCandidate,
    citation_ordinal: u32,
) -> crate::retrieval::context::ContextSegment {
    let citation = candidate.citation_seed.as_ref().map(|seed| {
        crate::domain::Citation::new(
            format!("TL-C{citation_ordinal}"),
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
fn context_ties_are_stable_and_do_not_depend_on_input_order() {
    let book_id = Uuid::new_v4();
    let mandatory = base_candidate(book_id, ContextSourceKind::Selection, "mandatory", "active");
    let mut alpha = base_candidate(
        book_id,
        ContextSourceKind::TextbookSearch,
        "alpha",
        "alpha text",
    );
    alpha.provenance_key = "scope-alpha".to_owned();
    let mut beta = base_candidate(
        book_id,
        ContextSourceKind::TextbookSearch,
        "beta",
        "beta text",
    );
    beta.provenance_key = "scope-beta".to_owned();
    let first = pack_context(
        book_id,
        test_budget(10_000),
        vec![beta.clone(), mandatory.clone(), alpha.clone()],
        |segments| Ok(estimate(segments)),
    )
    .unwrap();
    let second = pack_context(
        book_id,
        test_budget(10_000),
        vec![alpha, beta, mandatory],
        |segments| Ok(estimate(segments)),
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        first
            .segments
            .iter()
            .map(|segment| segment.stable_id.as_str())
            .collect::<Vec<_>>(),
        ["mandatory", "alpha", "beta"]
    );
}

#[test]
fn context_overlap_dedupes_only_same_provenance_source_and_locator() {
    let book_id = Uuid::new_v4();
    let mandatory = base_candidate(book_id, ContextSourceKind::Selection, "mandatory", "active");
    let wording = "deterministic overlap phrase ".repeat(12);
    let mut neighbor = base_candidate(book_id, ContextSourceKind::Neighbor, "neighbor", &wording);
    neighbor.provenance_key = "same-scope".to_owned();
    let mut overlapping = base_candidate(
        book_id,
        ContextSourceKind::TextbookSearch,
        "overlap-search",
        &format!("{}tail", wording),
    );
    overlapping.provenance_key = "same-scope".to_owned();
    let mut different_source = base_candidate(
        book_id,
        ContextSourceKind::TextbookSearch,
        "ai-same-wording",
        &wording,
    );
    different_source.source = ContextSource::AiTranscribed;
    different_source.review_status = CitationReviewStatus::Indexed;
    different_source.provenance_key = "same-scope".to_owned();
    different_source.citation_seed = Some(
        CitationSeed::new(
            book_id,
            Some(Uuid::nil()),
            different_source.locator.clone().unwrap(),
            "page 1".to_owned(),
            ContentSource::AiTranscribed,
            CitationReviewStatus::Indexed,
        )
        .unwrap(),
    );

    let packed = pack_context(
        book_id,
        test_budget(20_000),
        vec![mandatory, overlapping, different_source, neighbor],
        |segments| Ok(estimate(segments)),
    )
    .unwrap();
    let ids = packed
        .segments
        .iter()
        .map(|segment| segment.stable_id.as_str())
        .collect::<Vec<_>>();
    assert!(ids.contains(&"neighbor"));
    assert!(!ids.contains(&"overlap-search"));
    assert!(ids.contains(&"ai-same-wording"));
}

#[test]
fn context_rejects_cross_book_candidates_and_mandatory_overflow_before_callbacks() {
    let book_id = Uuid::new_v4();
    let mut decoy = base_candidate(
        Uuid::new_v4(),
        ContextSourceKind::TextbookSearch,
        "decoy",
        "decoy text",
    );
    decoy.provenance_key = "decoy".to_owned();
    let error = pack_context(
        book_id,
        test_budget(10_000),
        vec![
            base_candidate(book_id, ContextSourceKind::Selection, "mandatory", "active"),
            decoy,
        ],
        |segments| Ok(estimate(segments)),
    )
    .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidInput);

    let mut image_callbacks = 0;
    let mut provider_callbacks = 0;
    let error = pack_context(
        book_id,
        test_budget(100),
        vec![base_candidate(
            book_id,
            ContextSourceKind::Region,
            "mandatory-region",
            &"🧪".repeat(400),
        )],
        |segments| Ok(estimate(segments)),
    )
    .expect_err("complete region content cannot be truncated");
    if error.code != AppErrorCode::ContextTooLarge {
        image_callbacks += 1;
        provider_callbacks += 1;
    }
    assert_eq!(error.code, AppErrorCode::ContextTooLarge);
    assert_eq!(image_callbacks, 0);
    assert_eq!(provider_callbacks, 0);
}

#[test]
fn context_retrieval_unions_local_and_page_fts_and_rejects_decoy_book_anchor() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("context.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let profile_id = insert_profile(database.pool()).await;
        let target_book = insert_book(database.pool(), "Target context book", "a").await;
        let decoy_book = insert_book(database.pool(), "Decoy context book", "b").await;
        let target_section = insert_local_context(
            database.pool(),
            target_book,
            "Active spectral beacon selection",
            "Local spectral beacon FTS context",
        )
        .await;
        let decoy_section = insert_local_context(
            database.pool(),
            decoy_book,
            "Decoy active selection",
            &format!("{} decoy context leak", "spectral beacon ".repeat(80)),
        )
        .await;
        commit_ai_page(
            database.pool(),
            target_book,
            profile_id,
            "AI spectral beacon transcription",
            "Auxiliary spectral beacon diagram description",
        )
        .await;
        commit_ai_page(
            database.pool(),
            decoy_book,
            profile_id,
            &"spectral beacon ".repeat(60),
            "Decoy spectral beacon diagram",
        )
        .await;

        let locator = DocumentLocator::pdf(1, 1, None).unwrap();
        let query = SelectionContextQuery {
            book_id: target_book,
            section_id: target_section,
            anchor: ContentAnchor::Text {
                selection: SelectionAnchor {
                    locator: locator.clone(),
                    quote: TextQuote::new(
                        "Active spectral beacon selection".to_owned(),
                        String::new(),
                        String::new(),
                    )
                    .unwrap(),
                    section_id: Some(target_section),
                },
            },
            selected_text: "Active spectral beacon selection".to_owned(),
            query_text: "spectral beacon".to_owned(),
        };
        let candidates = retrieve_selection_context(database.pool(), &query)
            .await
            .unwrap();
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.book_id == target_book)
        );
        assert!(
            candidates
                .iter()
                .all(|candidate| !candidate.text.contains("decoy context leak"))
        );
        assert!(candidates.iter().any(|candidate| {
            candidate.source_kind == ContextSourceKind::TextbookSearch
                && candidate.source == ContextSource::LocalText
        }));
        let transcription = candidates
            .iter()
            .find(|candidate| candidate.source == ContextSource::AiTranscribed)
            .unwrap();
        let description = candidates
            .iter()
            .find(|candidate| candidate.source == ContextSource::AiDescription)
            .unwrap();
        assert!(transcription.relevance_micros > description.relevance_micros);
        assert!(transcription.citation_seed.is_some());
        assert!(description.citation_seed.is_none());

        let cross_book_anchor = SelectionContextQuery {
            anchor: ContentAnchor::Text {
                selection: SelectionAnchor {
                    locator,
                    quote: TextQuote::new(
                        "Active spectral beacon selection".to_owned(),
                        String::new(),
                        String::new(),
                    )
                    .unwrap(),
                    section_id: Some(decoy_section),
                },
            },
            ..query
        };
        let error = retrieve_selection_context(database.pool(), &cross_book_anchor)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput);
    });
}

async fn insert_profile(pool: &sqlx::SqlitePool) -> Uuid {
    let profile_id = Uuid::new_v4();
    let timestamp = "2026-08-05T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Context profile', 'safe-model', 128000, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(timestamp)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    profile_id
}

async fn insert_book(pool: &sqlx::SqlitePool, title: &str, hash_character: &str) -> Uuid {
    let book_id = Uuid::new_v4();
    let timestamp = "2026-08-05T00:00:00.000Z";
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, 'pdf', 'context.pdf', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hash_character.repeat(64))
    .bind(title)
    .bind(format!("books/{book_id}/original.pdf"))
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    book_id
}

async fn insert_local_context(
    pool: &sqlx::SqlitePool,
    book_id: Uuid,
    selected_text: &str,
    search_text: &str,
) -> Uuid {
    let section_id = Uuid::new_v4();
    let locator = serde_json::to_string(&DocumentLocator::pdf(1, 1, None).unwrap()).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Synthetic target heading', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(&locator)
    .execute(pool)
    .await
    .unwrap();
    for (ordinal, kind, text) in [
        (0_i64, "heading", "Synthetic target heading"),
        (1, "paragraph", selected_text),
        (2, "paragraph", "Neighbor context after active selection"),
        (3, "paragraph", "Definition: deterministic context"),
    ] {
        sqlx::query(
            "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(book_id.to_string())
        .bind(section_id.to_string())
        .bind(ordinal)
        .bind(kind)
        .bind(text)
        .bind(&locator)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, ?, ?, 10)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(search_text)
    .bind(locator)
    .execute(pool)
    .await
    .unwrap();
    section_id
}

async fn commit_ai_page(
    pool: &sqlx::SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    text: &str,
    description: &str,
) {
    let run_id = create_index_run(
        pool,
        CreateIndexRun {
            book_id,
            provider_profile_id: profile_id,
            analysis_schema_version: "textbooklens.page-analysis.v1".to_owned(),
            render_version: "context-render-v1".to_owned(),
            parser_version: "context-parser-v1".to_owned(),
        },
    )
    .await
    .unwrap();
    let page = state::queue(pool, run_id, 1, IndexQualityReason::NoText, None)
        .await
        .unwrap();
    let attempt_id = page.attempt_id.unwrap();
    state::claim_render(pool, page.page_id, attempt_id)
        .await
        .unwrap();
    state::mark_rendered(pool, page.page_id, attempt_id, &"c".repeat(64))
        .await
        .unwrap();
    state::claim_send(pool, page.page_id, attempt_id)
        .await
        .unwrap();
    state::mark_received(pool, page.page_id, attempt_id, &"d".repeat(64))
        .await
        .unwrap();
    let page_content = ValidatedPage {
        page_number: 1,
        review_reason: None,
        blocks: vec![ValidatedBlock {
            ordinal: 0,
            kind: IndexPageBlockKind::Paragraph,
            plain_text: Some(text.to_owned()),
            latex: None,
            table_cells: None,
            visual_description: Some(description.to_owned()),
            bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
            source: ContentSource::AiTranscribed,
        }],
    };
    commit_validated_page(
        pool,
        PageCommitRequest {
            page_id: page.page_id,
            attempt_id,
            page: &page_content,
        },
        &CancellationToken::new(),
    )
    .await
    .unwrap();
}
