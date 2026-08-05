use std::collections::BTreeMap;

use sqlx::SqlitePool;

use crate::{
    db::{
        Database,
        annotations::{MarkerRelocationStatus, list_annotation_markers},
    },
    domain::{
        ContentAnchor, DocumentLocator, NormalizedRect, RegionAnchor, RegionLocator,
        SelectionAnchor, TextQuote,
    },
    errors::AppErrorCode,
};

use super::*;

const NOW: &str = "2026-08-05T00:00:00.000Z";
const BODY_SENTINEL: &str = "NOTE_BODY_PRIVATE_SENTINEL";
const HASH_SENTINEL: &str = "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd";

struct Fixture {
    _temporary: tempfile::TempDir,
    database: Database,
    book_id: Uuid,
    decoy_book_id: Uuid,
    section_id: Uuid,
    decoy_section_id: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let database = Database::open(temporary.path().join("notes.sqlite3")).unwrap();
        let book_id = Uuid::new_v4();
        let decoy_book_id = Uuid::new_v4();
        let section_id = Uuid::new_v4();
        let decoy_section_id = Uuid::new_v4();
        tauri::async_runtime::block_on(async {
            seed_pdf(database.pool(), book_id, section_id).await;
            seed_pdf(database.pool(), decoy_book_id, decoy_section_id).await;
        });
        Self {
            _temporary: temporary,
            database,
            book_id,
            decoy_book_id,
            section_id,
            decoy_section_id,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.database.pool()
    }

    fn anchor(&self) -> ContentAnchor {
        text_anchor(self.section_id, "selected text")
    }

    fn create(&self, note_text: &str) -> CreateNote {
        CreateNote {
            book_id: self.book_id,
            section_id: self.section_id,
            anchor: self.anchor(),
            selected_text: Some("selected text".into()),
            note_text: note_text.into(),
        }
    }
}

