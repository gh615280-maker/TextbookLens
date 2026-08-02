use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    domain::BookFormat,
    errors::{AppError, AppErrorCode, AppResult},
};

use super::import::{ImportEvent, ImportStage, ProgressEmitter};

pub const COPY_CHUNK_SIZE: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ValidatedSource {
    pub canonical_path: PathBuf,
    pub format: BookFormat,
    pub extension: &'static str,
    pub original_filename: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct CopiedSource {
    pub sha256: String,
    pub relative_path: String,
    pub absolute_path: PathBuf,
}

pub fn validate_source(path: &str) -> AppResult<ValidatedSource> {
    let canonical_path =
        fs::canonicalize(path).map_err(|_| AppError::new(AppErrorCode::LocalIoError))?;
    let metadata =
        fs::metadata(&canonical_path).map_err(|_| AppError::new(AppErrorCode::LocalIoError))?;
    if !metadata.is_file() {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }

    let original_filename = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::new(AppErrorCode::InvalidInput))?
        .to_owned();
    let extension = canonical_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AppError::new(AppErrorCode::UnsupportedFileType))?;
    let (format, extension) = match extension.as_str() {
        "pdf" => (BookFormat::Pdf, "pdf"),
        "epub" => (BookFormat::Epub, "epub"),
        "docx" => (BookFormat::Docx, "docx"),
        _ => return Err(AppError::new(AppErrorCode::UnsupportedFileType)),
    };

    Ok(ValidatedSource {
        canonical_path,
        format,
        extension,
        original_filename,
        size: metadata.len(),
    })
}

pub async fn copy_source(
    paths: &AppPaths,
    book_id: Uuid,
    source: &ValidatedSource,
    cancellation: CancellationToken,
    progress: ProgressEmitter,
) -> AppResult<CopiedSource> {
    let paths = paths.clone();
    let source = source.clone();
    tokio::task::spawn_blocking(move || {
        if cancellation.is_cancelled() {
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }

        let book_directory = paths.books.join(book_id.to_string());
        fs::create_dir_all(book_directory.join("derived"))?;
        let partial = book_directory.join(format!("original.{}.partial", source.extension));
        let final_path = book_directory.join(format!("original.{}", source.extension));
        let source_file = File::open(&source.canonical_path)
            .map_err(|_| AppError::new(AppErrorCode::LocalIoError))?;
        let target_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        let mut reader = BufReader::with_capacity(COPY_CHUNK_SIZE, source_file);
        let mut writer = BufWriter::with_capacity(COPY_CHUNK_SIZE, target_file);
        let mut buffer = vec![0_u8; COPY_CHUNK_SIZE];
        let mut hasher = Sha256::new();
        let mut completed = 0_u64;

        loop {
            if cancellation.is_cancelled() {
                return Err(AppError::new(AppErrorCode::ImportCancelled));
            }
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            writer.write_all(&buffer[..bytes_read])?;
            hasher.update(&buffer[..bytes_read]);
            completed += bytes_read as u64;
            progress(ImportEvent {
                stage: ImportStage::Copying,
                completed,
                total: source.size,
                message_key: "import.copying".to_owned(),
            });
            if cancellation.is_cancelled() {
                return Err(AppError::new(AppErrorCode::ImportCancelled));
            }
        }

        writer.flush()?;
        writer.get_ref().sync_all()?;
        if cancellation.is_cancelled() {
            return Err(AppError::new(AppErrorCode::ImportCancelled));
        }
        drop(writer);
        fs::rename(&partial, &final_path)?;

        Ok(CopiedSource {
            sha256: encode_hex(&hasher.finalize()),
            relative_path: owned_source_relative_path(book_id, &source.format),
            absolute_path: final_path,
        })
    })
    .await
    .map_err(|_| AppError::new(AppErrorCode::LocalIoError))?
}

pub fn remove_book_directory(paths: &AppPaths, book_id: Uuid) -> AppResult<()> {
    let directory = paths.books.join(book_id.to_string());
    if directory.exists() {
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata_is_reparse(&metadata) {
            fs::remove_dir(directory)?;
        } else {
            fs::remove_dir_all(directory)?;
        }
    }
    Ok(())
}

pub fn recover_import_storage(paths: &AppPaths, owned_book_ids: &HashSet<Uuid>) -> AppResult<()> {
    fs::create_dir_all(&paths.books)?;
    for entry in fs::read_dir(&paths.books)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let Some(book_id) = entry
            .file_name()
            .to_str()
            .and_then(|name| Uuid::parse_str(name).ok())
        else {
            continue;
        };
        if owned_book_ids.contains(&book_id) {
            remove_partial_files(&entry.path())?;
        } else {
            remove_book_directory(paths, book_id)?;
        }
    }
    Ok(())
}

fn remove_partial_files(directory: &Path) -> AppResult<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            remove_partial_files(&entry.path())?;
        } else if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".partial"))
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

pub fn clear_derived_directory(paths: &AppPaths, book_id: Uuid) -> AppResult<()> {
    let books = fs::canonicalize(&paths.books)?;
    let requested_book = paths.books.join(book_id.to_string());
    let requested_metadata = fs::symlink_metadata(&requested_book)?;
    if metadata_is_reparse(&requested_metadata) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let book_directory = fs::canonicalize(requested_book)?;
    if book_directory.parent() != Some(books.as_path()) || !book_directory.starts_with(&books) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let derived = book_directory.join("derived");
    if let Ok(metadata) = fs::symlink_metadata(&derived) {
        if metadata_is_reparse(&metadata) {
            remove_reparse_entry(&derived)?;
        } else if metadata.is_dir() {
            fs::remove_dir_all(&derived)?;
        } else {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
    }
    fs::create_dir(&derived)?;
    Ok(())
}

#[cfg(windows)]
fn remove_reparse_entry(path: &Path) -> std::io::Result<()> {
    fs::remove_dir(path)
        .or_else(|directory_error| fs::remove_file(path).map_err(|_| directory_error))
}

#[cfg(not(windows))]
fn remove_reparse_entry(path: &Path) -> std::io::Result<()> {
    fs::remove_file(path)
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

pub fn resolve_owned_source(
    paths: &AppPaths,
    book_id: Uuid,
    format: &BookFormat,
    stored_path: &str,
) -> AppResult<PathBuf> {
    let expected_relative = owned_source_relative_path(book_id, format);
    if stored_path != expected_relative {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let book_directory = fs::canonicalize(paths.books.join(book_id.to_string()))?;
    let candidate = fs::canonicalize(
        paths
            .books
            .join(book_id.to_string())
            .join(format!("original.{}", format_extension(format))),
    )?;
    if candidate.parent() != Some(book_directory.as_path())
        || !candidate.starts_with(&book_directory)
        || !fs::metadata(&candidate)?.is_file()
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(candidate)
}

pub fn owned_source_relative_path(book_id: Uuid, format: &BookFormat) -> String {
    format!("books/{book_id}/original.{}", format_extension(format))
}

fn format_extension(format: &BookFormat) -> &'static str {
    match format {
        BookFormat::Pdf => "pdf",
        BookFormat::Epub => "epub",
        BookFormat::Docx => "docx",
    }
}

pub fn hash_file(path: &Path) -> AppResult<String> {
    let mut reader = BufReader::with_capacity(COPY_CHUNK_SIZE, File::open(path)?);
    let mut buffer = vec![0_u8; COPY_CHUNK_SIZE];
    let mut hasher = Sha256::new();
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(encode_hex(&hasher.finalize()))
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn noop_progress() -> Arc<dyn Fn(ImportEvent) + Send + Sync> {
    Arc::new(|_| {})
}
