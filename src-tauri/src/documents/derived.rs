use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    errors::{AppError, AppErrorCode, AppResult},
};

const DOCUMENT_HTML: &str = "document.html";
const DOCUMENT_HTML_PARTIAL: &str = ".document.html.partial";
const MAX_DOCUMENT_HTML_BYTES: usize = 8 * 1024 * 1024;

/// Writes the sole allowed derived artifact into the DB-owned book directory.
pub fn write_document_html(paths: &AppPaths, book_id: Uuid, content: &str) -> AppResult<()> {
    if content.is_empty() || content.len() > MAX_DOCUMENT_HTML_BYTES {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let derived = owned_derived_directory(paths, book_id)?;
    let target = derived.join(DOCUMENT_HTML);
    let partial = derived.join(DOCUMENT_HTML_PARTIAL);
    let result = (|| -> AppResult<()> {
        reject_reparse_point(&target)?;
        if target.exists() {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        reject_reparse_point(&partial)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = remove_path_entry(&partial);
    }
    result
}

pub fn remove_document_html(paths: &AppPaths, book_id: Uuid) -> AppResult<()> {
    let target = owned_derived_directory(paths, book_id)?.join(DOCUMENT_HTML);
    reject_reparse_point(&target)?;
    if target.exists() {
        fs::remove_file(target)?;
    }
    Ok(())
}

pub fn require_document_html(paths: &AppPaths, book_id: Uuid) -> AppResult<()> {
    let target = owned_derived_directory(paths, book_id)?.join(DOCUMENT_HTML);
    reject_reparse_point(&target)?;
    let metadata = fs::metadata(target).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::new(AppErrorCode::InvalidInput)
        } else {
            error.into()
        }
    })?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_DOCUMENT_HTML_BYTES as u64
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(())
}

fn owned_derived_directory(paths: &AppPaths, book_id: Uuid) -> AppResult<PathBuf> {
    let books = fs::canonicalize(&paths.books)?;
    let requested_book = paths.books.join(book_id.to_string());
    reject_reparse_point(&requested_book)?;
    let book = fs::canonicalize(requested_book)?;
    if book.parent() != Some(books.as_path()) || !book.starts_with(&books) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let derived = book.join("derived");
    reject_reparse_point(&derived)?;
    if !derived.exists() {
        fs::create_dir(&derived)?;
    }
    let derived = fs::canonicalize(derived)?;
    if derived.parent() != Some(book.as_path()) || !derived.starts_with(&book) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(derived)
}

