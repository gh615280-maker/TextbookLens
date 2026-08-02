use std::collections::BTreeMap;

use sqlx::SqlitePool;
use tempfile::TempDir;
use textbooklens_lib::{
    db::{
        Database,
        annotations::{MarkerRelocationStatus, list_annotation_markers},
    },
    domain::{DocumentLocator, NormalizedRect, SelectionAnchor, TextQuote},
};
use uuid::Uuid;

const NOW: &str = "2026-08-02T00:00:00Z";

#[test]
fn annotation_anchors_survive_database_restart_for_all_reader_formats() {
    let temporary = TempDir::new().expect("temporary database");
    let path = temporary.path().join("library.sqlite3");
    let pdf_book = Uuid::new_v4();
    let epub_book = Uuid::new_v4();
    let docx_book = Uuid::new_v4();
    let pdf_section = Uuid::new_v4();
    let epub_section = Uuid::new_v4();
    let docx_section = Uuid::new_v4();
    let docx_block = Uuid::new_v4();
    let pdf_anchor = SelectionAnchor {
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
    let epub_anchor = SelectionAnchor {
        locator: DocumentLocator::epub("epubcfi(/6/4!/4/2:1)".into(), epub_section).unwrap(),
        quote: quote("epub"),
        section_id: Some(epub_section),
    };
    let docx_anchor = SelectionAnchor {
        locator: DocumentLocator::docx(docx_block, 1, docx_block, 2).unwrap(),
        quote: quote("😀"),
        section_id: Some(docx_section),
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
            Some((docx_block, "A😀B")),
        )
        .await;
        seed_note(
            database.pool(),
            Uuid::new_v4(),
            pdf_book,
            pdf_section,
            &pdf_anchor,
        )
        .await;
        seed_note(
            database.pool(),
            Uuid::new_v4(),
            epub_book,
            epub_section,
            &epub_anchor,
        )
        .await;
        seed_note(
            database.pool(),
            Uuid::new_v4(),
            docx_book,
            docx_section,
            &docx_anchor,
        )
        .await;
        seed_ai(
            database.pool(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            pdf_book,
            pdf_section,
            &pdf_anchor,
        )
        .await;
        database.pool().close().await;
    });
    drop(database);

    let reopened = Database::open(&path).expect("reopened database");
    tauri::async_runtime::block_on(async {
        let stored_pdf: String = sqlx::query_scalar(
            "SELECT anchor_json FROM annotations WHERE book_id = ? ORDER BY created_at, id LIMIT 1",
        )
        .bind(pdf_book.to_string())
        .fetch_one(reopened.pool())
        .await
        .unwrap();
        assert_eq!(
            serde_json::from_str::<SelectionAnchor>(&stored_pdf).unwrap(),
            pdf_anchor
        );
        let section_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sections WHERE id = ? AND book_id = ?)",
        )
        .bind(pdf_section.to_string())
        .bind(pdf_book.to_string())
        .fetch_one(reopened.pool())
        .await
        .unwrap();
        assert!(section_exists);
        let pdf = list_annotation_markers(reopened.pool(), pdf_book)
            .await
            .expect("PDF markers");
        let epub = list_annotation_markers(reopened.pool(), epub_book)
            .await
            .expect("EPUB markers");
        let docx = list_annotation_markers(reopened.pool(), docx_book)
            .await
            .expect("DOCX markers");
        assert_eq!(pdf.len(), 2);
        assert_eq!(pdf[0].anchor.as_ref(), Some(&pdf_anchor));
        assert_eq!(pdf[1].anchor.as_ref(), Some(&pdf_anchor));
        assert_eq!(epub[0].anchor.as_ref(), Some(&epub_anchor));
        assert_eq!(docx[0].anchor.as_ref(), Some(&docx_anchor));
        assert!(
            pdf.iter()
                .chain(epub.iter())
                .chain(docx.iter())
                .all(|item| item.relocation_status == MarkerRelocationStatus::Primary)
        );
    });
}

