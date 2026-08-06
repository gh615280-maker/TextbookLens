use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::time::Duration;

use async_trait::async_trait;
use sqlx::{Row, SqlitePool};
use tokio::sync::Barrier;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    db::{
        Database,
        conversations::{FollowupCompletion, LearningRepository, NewSelectionCompletion},
        corrections::{SaveIndexCorrection, correction_value_sha256, save_index_correction},
        indexing::{CreateIndexRun, create_index_run},
        messages::CompletedAssistantMessage,
        notes::{CreateNote, create_note},
    },
    domain::{
        BookFormat, Citation, CitationReviewStatus, ContentAnchor, ContentSource, DocumentLocator,
        IndexCorrectionValueKind, IndexPageBlockKind, IndexQualityReason, LearningAction,
        LearningOverviewSource, NormalizedRect, SelectionAnchor, TextQuote, stable_block_id,
        stable_index_page_block_id, stable_section_id,
    },
    errors::AppErrorCode,
    indexing::{
        commit::{PageCommitRequest, commit_validated_page},
        state,
        validator::{ValidatedBlock, ValidatedPage},
    },
};

use super::{
    OVERVIEW_READ_QUERY_COUNT, OverviewReadObserver, checked_add_assign, get_learning_overview,
    get_learning_overview_observed, to_u32_i64,
};

const TIMESTAMP: &str = "2026-08-06T00:00:00.000Z";
const ANALYSIS_SCHEMA: &str = "textbooklens.page-analysis.v1";
const LOCAL_BODY_SENTINEL: &str = "PRIVATE_TEXTBOOK_BODY_SENTINEL";
const NOTE_BODY_SENTINEL: &str = "PRIVATE_NOTE_BODY_SENTINEL";
const QUESTION_SENTINEL: &str = "PRIVATE_PROMPT_SENTINEL";
const ANSWER_SENTINEL: &str = "PRIVATE_ANSWER_SENTINEL";
const DESCRIPTION_SENTINEL: &str = "PRIVATE_AI_DESCRIPTION_SENTINEL";
const CORRECTION_SENTINEL: &str = "PRIVATE_USER_CORRECTION_SENTINEL";
const TEACHING_SENTINEL: &str = "PRIVATE_TEACHING_INSTRUCTION_SENTINEL";

struct TestBook {
    id: Uuid,
    section_id: Option<Uuid>,
    locator: Option<DocumentLocator>,
}

#[test]
fn overview_supports_pdf_epub_docx_and_empty_ready_books() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("formats.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        for (index, format) in ["pdf", "epub", "docx"].into_iter().enumerate() {
            let book = insert_ready_book(
                database.pool(),
                format,
                char::from(b'a' + u8::try_from(index).unwrap()),
                true,
                LOCAL_BODY_SENTINEL,
            )
            .await;
            let overview = get_learning_overview(database.pool(), book.id)
                .await
                .unwrap();
            assert_eq!(
                overview.format,
                match format {
                    "pdf" => BookFormat::Pdf,
                    "epub" => BookFormat::Epub,
                    "docx" => BookFormat::Docx,
                    _ => unreachable!(),
                }
            );
            assert_eq!(overview.section_count, 1);
            assert_eq!(overview.sections[0].local_text_item_count, 1);
            assert_eq!(
                overview.sources[LearningOverviewSource::LocalText.index()].item_count,
                1
            );
            assert!(
                !serde_json::to_string(&overview)
                    .unwrap()
                    .contains(LOCAL_BODY_SENTINEL)
            );
        }

        let empty = insert_ready_book(database.pool(), "pdf", 'd', false, "unused").await;
        let counter = CountingObserver::default();
        let overview = get_learning_overview_observed(database.pool(), empty.id, &counter)
            .await
            .unwrap();
        assert_eq!(
            counter.count.load(Ordering::SeqCst),
            OVERVIEW_READ_QUERY_COUNT
        );
        assert_eq!(overview.section_count, 0);
        assert!(overview.sections.is_empty());
        assert!(overview.sources.iter().all(|source| source.item_count == 0));
    });
}