fn reject_reparse_point(path: &Path) -> AppResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_reparse_point(&metadata) => {
            Err(AppError::new(AppErrorCode::InvalidInput))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove_path_entry(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        #[cfg(windows)]
        Ok(metadata) if is_reparse_point(&metadata) => fs::remove_dir(path)
            .or_else(|directory_error| fs::remove_file(path).map_err(|_| directory_error)),
        Ok(metadata) if metadata.is_dir() => fs::remove_dir(path),
        Ok(_) => fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use uuid::Uuid;

    use crate::{app_state::AppPaths, errors::AppErrorCode};

    use super::{MAX_DOCUMENT_HTML_BYTES, require_document_html, write_document_html};

    #[test]
    fn import_derived_document_is_confined_and_atomic() {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        fs::create_dir_all(paths.books.join(book_id.to_string()).join("derived"))
            .expect("owned derived directory");

        write_document_html(&paths, book_id, "<p>safe</p>").expect("derived write");

        let derived = paths.books.join(book_id.to_string()).join("derived");
        assert_eq!(
            fs::read_to_string(derived.join("document.html")).unwrap(),
            "<p>safe</p>"
        );
        assert!(!derived.join(".document.html.partial").exists());
    }

    #[test]
    fn oversized_derived_html_is_rejected_without_a_partial() {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        let derived = paths.books.join(book_id.to_string()).join("derived");
        fs::create_dir_all(&derived).expect("owned derived directory");
        let content = "x".repeat(MAX_DOCUMENT_HTML_BYTES + 1);

        let error = write_document_html(&paths, book_id, &content).unwrap_err();

        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert!(!derived.join("document.html").exists());
        assert!(!derived.join(".document.html.partial").exists());
    }

    #[test]
    fn missing_document_html_is_an_invalid_parser_contract() {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        fs::create_dir_all(paths.books.join(book_id.to_string()).join("derived"))
            .expect("owned derived directory");

        let error = require_document_html(&paths, book_id).unwrap_err();

        assert_eq!(error.code, AppErrorCode::InvalidInput);
    }

    #[test]
    fn every_write_error_removes_the_sibling_partial_without_overwriting_target() {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        let derived = paths.books.join(book_id.to_string()).join("derived");
        fs::create_dir_all(&derived).expect("owned derived directory");
        fs::write(derived.join("document.html"), b"first").unwrap();
        fs::write(derived.join(".document.html.partial"), b"residue").unwrap();

        write_document_html(&paths, book_id, "second").unwrap_err();

        assert_eq!(fs::read(derived.join("document.html")).unwrap(), b"first");
        assert!(!derived.join(".document.html.partial").exists());
    }

    #[cfg(unix)]
    #[test]
    fn derived_target_symlink_is_rejected_without_touching_its_destination() {
        use std::os::unix::fs::symlink;

        assert_target_symlink_is_rejected(|source, target| symlink(source, target));
    }

    #[cfg(windows)]
    #[test]
    fn derived_target_reparse_point_is_rejected_without_touching_its_destination() {
        assert_windows_junction_is_rejected("document.html", false);
    }

    #[cfg(unix)]
    #[test]
    fn derived_partial_symlink_is_rejected_and_unlinked() {
        use std::os::unix::fs::symlink;

        assert_partial_symlink_is_rejected(|source, target| symlink(source, target));
    }

    #[cfg(windows)]
    #[test]
    fn derived_partial_reparse_point_is_rejected_and_unlinked() {
        assert_windows_junction_is_rejected(".document.html.partial", true);
    }

    #[cfg(unix)]
    #[test]
    fn derived_directory_symlink_is_rejected_without_writing_outside() {
        use std::os::unix::fs::symlink;

        assert_derived_directory_symlink_is_rejected(|source, target| symlink(source, target));
    }

    #[cfg(windows)]
    #[test]
    fn derived_directory_reparse_point_is_rejected_without_writing_outside() {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        let book = paths.books.join(book_id.to_string());
        fs::create_dir_all(&book).expect("owned book directory");
        let outside = temporary.path().join("outside-derived");
        fs::create_dir(&outside).unwrap();
        create_windows_junction(&outside, &book.join("derived"));

        let error = write_document_html(&paths, book_id, "replacement").unwrap_err();

        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert!(fs::read_dir(outside).unwrap().next().is_none());
    }

    #[cfg(unix)]
    fn assert_target_symlink_is_rejected(
        create_link: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
    ) {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        let derived = paths.books.join(book_id.to_string()).join("derived");
        fs::create_dir_all(&derived).expect("owned derived directory");
        let outside = temporary.path().join("outside.html");
        fs::write(&outside, b"outside").unwrap();
        create_link(&outside, &derived.join("document.html"))
            .expect("test must create a target reparse point");

        let error = write_document_html(&paths, book_id, "replacement").unwrap_err();

        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert_eq!(fs::read(outside).unwrap(), b"outside");
    }

    #[cfg(unix)]
    fn assert_partial_symlink_is_rejected(
        create_link: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
    ) {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        let derived = paths.books.join(book_id.to_string()).join("derived");
        fs::create_dir_all(&derived).expect("owned derived directory");
        let outside = temporary.path().join("outside.partial");
        fs::write(&outside, b"outside").unwrap();
        let partial = derived.join(".document.html.partial");
        create_link(&outside, &partial).expect("test must create a partial reparse point");

        let error = write_document_html(&paths, book_id, "replacement").unwrap_err();

        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert_eq!(fs::read(outside).unwrap(), b"outside");
        assert!(!partial.exists());
    }

    #[cfg(unix)]
    fn assert_derived_directory_symlink_is_rejected(
        create_link: impl FnOnce(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
    ) {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        let book = paths.books.join(book_id.to_string());
        fs::create_dir_all(&book).expect("owned book directory");
        let outside = temporary.path().join("outside-derived");
        fs::create_dir(&outside).unwrap();
        create_link(&outside, &book.join("derived"))
            .expect("test must create a derived-directory reparse point");

        let error = write_document_html(&paths, book_id, "replacement").unwrap_err();

        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert!(!outside.join("document.html").exists());
        assert!(!outside.join(".document.html.partial").exists());
    }

    #[cfg(windows)]
    fn assert_windows_junction_is_rejected(name: &str, expect_unlinked: bool) {
        let temporary = TempDir::new().expect("temporary root");
        let paths = paths(&temporary);
        let book_id = Uuid::new_v4();
        let derived = paths.books.join(book_id.to_string()).join("derived");
        fs::create_dir_all(&derived).expect("owned derived directory");
        let outside = temporary.path().join("outside-entry");
        fs::create_dir(&outside).unwrap();
        let link = derived.join(name);
        create_windows_junction(&outside, &link);

        let error = write_document_html(&paths, book_id, "replacement").unwrap_err();

        assert_eq!(error.code, AppErrorCode::InvalidInput);
        assert!(fs::read_dir(outside).unwrap().next().is_none());
        assert_eq!(link.exists(), !expect_unlinked);
    }

    #[cfg(windows)]
    fn create_windows_junction(source: &std::path::Path, target: &std::path::Path) {
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(target)
            .arg(source)
            .output()
            .expect("run mklink");
        assert!(
            output.status.success(),
            "mklink failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn paths(temporary: &TempDir) -> AppPaths {
        let root = temporary.path().join("app");
        let books = root.join("books");
        fs::create_dir_all(&books).expect("books directory");
        AppPaths {
            cache: root.join("cache"),
            database: root.join("library.sqlite3"),
            logs: root.join("logs"),
            root,
            books,
        }
    }
}
