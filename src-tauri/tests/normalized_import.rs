use std::{collections::BTreeMap, fs, sync::Arc};

use sqlx::Row;
use textbooklens_lib::{
    app_state::AppPaths,
    db::Database,
    document_repository,
    documents::{
        import::{
            BeginImportOutcome, BeginImportRequest, ImportEvent, ImportService, ParsedBookMetadata,
        },
        storage::noop_progress,
    },
    domain::{
        BlockKind, BookSummary, DocumentLocator, NormalizedBlockInput, NormalizedRect,
        NormalizedSectionInput, stable_block_id, stable_section_id,
    },
    errors::AppErrorCode,
    retrieval::search::search_book,
};
use uuid::Uuid;

async fn test_service() -> (tempfile::TempDir, Database, ImportService) {
    tokio::task::spawn_blocking(|| {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("app-data");
        let paths = AppPaths {
            books: root.join("books"),
            cache: root.join("cache"),
            logs: root.join("logs"),
            database: root.join("library.sqlite3"),
            root,
        };
        for path in [&paths.root, &paths.books, &paths.cache, &paths.logs] {
            fs::create_dir_all(path).unwrap();
        }
        let database = Database::open(&paths.database).unwrap();
        let service = ImportService::new(database.pool().clone(), paths);
        (temp, database, service)
    })
    .await
    .unwrap()
}

async fn begin_pdf(
    temp: &tempfile::TempDir,
    service: &ImportService,
    name: &str,
    unique_text: &str,
) -> BookSummary {
    let path = temp.path().join(name);
    fs::write(&path, format!("%PDF-1.7\n{unique_text}\n%%EOF")).unwrap();
    let outcome = service
        .begin_import(
            BeginImportRequest::new(path.to_string_lossy().into_owned()),
            Arc::new(|_: ImportEvent| {}),
        )
        .await
        .unwrap();
    let book = match outcome {
        BeginImportOutcome::Created { book } => book,
        BeginImportOutcome::Duplicate { .. } => panic!("test source must be unique"),
    };
    service
        .begin_parse(
            book.id,
            ParsedBookMetadata {
                title: name.trim_end_matches(".pdf").to_owned(),
                author: Some("TextbookLens".to_owned()),
                language: Some("zh-CN".to_owned()),
            },
        )
        .await
        .unwrap();
    book
}

fn pdf_section(
    book_id: Uuid,
    section_ordinal: u32,
    title: &str,
    texts: &[String],
) -> NormalizedSectionInput {
    let section_id = stable_section_id(book_id, section_ordinal);
    let page = section_ordinal + 1;
    NormalizedSectionInput {
        id: section_id,
        parent_id: None,
        ordinal: section_ordinal,
        title: title.to_owned(),
        locator: DocumentLocator::pdf(page, page, None).unwrap(),
        blocks: texts
            .iter()
            .enumerate()
            .map(|(ordinal, text)| NormalizedBlockInput {
                id: stable_block_id(book_id, section_ordinal, ordinal as u32),
                ordinal: ordinal as u32,
                kind: if ordinal == 0 {
                    BlockKind::Heading
                } else {
                    BlockKind::Paragraph
                },
                plain_text: text.clone(),
                locator: DocumentLocator::pdf(page, page, None).unwrap(),
            })
            .collect(),
    }
}

fn pdf_batch_with_serialized_size(
    book_id: Uuid,
    target_bytes: usize,
) -> Vec<NormalizedSectionInput> {
    let mut batch = vec![pdf_section(book_id, 0, "字节边界", &[String::new()])];
    let overhead = serde_json::to_vec(&batch).unwrap().len();
    let text_bytes = target_bytes.checked_sub(overhead).unwrap();
    let emoji_count = text_bytes / 4;
    let ascii_count = text_bytes % 4;
    batch[0].blocks[0].plain_text =
        format!("{}{}", "😀".repeat(emoji_count), "a".repeat(ascii_count));
    assert_eq!(serde_json::to_vec(&batch).unwrap().len(), target_bytes);
    batch
}

async fn insert_parsing_book(database: &Database, format: &str, label: &str) -> Uuid {
    let book_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO books (id, sha256, title, format, original_filename, stored_path, import_status, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'parsing', '2026-08-02T00:00:00Z', '2026-08-02T00:00:00Z')",
    )
    .bind(book_id.to_string())
    .bind(format!("{book_id:0>64}"))
    .bind(label)
    .bind(format)
    .bind(format!("{label}.{format}"))
    .bind(format!("books/{book_id}/original.{format}"))
    .execute(database.pool())
    .await
    .unwrap();
    book_id
}

