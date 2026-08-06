use std::sync::Mutex;

use tempfile::TempDir;

use super::*;

#[test]
fn walker_counts_only_canonical_owned_files_in_fixed_categories() {
    let temporary = TempDir::new().unwrap();
    let (paths, layout) = prepared_layout(&temporary);
    let book_id = Uuid::new_v4();
    let book = paths.books.join(book_id.to_string());
    fs::create_dir(&book).unwrap();
    fs::write(book.join("original.pdf"), b"source").unwrap();
    let derived = book.join("derived");
    fs::create_dir(&derived).unwrap();
    fs::write(derived.join("document.html"), b"derived-content").unwrap();
    let index = paths.cache.join("indexing-pages");
    fs::create_dir(&index).unwrap();
    fs::write(index.join("synthetic-page.png"), b"index").unwrap();
    fs::write(paths.cache.join("reader-cache.bin"), b"cache-data").unwrap();
    fs::write(paths.logs.join("textbooklens.log"), b"log").unwrap();
    fs::write(&paths.database, b"database").unwrap();
    let outside = temporary.path().join("outside-decoy.bin");
    fs::write(&outside, vec![b'x'; 8_192]).unwrap();

    let usage = collect_storage_usage(&layout).unwrap();

    assert_eq!(usage.total_bytes, 6 + 15 + 5 + 10 + 3 + 8);
    assert_eq!(usage.total_file_count, 6);
    assert_eq!(
        usage.categories,
        vec![
            StorageCategoryUsageDto {
                category: StorageCategory::Source,
                bytes: 6,
                file_count: 1,
            },
            StorageCategoryUsageDto {
                category: StorageCategory::Derived,
                bytes: 15,
                file_count: 1,
            },
            StorageCategoryUsageDto {
                category: StorageCategory::Index,
                bytes: 5,
                file_count: 1,
            },
            StorageCategoryUsageDto {
                category: StorageCategory::Database,
                bytes: 8,
                file_count: 1,
            },
            StorageCategoryUsageDto {
                category: StorageCategory::Cache,
                bytes: 10,
                file_count: 1,
            },
            StorageCategoryUsageDto {
                category: StorageCategory::Log,
                bytes: 3,
                file_count: 1,
            },
        ]
    );
    assert_eq!(fs::metadata(outside).unwrap().len(), 8_192);
    let serialized = serde_json::to_string(&usage).unwrap();
    for forbidden in [
        "original.pdf",
        "document.html",
        "synthetic-page.png",
        "outside-decoy.bin",
        "textbooklens.log",
        "bookId",
        "title",
        "path",
    ] {
        assert!(!serialized.contains(forbidden));
    }
}

#[test]
fn walker_rejects_unknown_owned_entries_instead_of_reporting_a_partial_total() {
    let temporary = TempDir::new().unwrap();
    let (paths, layout) = prepared_layout(&temporary);
    fs::write(paths.root.join("unexpected-private-data.bin"), b"sentinel").unwrap();

    let error = collect_storage_usage(&layout).unwrap_err();

    assert_eq!(error.code, MaintenanceErrorCode::StorageEntryUnsafe);
    assert_eq!(
        serde_json::to_value(MaintenanceErrorDto::from(error)).unwrap(),
        serde_json::json!({
            "code": "STORAGE_ENTRY_UNSAFE",
            "activeOperations": []
        })
    );
}

#[cfg(unix)]
#[test]
fn walker_rejects_symlinks_and_hard_link_aliases() {
    use std::os::unix::fs::symlink;

    let temporary = TempDir::new().unwrap();
    let (paths, layout) = prepared_layout(&temporary);
    let outside = temporary.path().join("outside.bin");
    fs::write(&outside, b"outside sentinel").unwrap();
    symlink(&outside, paths.cache.join("linked.bin")).unwrap();
    assert_eq!(
        collect_storage_usage(&layout).unwrap_err().code,
        MaintenanceErrorCode::StorageEntryUnsafe
    );
    fs::remove_file(paths.cache.join("linked.bin")).unwrap();

    let first = paths.cache.join("first.bin");
    fs::write(&first, b"same inode").unwrap();
    fs::hard_link(&first, paths.cache.join("alias.bin")).unwrap();
    assert_eq!(
        collect_storage_usage(&layout).unwrap_err().code,
        MaintenanceErrorCode::StorageEntryUnsafe
    );
}

#[cfg(windows)]
#[test]
fn walker_rejects_junction_reparse_points_without_counting_the_decoy() {
    let temporary = TempDir::new().unwrap();
    let (paths, layout) = prepared_layout(&temporary);
    let outside = temporary.path().join("outside-junction-target");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private-decoy.bin"), vec![b'd'; 4_096]).unwrap();
    create_windows_junction(&outside, &paths.cache.join("linked-cache"));

    let error = collect_storage_usage(&layout).unwrap_err();

    assert_eq!(error.code, MaintenanceErrorCode::StorageEntryUnsafe);
    assert_eq!(
        fs::read(outside.join("private-decoy.bin")).unwrap().len(),
        4_096
    );
}

