use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tempfile::TempDir;
use textbooklens_lib::{
    db::{
        Database,
        annotations::{MarkerRelocationStatus, list_annotation_markers},
    },
    domain::{
        ContentAnchor, DocumentLocator, NormalizedRect, RegionAnchor, RegionLocator,
        SelectionAnchor, TextQuote,
    },
};
use uuid::Uuid;

const NOW: &str = "2026-08-02T00:00:00Z";

#[test]
fn legacy_text_and_tagged_region_anchors_survive_restart_for_all_formats() {
    let temporary = TempDir::new().expect("temporary database");
    let path = temporary.path().join("library.sqlite3");
    let pdf_book = Uuid::new_v4();
    let epub_book = Uuid::new_v4();
    let docx_book = Uuid::new_v4();
    let pdf_section = Uuid::new_v4();
    let epub_section = Uuid::new_v4();
    let docx_section = Uuid::new_v4();
    let docx_block = Uuid::new_v4();

    let pdf_text = SelectionAnchor {
        locator: DocumentLocator::pdf(
            1,
            1,
            Some(BTreeMap::from([(
                1,
                vec![NormalizedRect::new(0.1, 0.2, 0.3, 0.1).unwrap()],
            )])),
        )
        .unwrap(),
        quote: quote("pdf"),
        section_id: Some(pdf_section),
    };
    let epub_text = SelectionAnchor {
        locator: DocumentLocator::epub("epubcfi(/6/4!/4/2:1)".into(), epub_section).unwrap(),
        quote: quote("epub"),
        section_id: Some(epub_section),
    };
    let docx_text = SelectionAnchor {
        locator: DocumentLocator::docx(docx_block, 1, docx_block, 2).unwrap(),
        quote: quote("🙂"),
        section_id: Some(docx_section),
    };
    let pdf_region = ContentAnchor::Region {
        region: region(RegionLocator::pdf(1).unwrap(), "a"),
    };
    let epub_region = ContentAnchor::Region {
        region: region(
            RegionLocator::epub(epub_section, "epubcfi(/6/2)".into()).unwrap(),
            "b",
        ),
    };
    let docx_region = ContentAnchor::Region {
        region: region(RegionLocator::docx(docx_block), "c"),
    };

    let database = Database::open(&path).expect("database");
    tauri::async_runtime::block_on(async {
        seed_book(database.pool(), pdf_book, pdf_section, "pdf", None).await;
        seed_book(database.pool(), epub_book, epub_section, "epub", None).await;
        seed_book(
            database.pool(),
            docx_book,
            docx_section,
            "docx",
            Some((docx_block, "A🙂B")),
        )
        .await;

        for (book, section, anchor) in [
            (pdf_book, pdf_section, &pdf_text),
            (epub_book, epub_section, &epub_text),
            (docx_book, docx_section, &docx_text),
        ] {
            seed_legacy_note(database.pool(), Uuid::new_v4(), book, section, anchor).await;
        }
        for (book, section, anchor) in [
            (pdf_book, pdf_section, &pdf_region),
            (epub_book, epub_section, &epub_region),
            (docx_book, docx_section, &docx_region),
        ] {
            seed_content_note(database.pool(), Uuid::new_v4(), book, section, anchor).await;
        }
        seed_ai(
            database.pool(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            pdf_book,
            pdf_section,
            &ContentAnchor::from(pdf_text.clone()),
        )
        .await;
        database.pool().close().await;
    });
    drop(database);

    let reopened = Database::open(&path).expect("reopened database");
    tauri::async_runtime::block_on(async {
        let stored: Vec<String> =
            sqlx::query_scalar("SELECT anchor_json FROM annotations ORDER BY id")
                .fetch_all(reopened.pool())
                .await
                .unwrap();
        assert!(stored.iter().any(|json| !json.contains("\"kind\"")));
        assert!(stored.iter().any(|json| json.contains("\"kind\":\"text\"")));
        assert!(
            stored
                .iter()
                .any(|json| json.contains("\"kind\":\"region\""))
        );
        assert!(stored.iter().all(|json| {
            !json.contains("stored_path")
                && !json.contains("screenshot")
                && !json.contains("imageBytes")
        }));

        let pdf = list_annotation_markers(reopened.pool(), pdf_book)
            .await
            .expect("PDF markers");
        let epub = list_annotation_markers(reopened.pool(), epub_book)
            .await
            .expect("EPUB markers");
        let docx = list_annotation_markers(reopened.pool(), docx_book)
            .await
            .expect("DOCX markers");
        let pdf_text = ContentAnchor::from(pdf_text.clone());
        let epub_text = ContentAnchor::from(epub_text.clone());
        let docx_text = ContentAnchor::from(docx_text.clone());
        for (items, expected) in [
            (&pdf, [&pdf_text, &pdf_region]),
            (&epub, [&epub_text, &epub_region]),
            (&docx, [&docx_text, &docx_region]),
        ] {
            assert!(items.iter().all(|item| {
                item.relocation_status == MarkerRelocationStatus::Primary && item.anchor.is_some()
            }));
            for anchor in expected {
                assert!(
                    items
                        .iter()
                        .any(|item| item.anchor.as_ref() == Some(anchor))
                );
            }
        }
    });
}

#[test]
fn unique_bounded_fallback_returns_the_original_anchor_without_rewriting_storage() {
    let temporary = TempDir::new().expect("temporary database");
    let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");
    let book = Uuid::new_v4();
    let section = Uuid::new_v4();
    let block = Uuid::new_v4();
    let fallback = quote("unique fallback text");
    let anchor = ContentAnchor::Region {
        region: RegionAnchor::new(
            RegionLocator::epub(section, "epubcfi(/changed)".into()).unwrap(),
            NormalizedRect::new(0.1, 0.2, 0.3, 0.2).unwrap(),
            sha256_text(&fallback.exact),
            Some(fallback),
        )
        .unwrap(),
    };
    let original_json = serde_json::to_string(&anchor).unwrap();

    tauri::async_runtime::block_on(async {
        seed_book(
            database.pool(),
            book,
            section,
            "epub",
            Some((block, "before unique fallback text after")),
        )
        .await;
        seed_content_note(database.pool(), Uuid::new_v4(), book, section, &anchor).await;
        let markers = list_annotation_markers(database.pool(), book)
            .await
            .expect("fallback markers");
        assert_eq!(markers.len(), 1);
        assert_eq!(
            markers[0].relocation_status,
            MarkerRelocationStatus::Fallback
        );
        assert_eq!(markers[0].anchor.as_ref(), Some(&anchor));
        let stored: String = sqlx::query_scalar("SELECT anchor_json FROM annotations LIMIT 1")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(stored, original_json);
    });
}

#[test]
fn corrupt_unknown_ambiguous_and_format_mismatched_anchors_are_history_only() {
    let temporary = TempDir::new().expect("temporary database");
    let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");
    let book = Uuid::new_v4();
    let section = Uuid::new_v4();
    let block = Uuid::new_v4();
    let epub_book = Uuid::new_v4();
    let epub_section = Uuid::new_v4();
    let epub_block = Uuid::new_v4();
    tauri::async_runtime::block_on(async {
        seed_book(
            database.pool(),
            book,
            section,
            "pdf",
            Some((block, "duplicate duplicate")),
        )
        .await;
        seed_note_json(database.pool(), Uuid::new_v4(), book, section, "{not-json}").await;
        seed_note_json(
            database.pool(),
            Uuid::new_v4(),
            book,
            section,
            r#"{"kind":"future","locator":{"format":"pdf","startPage":1,"endPage":1,"rectsByPage":null},"quote":{"exact":"secret","prefix":"","suffix":""},"sectionId":null,"payload":{"secret":"must-not-leak"}}"#,
        )
        .await;
        let wrong_format = ContentAnchor::Region {
            region: region(
                RegionLocator::epub(section, "epubcfi(/6/2)".into()).unwrap(),
                "d",
            ),
        };
        seed_content_note(
            database.pool(),
            Uuid::new_v4(),
            book,
            section,
            &wrong_format,
        )
        .await;
        seed_book(
            database.pool(),
            epub_book,
            epub_section,
            "epub",
            Some((epub_block, "duplicate duplicate")),
        )
        .await;
        let ambiguous_fallback = ContentAnchor::Region {
            region: RegionAnchor::new(
                RegionLocator::epub(epub_section, "epubcfi(/changed)".into()).unwrap(),
                NormalizedRect::new(0.1, 0.1, 0.2, 0.2).unwrap(),
                sha256_text("duplicate"),
                Some(quote("duplicate")),
            )
            .unwrap(),
        };
        seed_content_note(
            database.pool(),
            Uuid::new_v4(),
            epub_book,
            epub_section,
            &ambiguous_fallback,
        )
        .await;

        let markers = list_annotation_markers(database.pool(), book)
            .await
            .expect("safe marker list");
        assert_eq!(markers.len(), 3);
        assert!(markers.iter().all(|item| {
            item.anchor.is_none() && item.relocation_status == MarkerRelocationStatus::Unresolved
        }));
        let ambiguous = list_annotation_markers(database.pool(), epub_book)
            .await
            .expect("ambiguous fallback markers");
        assert_eq!(ambiguous.len(), 1);
        assert!(ambiguous[0].anchor.is_none());
        assert_eq!(
            ambiguous[0].relocation_status,
            MarkerRelocationStatus::Unresolved
        );
        let serialized = serde_json::to_string(&markers).unwrap();
        for forbidden in ["stored_path", "not-json", "must-not-leak"] {
            assert!(!serialized.contains(forbidden));
        }
    });
}

fn quote(exact: &str) -> TextQuote {
    TextQuote::new(exact.into(), "prefix".into(), "suffix".into()).unwrap()
}

fn region(locator: RegionLocator, hash_byte: &str) -> RegionAnchor {
    RegionAnchor::new(
        locator,
        NormalizedRect::new(0.1, 0.2, 0.3, 0.2).unwrap(),
        hash_byte.repeat(64),
        None,
    )
    .unwrap()
}

fn sha256_text(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn seed_book(
    pool: &SqlitePool,
    book: Uuid,
    section: Uuid,
    format: &str,
    block: Option<(Uuid, &str)>,
) {
    sqlx::query("INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, 'Fixture', ?, ?, ?, 'ready', ?, ?)")
        .bind(book.to_string()).bind(format!("hash-{book}")).bind(format).bind(format!("textbook.{format}")).bind(format!("books/{book}/source.bin")).bind(NOW).bind(NOW).execute(pool).await.unwrap();
    let locator = match format {
        "pdf" => DocumentLocator::pdf(1, 1, None).unwrap(),
        "epub" => DocumentLocator::epub("epubcfi(/6/2)".into(), section).unwrap(),
        "docx" => {
            let id = block.expect("DOCX fixture block").0;
            DocumentLocator::docx(id, 0, id, 0).unwrap()
        }
        _ => unreachable!(),
    };
    sqlx::query("INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', ?)").bind(section.to_string()).bind(book.to_string()).bind(serde_json::to_string(&locator).unwrap()).execute(pool).await.unwrap();
    if let Some((block_id, text)) = block {
        sqlx::query("INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', ?, ?)").bind(block_id.to_string()).bind(book.to_string()).bind(section.to_string()).bind(text).bind(serde_json::to_string(&locator).unwrap()).execute(pool).await.unwrap();
    }
}

async fn seed_legacy_note(
    pool: &SqlitePool,
    id: Uuid,
    book: Uuid,
    section: Uuid,
    anchor: &SelectionAnchor,
) {
    seed_note_json(
        pool,
        id,
        book,
        section,
        &serde_json::to_string(anchor).unwrap(),
    )
    .await;
}

async fn seed_content_note(
    pool: &SqlitePool,
    id: Uuid,
    book: Uuid,
    section: Uuid,
    anchor: &ContentAnchor,
) {
    seed_note_json(
        pool,
        id,
        book,
        section,
        &serde_json::to_string(anchor).unwrap(),
    )
    .await;
}

async fn seed_note_json(pool: &SqlitePool, id: Uuid, book: Uuid, section: Uuid, json: &str) {
    sqlx::query("INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, note_text, created_at, updated_at) VALUES (?, ?, ?, 'note', ?, 'selection', 'note', ?, ?)")
        .bind(id.to_string()).bind(book.to_string()).bind(section.to_string()).bind(json).bind(NOW).bind(NOW).execute(pool).await.unwrap();
}

async fn seed_ai(
    pool: &SqlitePool,
    annotation: Uuid,
    conversation: Uuid,
    book: Uuid,
    section: Uuid,
    anchor: &ContentAnchor,
) {
    let json = serde_json::to_string(anchor).unwrap();
    sqlx::query("INSERT INTO conversations (id, book_id, section_id, scope, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', ?, 'selection', ?, ?)").bind(conversation.to_string()).bind(book.to_string()).bind(section.to_string()).bind(&json).bind(NOW).bind(NOW).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', ?, 'selection', ?, ?, ?)").bind(annotation.to_string()).bind(book.to_string()).bind(section.to_string()).bind(json).bind(conversation.to_string()).bind(NOW).bind(NOW).execute(pool).await.unwrap();
}