fn docx_section(book_id: Uuid) -> NormalizedSectionInput {
    let section_id = stable_section_id(book_id, 0);
    let first_id = stable_block_id(book_id, 0, 0);
    let second_id = stable_block_id(book_id, 0, 1);
    NormalizedSectionInput {
        id: section_id,
        parent_id: None,
        ordinal: 0,
        title: "DOCX 章节".to_owned(),
        locator: DocumentLocator::Docx {
            start_block_id: first_id,
            start_offset: 0,
            end_block_id: second_id,
            end_offset: 2,
        },
        blocks: vec![
            NormalizedBlockInput {
                id: first_id,
                ordinal: 0,
                kind: BlockKind::Paragraph,
                plain_text: "😀ab".to_owned(),
                locator: DocumentLocator::Docx {
                    start_block_id: first_id,
                    start_offset: 0,
                    end_block_id: first_id,
                    end_offset: 3,
                },
            },
            NormalizedBlockInput {
                id: second_id,
                ordinal: 1,
                kind: BlockKind::Paragraph,
                plain_text: "终点".to_owned(),
                locator: DocumentLocator::Docx {
                    start_block_id: second_id,
                    start_offset: 0,
                    end_block_id: second_id,
                    end_offset: 2,
                },
            },
        ],
    }
}

