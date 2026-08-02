use std::{
    fs::{self, OpenOptions},
    io::Write,
};

use uuid::Uuid;

use crate::{
    app_state::AppPaths,
    errors::{AppError, AppErrorCode, AppResult},
};

const DOCUMENT_HTML: &str = "document.html";
const DOCUMENT_HTML_PARTIAL: &str = ".document.html.partial";

/// Writes the sole allowed derived artifact into the DB-owned book directory.
pub fn write_document_html(paths: &AppPaths, book_id: Uuid, content: &str) -> AppResult<()> {
    let derived = owned_derived_directory(paths, book_id)?;
    let target = derived.join(DOCUMENT_HTML);
    let partial = derived.join(DOCUMENT_HTML_PARTIAL);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let result = (|| -> AppResult<()> {
        file.write_all(content.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&partial, &target)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

pub fn remove_document_html(paths: &AppPaths, book_id: Uuid) -> AppResult<()> {
    let target = owned_derived_directory(paths, book_id)?.join(DOCUMENT_HTML);
    if target.exists() {
        fs::remove_file(target)?;
    }
    Ok(())
}

fn owned_derived_directory(paths: &AppPaths, book_id: Uuid) -> AppResult<std::path::PathBuf> {
    let books = fs::canonicalize(&paths.books)?;
    let book = fs::canonicalize(paths.books.join(book_id.to_string()))?;
    if book.parent() != Some(books.as_path()) || !book.starts_with(&books) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let derived = book.join("derived");
    fs::create_dir_all(&derived)?;
    let derived = fs::canonicalize(derived)?;
    if derived.parent() != Some(book.as_path()) || !derived.starts_with(&book) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    Ok(derived)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use uuid::Uuid;

    use crate::app_state::AppPaths;

    use super::write_document_html;

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
