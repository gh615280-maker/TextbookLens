use std::{collections::HashMap, sync::Arc};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    book_repository::{self as books, HashClaim},
    domain::{
        BookFormat, BookSummary, DocumentLocator, ImportStatus, NormalizedSectionInput,
        stable_block_id, stable_section_id,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

use super::{
    source::read_book_source_bytes,
    storage::{
        clear_derived_directory, copy_source, hash_file, remove_book_directory,
        resolve_owned_source, validate_source,
    },
};

pub type ProgressEmitter = Arc<dyn Fn(ImportEvent) + Send + Sync>;
pub type ImportCancellationRegistry = Arc<Mutex<HashMap<Uuid, CancellationToken>>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeginImportRequest {
    pub source_path: String,
}

impl BeginImportRequest {
    pub fn new(source_path: String) -> Self {
        Self { source_path }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BeginImportOutcome {
    Created { book: BookSummary },
    Duplicate { book: BookSummary },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportStage {
    Copying,
    Parsing,
    Indexing,
}

impl ImportStage {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Copying => "copying",
            Self::Parsing => "parsing",
            Self::Indexing => "indexing",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportEvent {
    pub stage: ImportStage,
    pub completed: u64,
    pub total: u64,
    pub message_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedBookMetadata {
    pub title: String,
    pub author: Option<String>,
    pub language: Option<String>,
}

#[derive(Clone)]
pub struct ImportService {
    pool: SqlitePool,
    paths: AppPaths,
    cancellations: ImportCancellationRegistry,
}

impl ImportService {
    pub fn new(pool: SqlitePool, paths: AppPaths) -> Self {
        Self::with_cancellations(pool, paths, Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn with_cancellations(
        pool: SqlitePool,
        paths: AppPaths,
        cancellations: ImportCancellationRegistry,
    ) -> Self {
        Self {
            pool,
            paths,
            cancellations,
        }
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn cancellations(&self) -> ImportCancellationRegistry {
        Arc::clone(&self.cancellations)
    }

    pub async fn begin_import(
        &self,
        request: BeginImportRequest,
        progress: ProgressEmitter,
    ) -> AppResult<BeginImportOutcome> {
        self.begin_import_with_token(request, CancellationToken::new(), progress)
            .await
    }

    pub async fn begin_import_with_token(
        &self,
        request: BeginImportRequest,
        cancellation: CancellationToken,
        progress: ProgressEmitter,
    ) -> AppResult<BeginImportOutcome> {
        let source = validate_source(&request.source_path)?;
        let book_id = Uuid::new_v4();
        books::insert_copying(
            &self.pool,
            book_id,
            &source.format,
            &source.original_filename,
        )
        .await?;
        self.cancellations
            .lock()
            .insert(book_id, cancellation.clone());

        let copied = copy_source(&self.paths, book_id, &source, cancellation, progress).await;
        let copied = match copied {
            Ok(copied) => copied,
            Err(error) => {
                self.cancellations.lock().remove(&book_id);
                let _ = remove_book_directory(&self.paths, book_id);
                let message = error.code.user_message();
                let _ = books::mark_failed(&self.pool, book_id, error.code, message, true).await;
                return Err(error);
            }
        };

        match books::claim_hash(
            &self.pool,
            book_id,
            &copied.sha256,
            &copied.relative_path,
            None,
        )
        .await
        {
            Ok(HashClaim::Claimed(book)) => Ok(BeginImportOutcome::Created { book }),
            Ok(HashClaim::Duplicate(book)) => {
                self.cancellations.lock().remove(&book_id);
                remove_book_directory(&self.paths, book_id)?;
                Ok(BeginImportOutcome::Duplicate { book })
            }
            Err(error) => {
                self.cancellations.lock().remove(&book_id);
                let _ = remove_book_directory(&self.paths, book_id);
                let _ = books::mark_failed(
                    &self.pool,
                    book_id,
                    error.code,
                    error.code.user_message(),
                    true,
                )
                .await;
                Err(error)
            }
        }
    }

    pub async fn read_book_source(&self, book_id: Uuid) -> AppResult<Vec<u8>> {
        read_book_source_bytes(&self.pool, &self.paths, book_id).await
    }

    pub async fn cancel_import(&self, book_id: Uuid) -> AppResult<()> {
        if let Some(cancellation) = self.cancellations.lock().get(&book_id).cloned() {
            cancellation.cancel();
        }
        match books::get(&self.pool, book_id).await {
            Ok(record) if record.summary.import_status == ImportStatus::Parsing => {
                remove_book_directory(&self.paths, book_id)?;
                books::mark_failed(
                    &self.pool,
                    book_id,
                    AppErrorCode::ImportCancelled,
                    AppErrorCode::ImportCancelled.user_message(),
                    true,
                )
                .await?;
                self.cancellations.lock().remove(&book_id);
            }
            Ok(_) => {}
            Err(error) if error.code == AppErrorCode::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(())
    }

    pub async fn retry_import(
        &self,
        book_id: Uuid,
        replacement_source_path: Option<String>,
        progress: ProgressEmitter,
    ) -> AppResult<BeginImportOutcome> {
        let record = books::get(&self.pool, book_id).await?;
        if record.summary.import_status != ImportStatus::Failed {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }

        if let Some(replacement_source_path) = replacement_source_path {
            let source = validate_source(&replacement_source_path)?;
            if source.format != record.summary.format {
                return Err(AppError::new(AppErrorCode::UnsupportedFileType));
            }
            remove_book_directory(&self.paths, book_id)?;
            books::prepare_failed_for_replacement(&self.pool, book_id).await?;
            let cancellation = CancellationToken::new();
            self.cancellations
                .lock()
                .insert(book_id, cancellation.clone());
            let copied = copy_source(&self.paths, book_id, &source, cancellation, progress).await;
            let copied = match copied {
                Ok(copied) => copied,
                Err(error) => {
                    self.cancellations.lock().remove(&book_id);
                    let _ = remove_book_directory(&self.paths, book_id);
                    let _ = books::mark_failed(
                        &self.pool,
                        book_id,
                        error.code,
                        error.code.user_message(),
                        true,
                    )
                    .await;
                    return Err(error);
                }
            };
            let claim = books::claim_hash(
                &self.pool,
                book_id,
                &copied.sha256,
                &copied.relative_path,
                Some(&source.original_filename),
            )
            .await;
            return match claim {
                Ok(HashClaim::Claimed(book)) => Ok(BeginImportOutcome::Created { book }),
                Ok(HashClaim::Duplicate(book)) => {
                    self.cancellations.lock().remove(&book_id);
                    remove_book_directory(&self.paths, book_id)?;
                    Ok(BeginImportOutcome::Duplicate { book })
                }
                Err(error) => {
                    self.cancellations.lock().remove(&book_id);
                    let _ = remove_book_directory(&self.paths, book_id);
                    let _ = books::mark_failed(
                        &self.pool,
                        book_id,
                        error.code,
                        error.code.user_message(),
                        true,
                    )
                    .await;
                    Err(error)
                }
            };
        }

        let owned_path = resolve_owned_source(&self.paths, book_id, &record.stored_path)
            .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
        let actual_hash = hash_file(&owned_path)?;
        if actual_hash != record.sha256 {
            return Err(AppError::new(AppErrorCode::FileCorrupted));
        }
        clear_derived_directory(&self.paths, book_id)?;
        let book = books::reset_failed_for_retry(&self.pool, book_id).await?;
        self.cancellations
            .lock()
            .insert(book_id, CancellationToken::new());
        Ok(BeginImportOutcome::Created { book })
    }

    pub async fn begin_parse(&self, book_id: Uuid, metadata: ParsedBookMetadata) -> AppResult<()> {
        self.ensure_not_cancelled(book_id)?;
        books::begin_parse(
            &self.pool,
            book_id,
            &metadata.title,
            metadata.author.as_deref(),
            metadata.language.as_deref(),
        )
        .await
    }

    pub async fn append_parsed_sections(
        &self,
        book_id: Uuid,
        sections: Vec<NormalizedSectionInput>,
    ) -> AppResult<()> {
        self.ensure_not_cancelled(book_id)?;
        books::require_parsing(&self.pool, book_id).await?;
        let format = books::get(&self.pool, book_id).await?.summary.format;
        validate_section_batch(book_id, &format, &sections)?;
        self.ensure_not_cancelled(book_id)
    }

    pub async fn finalize_import(
        &self,
        book_id: Uuid,
        progress: ProgressEmitter,
    ) -> AppResult<BookSummary> {
        self.ensure_not_cancelled(book_id)?;
        books::require_parsing(&self.pool, book_id).await?;
        progress(ImportEvent {
            stage: ImportStage::Indexing,
            completed: 0,
            total: 1,
            message_key: "import.indexing".to_owned(),
        });
        let book = books::finalize(&self.pool, book_id).await?;
        let cancelled = self
            .cancellations
            .lock()
            .remove(&book_id)
            .is_some_and(|cancellation| cancellation.is_cancelled());
        if cancelled {
            remove_book_directory(&self.paths, book_id)?;
            books::mark_failed(
                &self.pool,
                book_id,
                AppErrorCode::ImportCancelled,
                AppErrorCode::ImportCancelled.user_message(),
                true,
            )
            .await?;
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }
        progress(ImportEvent {
            stage: ImportStage::Indexing,
            completed: 1,
            total: 1,
            message_key: "import.ready".to_owned(),
        });
        Ok(book)
    }

    pub async fn mark_import_failed(
        &self,
        book_id: Uuid,
        stage: ImportStage,
        code: AppErrorCode,
    ) -> AppResult<()> {
        if let Some(cancellation) = self.cancellations.lock().remove(&book_id) {
            cancellation.cancel();
        }
        let message = format!("{}: {}", stage.as_str(), code.user_message());
        books::mark_failed(&self.pool, book_id, code, &message, false).await
    }

    pub async fn delete_failed_import(&self, book_id: Uuid) -> AppResult<()> {
        let record = books::get(&self.pool, book_id).await?;
        if record.summary.import_status != ImportStatus::Failed {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        remove_book_directory(&self.paths, book_id)?;
        self.cancellations.lock().remove(&book_id);
        books::delete_failed(&self.pool, book_id).await
    }

    pub async fn get_book(&self, book_id: Uuid) -> AppResult<BookSummary> {
        Ok(books::get(&self.pool, book_id).await?.summary)
    }

    pub async fn list_books(&self) -> AppResult<Vec<BookSummary>> {
        books::list(&self.pool).await
    }

    fn ensure_not_cancelled(&self, book_id: Uuid) -> AppResult<()> {
        if self
            .cancellations
            .lock()
            .get(&book_id)
            .is_some_and(CancellationToken::is_cancelled)
        {
            Err(AppError::new(AppErrorCode::ImportCancelled))
        } else {
            Ok(())
        }
    }
}

fn validate_section_batch(
    book_id: Uuid,
    format: &BookFormat,
    sections: &[NormalizedSectionInput],
) -> AppResult<()> {
    let block_count = sections
        .iter()
        .map(|section| section.blocks.len())
        .sum::<usize>();
    if sections.is_empty() || sections.len() > 25 || block_count > 500 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    for section in sections {
        if section.id != stable_section_id(book_id, section.ordinal)
            || !locator_matches_format(&section.locator, format)
        {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        for block in &section.blocks {
            if block.id != stable_block_id(book_id, section.ordinal, block.ordinal)
                || !locator_matches_format(&block.locator, format)
            {
                return Err(AppError::new(AppErrorCode::InvalidInput));
            }
        }
    }
    Ok(())
}

fn locator_matches_format(locator: &DocumentLocator, format: &BookFormat) -> bool {
    matches!(
        (locator, format),
        (DocumentLocator::Pdf { .. }, BookFormat::Pdf)
            | (DocumentLocator::Epub { .. }, BookFormat::Epub)
            | (DocumentLocator::Docx { .. }, BookFormat::Docx)
    )
}