async fn counts(database: &Database, book_id: Uuid) -> (i64, i64, i64, i64) {
    let sections = sqlx::query_scalar("SELECT COUNT(*) FROM sections WHERE book_id = ?")
        .bind(book_id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    let blocks = sqlx::query_scalar("SELECT COUNT(*) FROM blocks WHERE book_id = ?")
        .bind(book_id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    let chunks = sqlx::query_scalar("SELECT COUNT(*) FROM search_chunks WHERE book_id = ?")
        .bind(book_id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    let fts = sqlx::query_scalar(
        "SELECT COUNT(*) FROM search_chunks_fts JOIN search_chunks ON search_chunks.rowid = search_chunks_fts.rowid WHERE search_chunks.book_id = ?",
    )
    .bind(book_id.to_string())
    .fetch_one(database.pool())
    .await
    .unwrap();
    (sections, blocks, chunks, fts)
}

async fn fts_match_count(database: &Database, query: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM search_chunks_fts WHERE search_chunks_fts MATCH ?")
        .bind(format!("\"{}\"", query.replace('"', "\"\"")))
        .fetch_one(database.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn image_only_pdf_pages_finalize_without_local_text_chunks() {
    let (temp, database, service) = test_service().await;
    let book = begin_pdf(&temp, &service, "image-only.pdf", "image-only-source").await;
    service
        .append_parsed_sections(
            book.id,
            vec![
                pdf_section(book.id, 0, "Page 1", &[]),
                pdf_section(book.id, 1, "Page 2", &[]),
            ],
        )
        .await
        .unwrap();

    let ready = service
        .finalize_import(book.id, noop_progress())
        .await
        .unwrap();

    assert_eq!(
        ready.import_status,
        textbooklens_lib::domain::ImportStatus::Ready
    );
    assert_eq!(counts(&database, book.id).await, (2, 0, 0, 0));
}

#[tokio::test]
async fn batches_finalize_atomically_and_search_never_crosses_books() {
    let (temp, database, service) = test_service().await;
    let first = begin_pdf(&temp, &service, "first.pdf", "first-index-source").await;
    let chapter_one = pdf_section(
        first.id,
        0,
        "第一章",
        &[
            "第一章 线性代数".to_owned(),
            format!(
                "{} 能量 甲乙 100%_完成 反斜杠\\标记 a\"b foo OR bar (abc) *** 甲书唯一长标记 删除同步标记。",
                "矩阵向量与线性代数。".repeat(90)
            ),
        ],
    );
    let chapter_two = pdf_section(
        first.id,
        1,
        "第二章",
        &[format!("{}。", "微积分与极限".repeat(140))],
    );
    service
        .append_parsed_sections(first.id, vec![chapter_one])
        .await
        .unwrap();
    service
        .append_parsed_sections(first.id, vec![chapter_two])
        .await
        .unwrap();
    let ready = service
        .finalize_import(first.id, noop_progress())
        .await
        .unwrap();
    assert_eq!(
        ready.import_status,
        textbooklens_lib::domain::ImportStatus::Ready
    );
    let persisted = counts(&database, first.id).await;
    assert!(persisted.0 > 0 && persisted.1 > 0 && persisted.2 > 0);
    assert_eq!(persisted.2, persisted.3);

    let second = begin_pdf(&temp, &service, "second.pdf", "second-index-source").await;
    service
        .append_parsed_sections(
            second.id,
            vec![pdf_section(
                second.id,
                0,
                "隔离章节",
                &[format!(
                    "{} 第二本泄漏标记 乙书唯一长标记 丙丁。",
                    "线性代数 能量".repeat(150)
                )],
            )],
        )
        .await
        .unwrap();
    service
        .finalize_import(second.id, noop_progress())
        .await
        .unwrap();

    let fts_hits = search_book(database.pool(), first.id, "线性代", 10)
        .await
        .unwrap();
    assert!(!fts_hits.is_empty());
    assert!(
        fts_hits
            .iter()
            .all(|hit| !hit.snippet.contains("第二本泄漏标记"))
    );
    let short_hits = search_book(database.pool(), first.id, "能量", 10)
        .await
        .unwrap();
    assert!(!short_hits.is_empty());
    assert!(
        short_hits
            .iter()
            .all(|hit| !hit.snippet.contains("第二本泄漏标记"))
    );
    let escaped_like = search_book(database.pool(), first.id, "%_", 10)
        .await
        .unwrap();
    assert_eq!(
        escaped_like.len(),
        1,
        "LIKE wildcards must be treated literally"
    );
    assert!(
        search_book(database.pool(), first.id, "乙书唯一长标记", 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        search_book(database.pool(), second.id, "甲书唯一长标记", 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        search_book(database.pool(), first.id, "丙丁", 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        search_book(database.pool(), second.id, "甲乙", 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        search_book(database.pool(), first.id, "能量", 0)
            .await
            .unwrap()
            .len(),
        1,
        "limit is clamped to at least one"
    );
    assert_eq!(fts_match_count(&database, "删除同步标记").await, 1);
    for literal_query in ["a\"b", "foo OR bar", "(abc)", "***"] {
        let first_result = search_book(database.pool(), first.id, literal_query, 100).await;
        assert!(first_result.is_ok(), "literal FTS query {literal_query:?}");
        assert!(
            search_book(database.pool(), second.id, literal_query, 100)
                .await
                .unwrap()
                .is_empty(),
            "literal FTS query crossed books: {literal_query:?}"
        );
    }
    for literal_query in ["%", "_", "\\", "%_"] {
        assert!(
            !search_book(database.pool(), first.id, literal_query, 100)
                .await
                .unwrap()
                .is_empty(),
            "literal LIKE query should match: {literal_query:?}"
        );
        assert!(
            search_book(database.pool(), second.id, literal_query, 100)
                .await
                .unwrap()
                .is_empty(),
            "literal LIKE query crossed books: {literal_query:?}"
        );
    }
    assert_eq!(
        search_book(database.pool(), first.id, "   ", 10)
            .await
            .unwrap_err()
            .code,
        AppErrorCode::InvalidInput
    );
    sqlx::query(
        "UPDATE search_chunks SET text = 'FTS更新同步标记' WHERE rowid = (SELECT rowid FROM search_chunks WHERE book_id = ? AND text LIKE '%删除同步标记%' LIMIT 1)",
    )
    .bind(first.id.to_string())
    .execute(database.pool())
    .await
    .unwrap();
    assert_eq!(fts_match_count(&database, "删除同步标记").await, 0);
    assert_eq!(fts_match_count(&database, "FTS更新同步标记").await, 1);

    sqlx::query("DELETE FROM books WHERE id = ?")
        .bind(second.id.to_string())
        .execute(database.pool())
        .await
        .unwrap();
    assert_eq!(counts(&database, second.id).await, (0, 0, 0, 0));
    assert_eq!(fts_match_count(&database, "第二本泄漏标记").await, 0);
}

#[tokio::test]
async fn begin_parse_rebuild_clears_old_rows_and_repopulates_fts() {
    let (temp, database, service) = test_service().await;
    let book = begin_pdf(&temp, &service, "rebuild.pdf", "rebuild-source").await;
    service
        .append_parsed_sections(
            book.id,
            vec![pdf_section(
                book.id,
                0,
                "旧章节",
                &[format!("{}。", "旧索引线性代数".repeat(160))],
            )],
        )
        .await
        .unwrap();
    service
        .finalize_import(book.id, noop_progress())
        .await
        .unwrap();
    assert!(fts_match_count(&database, "旧索引").await > 0);

    service
        .begin_parse(
            book.id,
            ParsedBookMetadata {
                title: "重建教材".to_owned(),
                author: None,
                language: Some("zh-CN".to_owned()),
            },
        )
        .await
        .unwrap();
    assert_eq!(counts(&database, book.id).await, (0, 0, 0, 0));
    assert_eq!(fts_match_count(&database, "旧索引").await, 0);
    service
        .append_parsed_sections(
            book.id,
            vec![pdf_section(
                book.id,
                0,
                "新章节",
                &[format!("{}。", "新索引拓扑学".repeat(160))],
            )],
        )
        .await
        .unwrap();
    service
        .finalize_import(book.id, noop_progress())
        .await
        .unwrap();

    assert!(
        search_book(database.pool(), book.id, "线性代", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !search_book(database.pool(), book.id, "拓扑学", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(fts_match_count(&database, "新索引").await > 0);
    let rebuilt = counts(&database, book.id).await;
    assert!(rebuilt.0 > 0 && rebuilt.1 > 0 && rebuilt.2 > 0);
    assert_eq!(rebuilt.2, rebuilt.3);
}

#[tokio::test]
async fn invalid_batches_mark_failed_and_leave_no_partial_derived_data() {
    let (temp, database, service) = test_service().await;
    for case in [
        "empty_text",
        "duplicate_section",
        "wrong_id",
        "wrong_format",
        "zero_page",
        "reversed_pages",
        "rect_page_outside",
        "bad_rectangle",
        "nonfinite_rectangle",
        "duplicate_block",
        "too_many_sections",
        "too_many_blocks",
        "oversized_json",
    ] {
        let book = begin_pdf(
            &temp,
            &service,
            &format!("invalid-{case}.pdf"),
            &format!("invalid-{case}-source"),
        )
        .await;
        let mut batch = vec![pdf_section(book.id, 0, "章节", &["有效正文。".to_owned()])];
        match case {
            "empty_text" => batch[0].blocks[0].plain_text = "  ".to_owned(),
            "duplicate_section" => batch.push(batch[0].clone()),
            "wrong_id" => batch[0].id = Uuid::new_v4(),
            "wrong_format" => {
                batch[0].locator = DocumentLocator::Epub {
                    cfi: "epubcfi(/6/2)".to_owned(),
                    section_id: batch[0].id,
                };
            }
            "zero_page" => {
                batch[0].locator = DocumentLocator::Pdf {
                    start_page: 0,
                    end_page: 1,
                    rects_by_page: None,
                };
            }
            "reversed_pages" => {
                batch[0].locator = DocumentLocator::Pdf {
                    start_page: 2,
                    end_page: 1,
                    rects_by_page: None,
                };
            }
            "rect_page_outside" => {
                batch[0].locator = DocumentLocator::Pdf {
                    start_page: 1,
                    end_page: 1,
                    rects_by_page: Some(BTreeMap::from([(
                        2,
                        vec![NormalizedRect {
                            x: 0.0,
                            y: 0.0,
                            width: 0.5,
                            height: 0.5,
                        }],
                    )])),
                };
            }
            "bad_rectangle" => {
                batch[0].blocks[0].locator = DocumentLocator::Pdf {
                    start_page: 1,
                    end_page: 1,
                    rects_by_page: Some(BTreeMap::from([(
                        1,
                        vec![NormalizedRect {
                            x: 0.8,
                            y: 0.0,
                            width: 0.4,
                            height: 0.5,
                        }],
                    )])),
                };
            }
            "nonfinite_rectangle" => {
                batch[0].blocks[0].locator = DocumentLocator::Pdf {
                    start_page: 1,
                    end_page: 1,
                    rects_by_page: Some(BTreeMap::from([(
                        1,
                        vec![NormalizedRect {
                            x: f64::NAN,
                            y: 0.0,
                            width: 0.4,
                            height: 0.5,
                        }],
                    )])),
                };
            }
            "duplicate_block" => {
                let duplicate = batch[0].blocks[0].clone();
                batch[0].blocks.push(duplicate);
            }
            "too_many_sections" => {
                batch = (0..26)
                    .map(|ordinal| {
                        pdf_section(book.id, ordinal, "章节", &["有效正文。".to_owned()])
                    })
                    .collect();
            }
            "too_many_blocks" => {
                batch = vec![pdf_section(
                    book.id,
                    0,
                    "章节",
                    &(0..501)
                        .map(|ordinal| format!("有效正文 {ordinal}。"))
                        .collect::<Vec<_>>(),
                )];
            }
            "oversized_json" => {
                batch[0].blocks[0].plain_text = "大".repeat(8 * 1024 * 1024);
            }
            _ => unreachable!(),
        }

        let error = service
            .append_parsed_sections(book.id, batch)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::InvalidInput, "case {case}");
        let row = sqlx::query("SELECT import_status, import_error_code FROM books WHERE id = ?")
            .bind(book.id.to_string())
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(
            row.get::<String, _>("import_status"),
            "failed",
            "case {case}"
        );
        assert_eq!(
            row.get::<String, _>("import_error_code"),
            "INVALID_INPUT",
            "case {case}"
        );
        assert_eq!(
            counts(&database, book.id).await,
            (0, 0, 0, 0),
            "case {case}"
        );
    }
}

#[tokio::test]
async fn a_later_invalid_batch_clears_every_previously_persisted_row() {
    let (temp, database, service) = test_service().await;
    let book = begin_pdf(&temp, &service, "cross-batch.pdf", "cross-batch-source").await;
    let first = pdf_section(book.id, 0, "已写入章节", &["已写入正文。".to_owned()]);
    service
        .append_parsed_sections(book.id, vec![first.clone()])
        .await
        .unwrap();
    assert_eq!(counts(&database, book.id).await, (1, 1, 0, 0));

    let error = service
        .append_parsed_sections(book.id, vec![first])
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidInput);
    let row = sqlx::query("SELECT import_status, import_error_code FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("import_status"), "failed");
    assert_eq!(row.get::<String, _>("import_error_code"), "INVALID_INPUT");
    assert_eq!(counts(&database, book.id).await, (0, 0, 0, 0));
}

#[tokio::test]
async fn a_mid_batch_sql_failure_rolls_back_then_records_safe_parse_failure() {
    let (temp, database, service) = test_service().await;
    let book = begin_pdf(
        &temp,
        &service,
        "append-rollback.pdf",
        "append-rollback-source",
    )
    .await;
    sqlx::query(
        "CREATE TRIGGER fail_second_test_block BEFORE INSERT ON blocks WHEN new.ordinal = 1 BEGIN SELECT RAISE(ABORT, 'simulated block insert failure'); END",
    )
    .execute(database.pool())
    .await
    .unwrap();
    let batch = vec![pdf_section(
        book.id,
        0,
        "事务章节",
        &["第一块。".to_owned(), "第二块。".to_owned()],
    )];

    let error = service
        .append_parsed_sections(book.id, batch)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::DatabaseError);
    let row = sqlx::query(
        "SELECT import_status, import_error_code, import_error_stage FROM books WHERE id = ?",
    )
    .bind(book.id.to_string())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("import_status"), "failed");
    assert_eq!(row.get::<String, _>("import_error_code"), "DATABASE_ERROR");
    assert_eq!(row.get::<String, _>("import_error_stage"), "parsing");
    assert_eq!(counts(&database, book.id).await, (0, 0, 0, 0));
}

#[tokio::test]
async fn batch_count_and_utf8_json_limits_accept_exact_boundaries() {
    const MAX_JSON_BYTES: usize = 8 * 1024 * 1024;
    let (temp, database, service) = test_service().await;

    let sections_book = begin_pdf(&temp, &service, "25-sections.pdf", "25-sections-source").await;
    let sections = (0..25)
        .map(|ordinal| {
            pdf_section(
                sections_book.id,
                ordinal,
                "边界章节",
                &[format!("正文 {ordinal}。")],
            )
        })
        .collect();
    service
        .append_parsed_sections(sections_book.id, sections)
        .await
        .unwrap();
    assert_eq!(counts(&database, sections_book.id).await.0, 25);

    let blocks_book = begin_pdf(&temp, &service, "500-blocks.pdf", "500-blocks-source").await;
    let block_texts = (0..500)
        .map(|ordinal| format!("正文 {ordinal}。"))
        .collect::<Vec<_>>();
    service
        .append_parsed_sections(
            blocks_book.id,
            vec![pdf_section(blocks_book.id, 0, "边界章节", &block_texts)],
        )
        .await
        .unwrap();
    assert_eq!(counts(&database, blocks_book.id).await.1, 500);

    let exact_book = begin_pdf(&temp, &service, "8mib.pdf", "8mib-source").await;
    service
        .append_parsed_sections(
            exact_book.id,
            pdf_batch_with_serialized_size(exact_book.id, MAX_JSON_BYTES),
        )
        .await
        .unwrap();
    assert_eq!(counts(&database, exact_book.id).await, (1, 1, 0, 0));

    let oversized_book =
        begin_pdf(&temp, &service, "8mib-plus-one.pdf", "8mib-plus-one-source").await;
    let error = service
        .append_parsed_sections(
            oversized_book.id,
            pdf_batch_with_serialized_size(oversized_book.id, MAX_JSON_BYTES + 1),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::InvalidInput);
    assert_eq!(counts(&database, oversized_book.id).await, (0, 0, 0, 0));
}

#[tokio::test]
async fn format_locators_and_parent_ids_are_validated_without_constructor_assumptions() {
    let (_temp, database, _service) = test_service().await;

    let epub_id = insert_parsing_book(&database, "epub", "invalid-epub").await;
    let epub_section_id = stable_section_id(epub_id, 0);
    let epub_block_id = stable_block_id(epub_id, 0, 0);
    let invalid_epub = NormalizedSectionInput {
        id: epub_section_id,
        parent_id: None,
        ordinal: 0,
        title: "EPUB".to_owned(),
        locator: DocumentLocator::Epub {
            cfi: "  ".to_owned(),
            section_id: epub_section_id,
        },
        blocks: vec![NormalizedBlockInput {
            id: epub_block_id,
            ordinal: 0,
            kind: BlockKind::Paragraph,
            plain_text: "正文".to_owned(),
            locator: DocumentLocator::Epub {
                cfi: "epubcfi(/6/2)".to_owned(),
                section_id: Uuid::new_v4(),
            },
        }],
    };
    assert_eq!(
        document_repository::append_parsed_sections(database.pool(), epub_id, &[invalid_epub])
            .await
            .unwrap_err()
            .code,
        AppErrorCode::InvalidInput
    );

    let epub_mismatch_id = insert_parsing_book(&database, "epub", "epub-section-mismatch").await;
    let epub_section_id = stable_section_id(epub_mismatch_id, 0);
    let epub_block_id = stable_block_id(epub_mismatch_id, 0, 0);
    let mismatched_epub = NormalizedSectionInput {
        id: epub_section_id,
        parent_id: None,
        ordinal: 0,
        title: "EPUB".to_owned(),
        locator: DocumentLocator::Epub {
            cfi: "epubcfi(/6/2)".to_owned(),
            section_id: epub_section_id,
        },
        blocks: vec![NormalizedBlockInput {
            id: epub_block_id,
            ordinal: 0,
            kind: BlockKind::Paragraph,
            plain_text: "正文".to_owned(),
            locator: DocumentLocator::Epub {
                cfi: "epubcfi(/6/2/4)".to_owned(),
                section_id: Uuid::new_v4(),
            },
        }],
    };
    assert_eq!(
        document_repository::append_parsed_sections(
            database.pool(),
            epub_mismatch_id,
            &[mismatched_epub]
        )
        .await
        .unwrap_err()
        .code,
        AppErrorCode::InvalidInput
    );

    for case in ["foreign_block", "unordered_offsets", "offset_overflow"] {
        let docx_id = insert_parsing_book(&database, "docx", case).await;
        let mut section = docx_section(docx_id);
        match case {
            "foreign_block" => {
                if let DocumentLocator::Docx { start_block_id, .. } = &mut section.locator {
                    *start_block_id = Uuid::new_v4();
                }
            }
            "unordered_offsets" => {
                let first_id = section.blocks[0].id;
                section.blocks[0].locator = DocumentLocator::Docx {
                    start_block_id: first_id,
                    start_offset: 3,
                    end_block_id: first_id,
                    end_offset: 2,
                };
            }
            "offset_overflow" => {
                let first_id = section.blocks[0].id;
                section.blocks[0].locator = DocumentLocator::Docx {
                    start_block_id: first_id,
                    start_offset: 0,
                    end_block_id: first_id,
                    end_offset: 4,
                };
            }
            _ => unreachable!(),
        }
        assert_eq!(
            document_repository::append_parsed_sections(database.pool(), docx_id, &[section])
                .await
                .unwrap_err()
                .code,
            AppErrorCode::InvalidInput,
            "case {case}"
        );
        assert_eq!(counts(&database, docx_id).await, (0, 0, 0, 0));
    }

    let parent_book = insert_parsing_book(&database, "pdf", "parent-book").await;
    let parent = pdf_section(parent_book, 0, "父章节", &["父正文。".to_owned()]);
    document_repository::append_parsed_sections(
        database.pool(),
        parent_book,
        std::slice::from_ref(&parent),
    )
    .await
    .unwrap();
    let child_book = insert_parsing_book(&database, "pdf", "child-book").await;
    let mut child = pdf_section(child_book, 0, "子章节", &["子正文。".to_owned()]);
    child.parent_id = Some(parent.id);
    assert_eq!(
        document_repository::append_parsed_sections(database.pool(), child_book, &[child])
            .await
            .unwrap_err()
            .code,
        AppErrorCode::InvalidInput
    );
    assert_eq!(counts(&database, child_book).await, (0, 0, 0, 0));
    assert_eq!(counts(&database, parent_book).await.0, 1);
}

#[tokio::test]
async fn indexing_failure_rolls_back_chunks_and_records_safe_failure() {
    let (temp, database, service) = test_service().await;
    let book = begin_pdf(&temp, &service, "rollback.pdf", "rollback-source").await;
    service
        .append_parsed_sections(
            book.id,
            vec![pdf_section(
                book.id,
                0,
                "回滚章节",
                &[format!("{}。", "原子事务正文".repeat(180))],
            )],
        )
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_test_chunk_insert BEFORE INSERT ON search_chunks BEGIN SELECT RAISE(ABORT, 'simulated indexing failure'); END",
    )
    .execute(database.pool())
    .await
    .unwrap();

    let error = service
        .finalize_import(book.id, noop_progress())
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::DatabaseError);
    let row = sqlx::query(
        "SELECT import_status, import_error_code, import_error_stage FROM books WHERE id = ?",
    )
    .bind(book.id.to_string())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("import_status"), "failed");
    assert_eq!(row.get::<String, _>("import_error_code"), "DATABASE_ERROR");
    assert_eq!(row.get::<String, _>("import_error_stage"), "indexing");
    assert_eq!(counts(&database, book.id).await, (0, 0, 0, 0));
}

#[tokio::test]
async fn cancellation_at_indexing_boundary_never_commits_ready_or_fts_rows() {
    let (temp, database, service) = test_service().await;
    let source = temp.path().join("cancel-source.pdf");
    fs::write(&source, "%PDF-1.7\ncancel-race\n%%EOF").unwrap();
    let outcome = service
        .begin_import(
            BeginImportRequest::new(source.to_string_lossy().into_owned()),
            Arc::new(|_: ImportEvent| {}),
        )
        .await
        .unwrap();
    let book = match outcome {
        BeginImportOutcome::Created { book } => book,
        BeginImportOutcome::Duplicate { .. } => unreachable!(),
    };
    service
        .begin_parse(
            book.id,
            ParsedBookMetadata {
                title: "取消竞态".to_owned(),
                author: None,
                language: Some("zh-CN".to_owned()),
            },
        )
        .await
        .unwrap();
    service
        .append_parsed_sections(
            book.id,
            vec![pdf_section(
                book.id,
                0,
                "取消章节",
                &[format!("{}。", "取消前正文".repeat(180))],
            )],
        )
        .await
        .unwrap();
    let registry = service.cancellations();
    let cancel_at_indexing = Arc::new(move |event: ImportEvent| {
        if event.stage == textbooklens_lib::documents::import::ImportStage::Indexing
            && event.completed == 0
        {
            registry.cancel(book.id);
        }
    });

    let error = service
        .finalize_import(book.id, cancel_at_indexing)
        .await
        .unwrap_err();
    assert_eq!(error.code, AppErrorCode::ImportCancelled);
    let status: String = sqlx::query_scalar("SELECT import_status FROM books WHERE id = ?")
        .bind(book.id.to_string())
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(counts(&database, book.id).await, (0, 0, 0, 0));
    assert!(!service.paths().books.join(book.id.to_string()).exists());
    assert!(source.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_cancel_during_index_transaction_rolls_back_before_ready() {
    let (temp, database, service) = test_service().await;
    let source = temp.path().join("cancel-in-transaction.pdf");
    fs::write(&source, "%PDF-1.7\ncancel-in-transaction\n%%EOF").unwrap();
    let book = match service
        .begin_import(
            BeginImportRequest::new(source.to_string_lossy().into_owned()),
            Arc::new(|_: ImportEvent| {}),
        )
        .await
        .unwrap()
    {
        BeginImportOutcome::Created { book } => book,
        BeginImportOutcome::Duplicate { .. } => unreachable!(),
    };
    service
        .begin_parse(
            book.id,
            ParsedBookMetadata {
                title: "事务内取消".to_owned(),
                author: None,
                language: Some("zh-CN".to_owned()),
            },
        )
        .await
        .unwrap();
    service
        .append_parsed_sections(
            book.id,
            vec![pdf_section(
                book.id,
                0,
                "事务取消章节",
                &[format!("{}。", "事务持锁取消正文".repeat(180))],
            )],
        )
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER slow_test_chunk_insert BEFORE INSERT ON search_chunks BEGIN SELECT length(randomblob(40000000)); SELECT length(randomblob(40000000)); SELECT length(randomblob(40000000)); SELECT length(randomblob(40000000)); END",
    )
    .execute(database.pool())
    .await
    .unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let book_id = book.id;
    let progress = Arc::new(move |event: ImportEvent| {
        if event.stage == textbooklens_lib::documents::import::ImportStage::Indexing
            && event.completed == 0
        {
            let _ = started_tx.send(());
        }
    });
    let finalize_service = service.clone();
    let finalize =
        tokio::spawn(async move { finalize_service.finalize_import(book_id, progress).await });

    tokio::task::spawn_blocking(move || started_rx.recv().unwrap())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    service.cancel_import(book.id).await.unwrap();
    let error = finalize.await.unwrap().unwrap_err();
    assert_eq!(error.code, AppErrorCode::ImportCancelled);
    let row = sqlx::query(
        "SELECT import_status, import_error_code, import_error_stage FROM books WHERE id = ?",
    )
    .bind(book.id.to_string())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("import_status"), "failed");
    assert_eq!(
        row.get::<String, _>("import_error_code"),
        "IMPORT_CANCELLED"
    );
    assert_eq!(row.get::<String, _>("import_error_stage"), "indexing");
    assert_eq!(counts(&database, book.id).await, (0, 0, 0, 0));
    assert!(!service.paths().books.join(book.id.to_string()).exists());
    assert!(source.exists());
    assert!(service.cancellations().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_cancel_after_commit_before_attempt_take_compensates_ready() {
    let (temp, database, service) = test_service().await;
    let source = temp.path().join("cancel-post-commit.pdf");
    fs::write(&source, "%PDF-1.7\ncancel-post-commit\n%%EOF").unwrap();
    let book = match service
        .begin_import(
            BeginImportRequest::new(source.to_string_lossy().into_owned()),
            Arc::new(|_: ImportEvent| {}),
        )
        .await
        .unwrap()
    {
        BeginImportOutcome::Created { book } => book,
        BeginImportOutcome::Duplicate { .. } => unreachable!(),
    };
    service
        .begin_parse(
            book.id,
            ParsedBookMetadata {
                title: "提交后取消".to_owned(),
                author: None,
                language: Some("zh-CN".to_owned()),
            },
        )
        .await
        .unwrap();
    service
        .append_parsed_sections(
            book.id,
            vec![pdf_section(
                book.id,
                0,
                "提交后取消章节",
                &[format!("{}。", "提交后取消正文".repeat(180))],
            )],
        )
        .await
        .unwrap();
    let (committed_tx, committed_rx) = std::sync::mpsc::channel();
    let (cancelled_tx, cancelled_rx) = std::sync::mpsc::channel();
    let cancelled_rx = Arc::new(std::sync::Mutex::new(cancelled_rx));
    let book_id = book.id;
    let progress = Arc::new(move |event: ImportEvent| {
        if event.message_key == "import.indexed" {
            committed_tx.send(()).unwrap();
            cancelled_rx.lock().unwrap().recv().unwrap();
        }
    });
    let cancel_service = service.clone();
    let cancel = tokio::spawn(async move {
        tokio::task::spawn_blocking(move || committed_rx.recv().unwrap())
            .await
            .unwrap();
        let result = cancel_service.cancel_import(book_id).await;
        cancelled_tx.send(()).unwrap();
        result
    });

    let error = service
        .finalize_import(book.id, progress)
        .await
        .unwrap_err();
    cancel.await.unwrap().unwrap();
    assert_eq!(error.code, AppErrorCode::ImportCancelled);
    let row = sqlx::query(
        "SELECT import_status, import_error_code, import_error_stage FROM books WHERE id = ?",
    )
    .bind(book.id.to_string())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("import_status"), "failed");
    assert_eq!(
        row.get::<String, _>("import_error_code"),
        "IMPORT_CANCELLED"
    );
    assert_eq!(row.get::<String, _>("import_error_stage"), "indexing");
    assert_eq!(counts(&database, book.id).await, (0, 0, 0, 0));
    assert!(!service.paths().books.join(book.id.to_string()).exists());
    assert!(source.exists());
    assert!(service.cancellations().is_empty());
}