#[test]
fn create_update_list_get_and_delete_are_normalized_exact_revision_transactions() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        let created = create_note(fixture.pool(), fixture.create("  alpha\r\n beta\r  "))
            .await
            .unwrap();
        assert_eq!(created.note_text, "  alpha\n beta\n  ");
        assert_eq!(created.revision, 1);
        assert!(created.created_at.to_rfc3339().ends_with("+00:00"));
        assert_eq!(created.created_at, created.updated_at);

        let listed = list_notes(fixture.pool(), fixture.book_id).await.unwrap();
        assert_eq!(listed, vec![created.clone()]);
        assert_eq!(
            get_note(fixture.pool(), fixture.book_id, created.id)
                .await
                .unwrap(),
            created
        );

        let stale = update_note(
            fixture.pool(),
            UpdateNote {
                book_id: fixture.book_id,
                note_id: created.id,
                expected_revision: 2,
                note_text: "stale".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(stale.code, AppErrorCode::RequestConflict);

        let cross_book = update_note(
            fixture.pool(),
            UpdateNote {
                book_id: fixture.decoy_book_id,
                note_id: created.id,
                expected_revision: 1,
                note_text: "cross book".into(),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(cross_book.code, AppErrorCode::NotFound);

        let updated = update_note(
            fixture.pool(),
            UpdateNote {
                book_id: fixture.book_id,
                note_id: created.id,
                expected_revision: 1,
                note_text: "updated".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.revision, 2);
        assert_eq!(updated.note_text, "updated");
        assert_eq!(updated.anchor, created.anchor);
        assert_eq!(updated.selected_text, created.selected_text);

        let stale_delete = delete_note(
            fixture.pool(),
            DeleteNote {
                book_id: fixture.book_id,
                note_id: created.id,
                expected_revision: 1,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(stale_delete.code, AppErrorCode::RequestConflict);
        delete_note(
            fixture.pool(),
            DeleteNote {
                book_id: fixture.book_id,
                note_id: created.id,
                expected_revision: 2,
            },
        )
        .await
        .unwrap();
        assert!(
            list_notes(fixture.pool(), fixture.book_id)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            get_note(fixture.pool(), fixture.book_id, created.id)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::NotFound
        );
    });
}

#[test]
fn note_bounds_anchor_ownership_and_ready_book_are_rejected_without_partial_rows() {
    let fixture = Fixture::new();
    tauri::async_runtime::block_on(async {
        for body in ["", " \n\t "] {
            assert_eq!(
                create_note(fixture.pool(), fixture.create(body))
                    .await
                    .unwrap_err()
                    .code,
                AppErrorCode::InvalidInput
            );
        }
        assert_eq!(
            create_note(
                fixture.pool(),
                fixture.create(&"x".repeat(MAX_NOTE_CODE_POINTS + 1))
            )
            .await
            .unwrap_err()
            .code,
            AppErrorCode::InvalidInput
        );

        let mut mismatch = fixture.create("body");
        mismatch.selected_text = Some("changed".into());
        assert_eq!(
            create_note(fixture.pool(), mismatch)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput
        );

        let mut foreign_section = fixture.create("body");
        foreign_section.section_id = fixture.decoy_section_id;
        assert_eq!(
            create_note(fixture.pool(), foreign_section)
                .await
                .unwrap_err()
                .code,
            AppErrorCode::AnchorNotFound
        );

        sqlx::query("UPDATE books SET import_status = 'parsing' WHERE id = ?")
            .bind(fixture.book_id.to_string())
            .execute(fixture.pool())
            .await
            .unwrap();
        assert_eq!(
            create_note(fixture.pool(), fixture.create("body"))
                .await
                .unwrap_err()
                .code,
            AppErrorCode::BookNotReady
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM annotations")
            .fetch_one(fixture.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    });
}

#[test]
fn all_format_region_notes_restart_as_markers_without_capture_or_path_storage() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("restart.sqlite3");
    let database = Database::open(&path).unwrap();
    let pdf_book = Uuid::new_v4();
    let epub_book = Uuid::new_v4();
    let docx_book = Uuid::new_v4();
    let pdf_section = Uuid::new_v4();
    let epub_section = Uuid::new_v4();
    let docx_section = Uuid::new_v4();
    let docx_block = Uuid::new_v4();
    tauri::async_runtime::block_on(async {
        seed_pdf(database.pool(), pdf_book, pdf_section).await;
        seed_epub(database.pool(), epub_book, epub_section).await;
        seed_docx(database.pool(), docx_book, docx_section, docx_block).await;
        for (book_id, section_id, anchor) in [
            (
                pdf_book,
                pdf_section,
                region_anchor(RegionLocator::pdf(1).unwrap()),
            ),
            (
                epub_book,
                epub_section,
                region_anchor(RegionLocator::epub(epub_section, "epubcfi(/6/2)".into()).unwrap()),
            ),
            (
                docx_book,
                docx_section,
                region_anchor(RegionLocator::docx(docx_block)),
            ),
        ] {
            create_note(
                database.pool(),
                CreateNote {
                    book_id,
                    section_id,
                    anchor,
                    selected_text: None,
                    note_text: "local region note".into(),
                },
            )
            .await
            .unwrap();
        }
        let stored: Vec<String> = sqlx::query_scalar("SELECT anchor_json FROM annotations")
            .fetch_all(database.pool())
            .await
            .unwrap();
        assert!(stored.iter().all(|json| {
            json.contains("\"kind\":\"region\"")
                && !json.contains("screenshot")
                && !json.contains("image")
                && !json.contains("path")
                && !json.contains("capture")
        }));
        database.pool().close().await;
    });
    drop(database);

    let reopened = Database::open(&path).unwrap();
    tauri::async_runtime::block_on(async {
        for book_id in [pdf_book, epub_book, docx_book] {
            let markers = list_annotation_markers(reopened.pool(), book_id)
                .await
                .unwrap();
            assert_eq!(markers.len(), 1);
            assert_eq!(
                markers[0].relocation_status,
                MarkerRelocationStatus::Primary
            );
            assert!(markers[0].anchor.is_some());
        }
    });
}

#[test]
fn production_graph_and_debug_surfaces_are_provider_credential_network_and_body_free() {
    let fixture = Fixture::new();
    let source = include_str!("notes.rs");
    let commands = include_str!("../commands/annotations.rs");
    for forbidden in [
        "ProviderRuntime",
        "AiProvider",
        "CredentialStore",
        "reqwest",
        "TcpStream",
        "provider_failure",
    ] {
        assert!(!source.contains(forbidden));
        assert!(!commands.contains(forbidden));
    }

    tauri::async_runtime::block_on(async {
        let before = protected_table_counts(fixture.pool()).await;
        let created = create_note(fixture.pool(), fixture.create(BODY_SENTINEL))
            .await
            .unwrap();
        let after_create = protected_table_counts(fixture.pool()).await;
        assert_eq!(before, after_create);
        update_note(
            fixture.pool(),
            UpdateNote {
                book_id: fixture.book_id,
                note_id: created.id,
                expected_revision: 1,
                note_text: "replacement private body".into(),
            },
        )
        .await
        .unwrap();
        let _ = list_notes(fixture.pool(), fixture.book_id).await.unwrap();
        delete_note(
            fixture.pool(),
            DeleteNote {
                book_id: fixture.book_id,
                note_id: created.id,
                expected_revision: 2,
            },
        )
        .await
        .unwrap();
        assert_eq!(before, protected_table_counts(fixture.pool()).await);

        let request_debug = format!("{:?}", fixture.create(BODY_SENTINEL));
        let dto_debug = format!("{created:?}");
        for debug in [request_debug, dto_debug] {
            assert!(!debug.contains(BODY_SENTINEL));
            assert!(!debug.contains(HASH_SENTINEL));
            assert!(!debug.to_lowercase().contains("path"));
        }
    });
}

async fn protected_table_counts(pool: &SqlitePool) -> (i64, i64, i64, i64) {
    (
        sqlx::query_scalar("SELECT COUNT(*) FROM conversations")
            .fetch_one(pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT COUNT(*) FROM messages")
            .fetch_one(pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_profiles")
            .fetch_one(pool)
            .await
            .unwrap(),
        sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources")
            .fetch_one(pool)
            .await
            .unwrap(),
    )
}

fn text_anchor(section_id: Uuid, exact: &str) -> ContentAnchor {
    ContentAnchor::Text {
        selection: SelectionAnchor {
            locator: DocumentLocator::pdf(
                1,
                1,
                Some(BTreeMap::from([(
                    1,
                    vec![NormalizedRect::new(0.1, 0.2, 0.3, 0.1).unwrap()],
                )])),
            )
            .unwrap(),
            quote: TextQuote::new(exact.into(), "prefix".into(), "suffix".into()).unwrap(),
            section_id: Some(section_id),
        },
    }
}

fn region_anchor(locator: RegionLocator) -> ContentAnchor {
    ContentAnchor::Region {
        region: RegionAnchor::new(
            locator,
            NormalizedRect::new(0.1, 0.2, 0.3, 0.2).unwrap(),
            HASH_SENTINEL.into(),
            None,
        )
        .unwrap(),
    }
}

async fn seed_pdf(pool: &SqlitePool, book_id: Uuid, section_id: Uuid) {
    seed_book(
        pool,
        book_id,
        section_id,
        "pdf",
        DocumentLocator::pdf(1, 1, None).unwrap(),
        None,
    )
    .await;
}

async fn seed_epub(pool: &SqlitePool, book_id: Uuid, section_id: Uuid) {
    seed_book(
        pool,
        book_id,
        section_id,
        "epub",
        DocumentLocator::epub("epubcfi(/6/2)".into(), section_id).unwrap(),
        None,
    )
    .await;
}

async fn seed_docx(pool: &SqlitePool, book_id: Uuid, section_id: Uuid, block_id: Uuid) {
    let locator = DocumentLocator::docx(block_id, 0, block_id, 0).unwrap();
    seed_book(
        pool,
        book_id,
        section_id,
        "docx",
        locator,
        Some((block_id, "docx block")),
    )
    .await;
}

async fn seed_book(
    pool: &SqlitePool,
    book_id: Uuid,
    section_id: Uuid,
    format: &str,
    locator: DocumentLocator,
    block: Option<(Uuid, &str)>,
) {
    sqlx::query("INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Fixture', ?, ?, ?, 'ready', ?, ?)")
        .bind(book_id.to_string())
        .bind(format!("hash-{book_id}"))
        .bind(format)
        .bind(format!("fixture.{format}"))
        .bind(format!("books/{book_id}/source"))
        .bind(NOW)
        .bind(NOW)
        .execute(pool)
        .await
        .unwrap();
    let locator_json = serde_json::to_string(&locator).unwrap();
    sqlx::query("INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', ?)")
        .bind(section_id.to_string())
        .bind(book_id.to_string())
        .bind(&locator_json)
        .execute(pool)
        .await
        .unwrap();
    if let Some((block_id, text)) = block {
        sqlx::query("INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', ?, ?)")
            .bind(block_id.to_string())
            .bind(book_id.to_string())
            .bind(section_id.to_string())
            .bind(text)
            .bind(locator_json)
            .execute(pool)
            .await
            .unwrap();
    }
}