#[test]
fn mixed_sources_are_effective_isolated_redacted_deterministic_and_restart_stable() {
    let temporary = tempfile::tempdir().unwrap();
    let database_path = temporary.path().join("mixed.sqlite3");
    let database = Database::open(&database_path).unwrap();
    let expected = tauri::async_runtime::block_on(async {
        let profile_id = insert_profile(database.pool()).await;
        let target =
            insert_ready_book(database.pool(), "pdf", 'e', true, LOCAL_BODY_SENTINEL).await;
        let decoy = insert_ready_book(
            database.pool(),
            "pdf",
            'f',
            true,
            "DECOY_PRIVATE_TEXTBOOK_BODY_SENTINEL",
        )
        .await;
        sqlx::query("UPDATE teaching_preferences SET instruction = ?, revision = 1 WHERE id = 1")
            .bind(TEACHING_SENTINEL)
            .execute(database.pool())
            .await
            .unwrap();

        let target_page = commit_ai_page(database.pool(), target.id, profile_id, 1).await;
        let _queued = state::queue(
            database.pool(),
            target_page.run_id,
            2,
            IndexQualityReason::NoText,
            None,
        )
        .await
        .unwrap();
        let decoy_page = commit_ai_page(database.pool(), decoy.id, profile_id, 1).await;

        let correction = save_index_correction(
            database.pool(),
            SaveIndexCorrection {
                book_id: target.id,
                page_id: target_page.page_id,
                target_block_id: target_page.correctable_block_id,
                target_content_version: 1,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256("provider text to correct"),
                corrected_value: CORRECTION_SENTINEL.to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
        save_index_correction(
            database.pool(),
            SaveIndexCorrection {
                book_id: decoy.id,
                page_id: decoy_page.page_id,
                target_block_id: decoy_page.correctable_block_id,
                target_content_version: 1,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256("provider text to correct"),
                corrected_value: "DECOY_PRIVATE_CORRECTION_SENTINEL".to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();

        create_private_note(database.pool(), &target, NOTE_BODY_SENTINEL).await;
        create_private_note(database.pool(), &decoy, "DECOY_PRIVATE_NOTE_BODY_SENTINEL").await;
        persist_history(database.pool(), &target, profile_id).await;
        persist_history(database.pool(), &decoy, profile_id).await;

        let counter = CountingObserver::default();
        let first = get_learning_overview_observed(database.pool(), target.id, &counter)
            .await
            .unwrap();
        let second = get_learning_overview(database.pool(), target.id)
            .await
            .unwrap();
        assert_eq!(
            counter.count.load(Ordering::SeqCst),
            OVERVIEW_READ_QUERY_COUNT
        );
        assert_eq!(first, second);
        assert!(first.teaching_instruction_configured);
        assert_eq!(first.section_count, 1);
        assert_eq!(
            source(&first, LearningOverviewSource::LocalText),
            (1, 1, 0, true)
        );
        assert_eq!(
            source(&first, LearningOverviewSource::AiTranscribed),
            (1, 0, 1, true)
        );
        assert_eq!(
            source(&first, LearningOverviewSource::AiDescription),
            (1, 0, 1, false)
        );
        assert_eq!(
            source(&first, LearningOverviewSource::UserCorrected),
            (1, 0, 1, true)
        );
        assert_eq!(
            source(&first, LearningOverviewSource::UserNote),
            (1, 1, 0, false)
        );
        assert_eq!(
            source(&first, LearningOverviewSource::HistorySummary),
            (2, 1, 0, false)
        );
        assert_eq!(first.activity.user_note_count, 1);
        assert_eq!(first.activity.completed_conversation_count, 1);
        assert_eq!(first.activity.completed_exchange_count, 2);
        assert_eq!(first.activity.citation_count, 1);
        assert_eq!(first.sections[0].completed_conversation_count, 1);
        assert_eq!(first.sections[0].completed_exchange_count, 2);
        let serialized = serde_json::to_string(&first).unwrap();
        for forbidden in [
            LOCAL_BODY_SENTINEL,
            NOTE_BODY_SENTINEL,
            QUESTION_SENTINEL,
            ANSWER_SENTINEL,
            DESCRIPTION_SENTINEL,
            CORRECTION_SENTINEL,
            TEACHING_SENTINEL,
            "DECOY_PRIVATE",
            "synthetic-provider-model",
            "books/",
        ] {
            assert!(!serialized.contains(forbidden), "leaked {forbidden}");
        }
        assert!(!format!("{first:?}").contains("Visible synthetic section"));

        sqlx::query("UPDATE index_corrections SET conflict_state = 'conflict' WHERE id = ?")
            .bind(correction.id.to_string())
            .execute(database.pool())
            .await
            .unwrap();
        let conflicted = get_learning_overview(database.pool(), target.id)
            .await
            .unwrap();
        assert_eq!(
            source(&conflicted, LearningOverviewSource::UserCorrected).0,
            0
        );
        sqlx::query("UPDATE index_corrections SET conflict_state = 'active' WHERE id = ?")
            .bind(correction.id.to_string())
            .execute(database.pool())
            .await
            .unwrap();

        first
    });
    tauri::async_runtime::block_on(database.pool().close());
    drop(database);
    let reopened = Database::open(&database_path).unwrap();
    let actual =
        tauri::async_runtime::block_on(get_learning_overview(reopened.pool(), expected.book_id))
            .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn one_transaction_snapshot_excludes_a_concurrent_note_until_the_next_read() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("snapshot.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let book = insert_ready_book(database.pool(), "pdf", '1', true, "snapshot text").await;
        let observer = Arc::new(PauseObserver {
            reached: Arc::new(Barrier::new(2)),
            resume: Arc::new(Barrier::new(2)),
        });
        let pool = database.pool().clone();
        let task_observer = observer.clone();
        let mut read = tokio::spawn(async move {
            get_learning_overview_observed(&pool, book.id, task_observer.as_ref()).await
        });
        tokio::select! {
            result = &mut read => panic!("overview reader ended before the snapshot barrier: {result:?}"),
            _ = observer.reached.wait() => {},
            _ = tokio::time::sleep(Duration::from_secs(5)) => panic!("overview reader reached the snapshot barrier"),
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            insert_note_row(database.pool(), &book, "concurrent private note"),
        )
        .await
        .expect("concurrent writer committed while the WAL snapshot was open");
        tokio::time::timeout(Duration::from_secs(5), observer.resume.wait())
            .await
            .expect("overview reader resumed");
        let snapshot = tokio::time::timeout(Duration::from_secs(5), read)
            .await
            .expect("overview reader completed")
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.activity.user_note_count, 0);
        let next = get_learning_overview(database.pool(), book.id)
            .await
            .unwrap();
        assert_eq!(next.activity.user_note_count, 1);
    });
}

#[test]
fn missing_unready_cross_book_fk_and_provenance_corruption_fail_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("corruption.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let missing = get_learning_overview(database.pool(), Uuid::new_v4())
            .await
            .unwrap_err();
        assert_eq!(missing.code, AppErrorCode::NotFound);

        let unready = insert_unready_book(database.pool()).await;
        let error = get_learning_overview(database.pool(), unready)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::BookNotReady);

        let profile_id = insert_profile(database.pool()).await;
        let target = insert_ready_book(database.pool(), "pdf", '2', true, "target text").await;
        let decoy = insert_ready_book(database.pool(), "pdf", '3', true, "decoy text").await;
        let page = commit_ai_page(database.pool(), target.id, profile_id, 1).await;
        let correction = save_index_correction(
            database.pool(),
            SaveIndexCorrection {
                book_id: target.id,
                page_id: page.page_id,
                target_block_id: page.correctable_block_id,
                target_content_version: 1,
                value_kind: IndexCorrectionValueKind::Text,
                original_value_sha256: correction_value_sha256("provider text to correct"),
                corrected_value: "corrected".to_owned(),
                expected_revision: 0,
            },
        )
        .await
        .unwrap();
        sqlx::query("UPDATE index_corrections SET original_value_sha256 = ? WHERE id = ?")
            .bind("f".repeat(64))
            .bind(correction.id.to_string())
            .execute(database.pool())
            .await
            .unwrap();
        assert_eq!(
            get_learning_overview(database.pool(), target.id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::DatabaseError
        );
        sqlx::query("UPDATE index_corrections SET original_value_sha256 = ? WHERE id = ?")
            .bind(correction_value_sha256("provider text to correct"))
            .bind(correction.id.to_string())
            .execute(database.pool())
            .await
            .unwrap();

        let mut connection = database.pool().acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 99, 'paragraph', 'cross-book', ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(target.id.to_string())
        .bind(decoy.section_id.unwrap().to_string())
        .bind(serde_json::to_string(decoy.locator.as_ref().unwrap()).unwrap())
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        assert_eq!(
            get_learning_overview(database.pool(), target.id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::DatabaseError
        );
    });
}

#[test]
fn duplicate_history_ownership_and_foreign_key_anomalies_fail_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("ownership.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let profile_id = insert_profile(database.pool()).await;
        let book = insert_ready_book(database.pool(), "pdf", '4', true, "history text").await;
        let persisted = persist_history(database.pool(), &book, profile_id).await;
        let row = sqlx::query("SELECT anchor_json, selected_text FROM conversations WHERE id = ?")
            .bind(persisted.conversation_id.to_string())
            .fetch_one(database.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(book.id.to_string())
        .bind(book.section_id.unwrap().to_string())
        .bind(row.try_get::<String, _>("anchor_json").unwrap())
        .bind(row.try_get::<Option<String>, _>("selected_text").unwrap())
        .bind(persisted.conversation_id.to_string())
        .bind(TIMESTAMP)
        .bind(TIMESTAMP)
        .execute(database.pool())
        .await
        .unwrap();
        assert_eq!(
            get_learning_overview(database.pool(), book.id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::DatabaseError
        );

        sqlx::query("DELETE FROM annotations WHERE id != ? AND conversation_id = ?")
            .bind(persisted.annotation_id.to_string())
            .bind(persisted.conversation_id.to_string())
            .execute(database.pool())
            .await
            .unwrap();
        let mut connection = database.pool().acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO messages (id, conversation_id, ordinal, role, action, content, created_at) VALUES (?, ?, 0, 'user', 'ask', 'orphan', ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(Uuid::new_v4().to_string())
        .bind(TIMESTAMP)
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        assert_eq!(
            get_learning_overview(database.pool(), book.id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::DatabaseError
        );
    });
}

#[test]
fn checked_integer_aggregation_rejects_every_overflow() {
    assert!(to_u32_i64(i64::from(u32::MAX) + 1).is_err());
    assert!(to_u32_i64(-1).is_err());
    let mut value = u32::MAX;
    assert!(checked_add_assign(&mut value, 1).is_err());
    assert_eq!(value, u32::MAX);
}

#[derive(Default)]
struct CountingObserver {
    count: AtomicU8,
}

#[async_trait]
impl OverviewReadObserver for CountingObserver {
    async fn after_query(&self, query_count: u8) {
        self.count.store(query_count, Ordering::SeqCst);
    }
}

struct PauseObserver {
    reached: Arc<Barrier>,
    resume: Arc<Barrier>,
}

#[async_trait]
impl OverviewReadObserver for PauseObserver {
    async fn after_query(&self, query_count: u8) {
        if query_count == 6 {
            self.reached.wait().await;
            self.resume.wait().await;
        }
    }
}

struct CommittedPage {
    run_id: Uuid,
    page_id: Uuid,
    correctable_block_id: Uuid,
}

async fn insert_ready_book(
    pool: &SqlitePool,
    format: &str,
    hash_character: char,
    with_section: bool,
    local_text: &str,
) -> TestBook {
    let book_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Private book title', ?, 'private-original-name', ?, 'ready', ?, ?)",
    )
    .bind(book_id.to_string())
    .bind(hash_character.to_string().repeat(64))
    .bind(format)
    .bind(format!("books/{book_id}/original.{format}"))
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    if !with_section {
        return TestBook {
            id: book_id,
            section_id: None,
            locator: None,
        };
    }
    let section_id = stable_section_id(book_id, 0);
    let block_id = stable_block_id(book_id, 0, 0);
    let locator = match format {
        "pdf" => DocumentLocator::pdf(1, 1, None).unwrap(),
        "epub" => DocumentLocator::epub("epubcfi(/6/2)".to_owned(), section_id).unwrap(),
        "docx" => DocumentLocator::docx(block_id, 0, block_id, 1).unwrap(),
        _ => unreachable!(),
    };
    let locator_json = serde_json::to_string(&locator).unwrap();
    sqlx::query(
        "INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Visible synthetic section', ?)",
    )
    .bind(section_id.to_string())
    .bind(book_id.to_string())
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', ?, ?)",
    )
    .bind(block_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(local_text)
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO search_chunks (id, book_id, section_id, ordinal, text, locator_json, token_estimate) VALUES (?, ?, ?, 0, ?, ?, 1)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .bind(local_text)
    .bind(&locator_json)
    .execute(pool)
    .await
    .unwrap();
    TestBook {
        id: book_id,
        section_id: Some(section_id),
        locator: Some(locator),
    }
}

async fn insert_unready_book(pool: &SqlitePool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, title, format, original_filename, import_status, created_at, updated_at) VALUES (?, 'Queued', 'pdf', 'queued.pdf', 'queued', ?, ?)",
    )
    .bind(id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn insert_profile(pool: &SqlitePool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, created_at, updated_at, validated_at) VALUES (?, 'openai', 'PRIVATE_PROVIDER_SENTINEL', 'synthetic-provider-model', 100000, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn commit_ai_page(
    pool: &SqlitePool,
    book_id: Uuid,
    profile_id: Uuid,
    page_number: u32,
) -> CommittedPage {
    let run_id = create_index_run(
        pool,
        CreateIndexRun {
            book_id,
            provider_profile_id: profile_id,
            analysis_schema_version: ANALYSIS_SCHEMA.to_owned(),
            render_version: "synthetic-render-v1".to_owned(),
            parser_version: "synthetic-parser-v1".to_owned(),
        },
    )
    .await
    .unwrap();
    let page = state::queue(pool, run_id, page_number, IndexQualityReason::NoText, None)
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
        page_number,
        review_reason: None,
        blocks: vec![
            ValidatedBlock {
                ordinal: 0,
                kind: IndexPageBlockKind::Paragraph,
                plain_text: Some("provider transcription sentinel".to_owned()),
                latex: None,
                table_cells: None,
                visual_description: Some(DESCRIPTION_SENTINEL.to_owned()),
                bounds: Some(NormalizedRect::new(0.1, 0.1, 0.8, 0.2).unwrap()),
                source: ContentSource::AiTranscribed,
            },
            ValidatedBlock {
                ordinal: 1,
                kind: IndexPageBlockKind::Paragraph,
                plain_text: Some("provider text to correct".to_owned()),
                latex: None,
                table_cells: None,
                visual_description: None,
                bounds: Some(NormalizedRect::new(0.1, 0.4, 0.8, 0.2).unwrap()),
                source: ContentSource::AiTranscribed,
            },
        ],
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
    CommittedPage {
        run_id,
        page_id: page.page_id,
        correctable_block_id: stable_index_page_block_id(
            book_id,
            page_number,
            run_id,
            ANALYSIS_SCHEMA,
            1,
        ),
    }
}

async fn create_private_note(pool: &SqlitePool, book: &TestBook, body: &str) {
    create_note(
        pool,
        CreateNote {
            book_id: book.id,
            section_id: book.section_id.unwrap(),
            anchor: selection_anchor(book, LOCAL_BODY_SENTINEL),
            selected_text: Some(LOCAL_BODY_SENTINEL.to_owned()),
            note_text: body.to_owned(),
        },
    )
    .await
    .unwrap();
}

async fn insert_note_row(pool: &SqlitePool, book: &TestBook, body: &str) {
    let anchor = selection_anchor(book, "snapshot text");
    sqlx::query(
        "INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, note_text, created_at, updated_at) VALUES (?, ?, ?, 'note', ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(book.id.to_string())
    .bind(book.section_id.unwrap().to_string())
    .bind(serde_json::to_string(&anchor).unwrap())
    .bind("snapshot text")
    .bind(body)
    .bind(TIMESTAMP)
    .bind(TIMESTAMP)
    .execute(pool)
    .await
    .unwrap();
}

async fn persist_history(
    pool: &SqlitePool,
    book: &TestBook,
    profile_id: Uuid,
) -> crate::db::conversations::PersistedLearningResult {
    let section_id = book.section_id.unwrap();
    let locator = book.locator.clone().unwrap();
    let citation = Citation::new(
        "TL-C1".to_owned(),
        "Visible synthetic citation".to_owned(),
        book.id,
        Some(section_id),
        locator,
        ContentSource::LocalText,
        CitationReviewStatus::NotRequired,
    )
    .unwrap();
    let repository = LearningRepository::new(pool.clone());
    let persisted = repository
        .persist_new_selection(NewSelectionCompletion {
            book_id: book.id,
            section_id,
            anchor: selection_anchor(book, LOCAL_BODY_SENTINEL),
            selected_text: Some(LOCAL_BODY_SENTINEL.to_owned()),
            action: LearningAction::Ask,
            question: QUESTION_SENTINEL.to_owned(),
            assistant: CompletedAssistantMessage {
                provider_profile_id: profile_id,
                model_id: "synthetic-provider-model".to_owned(),
                answer: format!("{ANSWER_SENTINEL} [TL-C1]"),
                available_citations: vec![citation],
            },
        })
        .await
        .unwrap();
    repository
        .persist_followup(FollowupCompletion {
            book_id: book.id,
            conversation_id: persisted.conversation_id,
            expected_next_ordinal: 2,
            question: format!("{QUESTION_SENTINEL} followup"),
            assistant: CompletedAssistantMessage {
                provider_profile_id: profile_id,
                model_id: "synthetic-provider-model".to_owned(),
                answer: format!("{ANSWER_SENTINEL} followup"),
                available_citations: Vec::new(),
            },
        })
        .await
        .unwrap();
    persisted
}

fn selection_anchor(book: &TestBook, text: &str) -> ContentAnchor {
    ContentAnchor::Text {
        selection: SelectionAnchor {
            locator: book.locator.clone().unwrap(),
            quote: TextQuote::new(text.to_owned(), String::new(), String::new()).unwrap(),
            section_id: book.section_id,
        },
    }
}

fn source(
    overview: &crate::domain::LearningOverview,
    source: LearningOverviewSource,
) -> (u32, u32, u32, bool) {
    let summary = &overview.sources[source.index()];
    assert_eq!(summary.source, source);
    (
        summary.item_count,
        summary.covered_section_count,
        summary.covered_page_count,
        summary.quoteable_as_textbook,
    )
}