#[test]
fn checked_arithmetic_fails_before_any_total_wraps() {
    let mut bytes = StorageAccumulator::default();
    bytes.categories[StorageCategory::Source.index()].bytes = u64::MAX;
    bytes.total_bytes = u64::MAX;
    assert_eq!(
        bytes.add(StorageCategory::Source, 1).unwrap_err().code,
        MaintenanceErrorCode::StorageSizeOverflow
    );

    let mut files = StorageAccumulator::default();
    files.categories[StorageCategory::Cache.index()].files = u64::MAX;
    files.total_files = u64::MAX;
    assert_eq!(
        files.add(StorageCategory::Cache, 0).unwrap_err().code,
        MaintenanceErrorCode::StorageSizeOverflow
    );
}

#[test]
fn app_root_validation_rejects_relative_unresolved_and_non_owned_paths() {
    assert_eq!(
        validate_untrusted_root_syntax(Path::new(APP_DATA_DIRECTORY_NAME))
            .unwrap_err()
            .code,
        MaintenanceErrorCode::StorageRootInvalid
    );
    let unresolved = PathBuf::from(format!(
        "C:\\%USERPROFILE%\\AppData\\Roaming\\{APP_DATA_DIRECTORY_NAME}"
    ));
    assert_eq!(
        validate_untrusted_root_syntax(&unresolved)
            .unwrap_err()
            .code,
        MaintenanceErrorCode::StorageRootInvalid
    );
    let glob = PathBuf::from(format!("C:\\temp\\*\\{APP_DATA_DIRECTORY_NAME}"));
    assert_eq!(
        validate_untrusted_root_syntax(&glob).unwrap_err().code,
        MaintenanceErrorCode::StorageRootInvalid
    );
    let wrong_leaf = std::env::current_dir().unwrap();
    assert_eq!(
        validate_untrusted_root_syntax(&wrong_leaf)
            .unwrap_err()
            .code,
        MaintenanceErrorCode::StorageRootInvalid
    );
}

#[cfg(windows)]
#[test]
fn app_root_validation_rejects_unc_and_device_inputs() {
    for candidate in [
        format!(r"\\server\share\{APP_DATA_DIRECTORY_NAME}"),
        format!(r"\\?\C:\temp\{APP_DATA_DIRECTORY_NAME}"),
        format!(r"\\.\C:\temp\{APP_DATA_DIRECTORY_NAME}"),
    ] {
        assert_eq!(
            validate_untrusted_root_syntax(Path::new(&candidate))
                .unwrap_err()
                .code,
            MaintenanceErrorCode::StorageRootInvalid
        );
    }
}

#[test]
fn open_directory_uses_only_the_revalidated_app_root_and_an_injected_launcher() {
    let temporary = TempDir::new().unwrap();
    let (paths, _layout) = prepared_layout(&temporary);
    let launcher = FakeLauncher::default();

    open_canonical_app_data_directory(&paths.root, &launcher).unwrap();

    let opened = launcher.opened.lock().unwrap();
    assert_eq!(opened.as_slice(), [paths.root]);
    assert!(!opened[0].starts_with(temporary.path().join("outside")));
}

#[test]
fn launcher_failure_returns_only_a_stable_safe_code() {
    let temporary = TempDir::new().unwrap();
    let (paths, _layout) = prepared_layout(&temporary);
    let launcher = FakeLauncher {
        opened: Mutex::new(Vec::new()),
        fail: true,
    };

    let error = open_canonical_app_data_directory(&paths.root, &launcher).unwrap_err();

    assert_eq!(error.code, MaintenanceErrorCode::AppDataOpenFailed);
    let serialized = serde_json::to_string(&MaintenanceErrorDto::from(error)).unwrap();
    assert_eq!(
        serialized,
        r#"{"code":"APP_DATA_OPEN_FAILED","activeOperations":[]}"#
    );
    assert!(!serialized.contains(&paths.root.to_string_lossy().into_owned()));
}

#[derive(Default)]
struct FakeLauncher {
    opened: Mutex<Vec<PathBuf>>,
    fail: bool,
}

impl DirectoryLauncher for FakeLauncher {
    fn launch(&self, canonical_app_data_root: &Path) -> Result<(), DirectoryLaunchError> {
        self.opened
            .lock()
            .unwrap()
            .push(canonical_app_data_root.to_path_buf());
        if self.fail {
            Err(DirectoryLaunchError)
        } else {
            Ok(())
        }
    }
}

fn prepared_layout(temporary: &TempDir) -> (AppPaths, StorageLayout) {
    let prepared = prepare_app_data_paths(&temporary.path().join(APP_DATA_DIRECTORY_NAME)).unwrap();
    let paths = AppPaths {
        root: prepared.root,
        books: prepared.books,
        cache: prepared.cache,
        logs: prepared.logs,
        database: prepared.database,
    };
    let layout = StorageLayout::from_app_paths(&paths).unwrap();
    assert!(StorageLayout::from_root(&paths.root).is_ok());
    (paths, layout)
}

#[cfg(windows)]
fn create_windows_junction(source: &Path, target: &Path) {
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(target)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