#[test]
fn corrupt_or_mismatched_anchors_are_history_only_without_panics_or_internal_data() {
    let temporary = TempDir::new().expect("temporary database");
    let database = Database::open(temporary.path().join("library.sqlite3")).expect("database");
    let book = Uuid::new_v4();
    let section = Uuid::new_v4();
    let corrupt = Uuid::new_v4();
    let mismatch = Uuid::new_v4();
    tauri::async_runtime::block_on(async {
        seed_book(database.pool(), book, section, "pdf", None).await;
        seed_note_json(database.pool(), corrupt, book, section, "{not-json}").await;
        let wrong = SelectionAnchor {
            locator: DocumentLocator::epub("epubcfi(/6/2)".into(), section).unwrap(),
            quote: quote("wrong"),
            section_id: Some(section),
        };
        seed_note(database.pool(), mismatch, book, section, &wrong).await;
        let markers = list_annotation_markers(database.pool(), book)
            .await
            .expect("safe marker list");
        assert_eq!(markers.len(), 2);
        assert!(markers.iter().all(|item| item.anchor.is_none()
            && item.relocation_status == MarkerRelocationStatus::Unresolved));
        let serialized = serde_json::to_string(&markers).unwrap();
        assert!(!serialized.contains("stored_path") && !serialized.contains("not-json"));
    });
}

fn quote(exact: &str) -> TextQuote {
    TextQuote::new(exact.into(), "prefix".into(), "suffix".into()).unwrap()
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
            let id = block.unwrap().0;
            DocumentLocator::docx(id, 0, id, 0).unwrap()
        }
        _ => unreachable!(),
    };
    sqlx::query("INSERT INTO sections (id, book_id, ordinal, title, locator_json) VALUES (?, ?, 0, 'Section', ?)").bind(section.to_string()).bind(book.to_string()).bind(serde_json::to_string(&locator).unwrap()).execute(pool).await.unwrap();
    if let Some((block_id, text)) = block {
        sqlx::query("INSERT INTO blocks (id, book_id, section_id, ordinal, kind, plain_text, locator_json) VALUES (?, ?, ?, 0, 'paragraph', ?, ?)").bind(block_id.to_string()).bind(book.to_string()).bind(section.to_string()).bind(text).bind(serde_json::to_string(&locator).unwrap()).execute(pool).await.unwrap();
    }
}

async fn seed_note(
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
async fn seed_note_json(pool: &SqlitePool, id: Uuid, book: Uuid, section: Uuid, anchor_json: &str) {
    sqlx::query("INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, note_text, created_at, updated_at) VALUES (?, ?, ?, 'note', ?, 'selection', 'note', ?, ?)")
        .bind(id.to_string()).bind(book.to_string()).bind(section.to_string()).bind(anchor_json).bind(NOW).bind(NOW).execute(pool).await.unwrap();
}
async fn seed_ai(
    pool: &SqlitePool,
    annotation: Uuid,
    conversation: Uuid,
    book: Uuid,
    section: Uuid,
    anchor: &SelectionAnchor,
) {
    let json = serde_json::to_string(anchor).unwrap();
    sqlx::query("INSERT INTO conversations (id, book_id, section_id, scope, anchor_json, selected_text, created_at, updated_at) VALUES (?, ?, ?, 'selection', ?, 'selection', ?, ?)").bind(conversation.to_string()).bind(book.to_string()).bind(section.to_string()).bind(&json).bind(NOW).bind(NOW).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO annotations (id, book_id, section_id, kind, anchor_json, selected_text, conversation_id, created_at, updated_at) VALUES (?, ?, ?, 'ai_conversation', ?, 'selection', ?, ?, ?)").bind(annotation.to_string()).bind(book.to_string()).bind(section.to_string()).bind(json).bind(conversation.to_string()).bind(NOW).bind(NOW).execute(pool).await.unwrap();
}
