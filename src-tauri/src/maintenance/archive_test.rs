use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
};

use tempfile::TempDir;

use super::*;
use crate::{
    db::Database,
    domain::{ActiveOperationKind, MaintenanceStatusCode},
    maintenance::storage::prepare_app_data_paths,
};

const BOOK_TITLE_SENTINEL: &str = "PRIVATE_BOOK_TITLE_SENTINEL";
const SOURCE_SENTINEL: &[u8] = b"%PDF-1.7\nPRIVATE_SOURCE_TEXT_SENTINEL\n%%EOF";
const DERIVED_SENTINEL: &[u8] = b"PRIVATE_DERIVED_TEXT_SENTINEL";
const CACHE_SENTINEL: &[u8] = b"PRIVATE_CACHE_SENTINEL";
const LOG_CREDENTIAL_SENTINEL: &[u8] = b"SYNTHETIC_CREDENTIAL_SENTINEL";
const REMOTE_REFERENCE_SENTINEL: &str = "enc:v1:REMOTE_REFERENCE_SENTINEL";

struct BackupFixture {
    temporary: TempDir,
    database: Database,
    paths: AppPaths,
    gate: MaintenanceGate,
    book_id: Uuid,
    source: PathBuf,
}

impl BackupFixture {
    fn new() -> Self {
        let temporary = TempDir::new().expect("temporary fixture");
        let prepared = prepare_app_data_paths(&temporary.path().join(APP_DATA_DIRECTORY_NAME))
            .expect("canonical app paths");
        let paths = AppPaths {
            root: prepared.root,
            books: prepared.books,
            cache: prepared.cache,
            logs: prepared.logs,
            database: prepared.database,
        };
        let database = Database::open(&paths.database).expect("fixture database");
        let book_id = uuid::uuid!("7cc38f8b-3db0-4728-9821-9da5aa3ce35d");
        let book = paths.books.join(book_id.to_string());
        let derived = book.join("derived");
        fs::create_dir_all(&derived).expect("book directories");
        let source = book.join("original.pdf");
        fs::write(&source, SOURCE_SENTINEL).expect("synthetic source");
        fs::write(derived.join("document.html"), DERIVED_SENTINEL).expect("synthetic derived file");
        fs::write(paths.cache.join("private-cache.bin"), CACHE_SENTINEL).expect("synthetic cache");
        fs::write(paths.logs.join("diagnostic.log"), LOG_CREDENTIAL_SENTINEL)
            .expect("synthetic log");

        tauri::async_runtime::block_on(async {
            sqlx::query(
                "INSERT INTO books (id, sha256, title, author, language, format, original_filename, stored_path, import_status, created_at, updated_at, reading_progress) VALUES (?, ?, ?, NULL, NULL, 'pdf', 'synthetic.pdf', ?, 'ready', ?, ?, 0)",
            )
            .bind(book_id.to_string())
            .bind(hex_digest(Sha256::digest(SOURCE_SENTINEL)))
            .bind(BOOK_TITLE_SENTINEL)
            .bind(format!("books/{book_id}/original.pdf"))
            .bind("2026-08-06T00:00:00.000Z")
            .bind("2026-08-06T00:00:00.000Z")
            .execute(database.pool())
            .await
            .expect("synthetic book row");
            sqlx::query(
                "INSERT INTO books (id, sha256, title, author, language, format, original_filename, stored_path, import_status, import_error_code, import_error_message, import_error_stage, created_at, updated_at, reading_progress) VALUES (?, NULL, 'FAILED_COPY_METADATA_SENTINEL', NULL, NULL, 'epub', 'failed.epub', NULL, 'failed', 'LOCAL_IO_ERROR', 'safe synthetic failure', 'copying', ?, ?, 0)",
            )
            .bind("78f25a9a-1ac6-4a5d-b329-a1d49a549001")
            .bind("2026-08-06T00:00:00.000Z")
            .bind("2026-08-06T00:00:00.000Z")
            .execute(database.pool())
            .await
            .expect("synthetic pre-copy failure row");
            sqlx::query("UPDATE app_settings SET ui_language = 'zh-TW' WHERE id = 1")
                .execute(database.pool())
                .await
                .expect("synthetic setting");
            sqlx::query(
                "INSERT INTO provider_remote_resources (id, provider_kind, encrypted_reference, cleanup_status, created_at, updated_at) VALUES (?, 'openai', ?, 'pending', ?, ?)",
            )
            .bind("1c88dffe-1424-4cbe-a1a3-b264139c6768")
            .bind(REMOTE_REFERENCE_SENTINEL)
            .bind("2026-08-06T00:00:00.000Z")
            .bind("2026-08-06T00:00:00.000Z")
            .execute(database.pool())
            .await
            .expect("synthetic remote resource");
        });

        Self {
            temporary,
            database,
            paths,
            gate: MaintenanceGate::default(),
            book_id,
            source,
        }
    }

    fn destination(&self, name: &str) -> PathBuf {
        self.temporary.path().join(name)
    }

    fn service(&self) -> BackupService {
        BackupService::new(
            self.database.pool().clone(),
            self.paths.clone(),
            self.gate.clone(),
        )
    }

    fn service_with_fault(&self, fault: Arc<dyn BackupFaultInjector>) -> BackupService {
        BackupService::with_fault_injector(
            self.database.pool().clone(),
            self.paths.clone(),
            self.gate.clone(),
            fault,
        )
    }
}

#[test]
fn backup_is_single_verified_archive_with_sanitized_sqlite_and_durable_owned_files() {
    let fixture = BackupFixture::new();
    let destination = fixture.destination("textbooklens.tlbackup");
    let source_before = fs::read(&fixture.source).unwrap();

    let summary =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect("backup succeeds");

    assert_eq!(summary.format_version, BACKUP_FORMAT_VERSION);
    assert_eq!(summary.entry_count, 3);
    assert_eq!(
        summary.archive_bytes,
        fs::metadata(&destination).unwrap().len()
    );
    assert_eq!(fs::read(&fixture.source).unwrap(), source_before);
    assert_no_temporary_archives(fixture.temporary.path());

    let verified = verify_archive_file(&destination).expect("archive verifies");
    assert_eq!(verified.archive_bytes, summary.archive_bytes);
    assert_eq!(verified.entry_count, 3);

    let (archive, manifest) = read_archive(&destination);
    assert!(contains_bytes(&archive, SOURCE_SENTINEL));
    assert!(contains_bytes(&archive, DERIVED_SENTINEL));
    assert!(contains_bytes(&archive, BOOK_TITLE_SENTINEL.as_bytes()));
    for forbidden in [
        CACHE_SENTINEL,
        LOG_CREDENTIAL_SENTINEL,
        REMOTE_REFERENCE_SENTINEL.as_bytes(),
    ] {
        assert!(!contains_bytes(&archive, forbidden));
    }
    assert_eq!(manifest.entries[0].path, "library.sqlite3");
    assert!(
        manifest
            .entries
            .iter()
            .any(|entry| entry.path == format!("books/{}/original.pdf", fixture.book_id))
    );
    assert!(manifest.entries.iter().all(|entry| {
        !entry.path.starts_with("cache/")
            && !entry.path.starts_with("logs/")
            && !entry.path.contains(':')
    }));

    let database_entry = &manifest.entries[0];
    let start = usize::try_from(database_entry.offset).unwrap();
    let end = usize::try_from(database_entry.offset + database_entry.size).unwrap();
    let extracted_path = fixture.destination("extracted.sqlite3");
    fs::write(&extracted_path, &archive[start..end]).unwrap();
    let extracted = Database::open(&extracted_path).expect("sanitized database opens");
    tauri::async_runtime::block_on(async {
        let title: String = sqlx::query_scalar("SELECT title FROM books WHERE id = ?")
            .bind(fixture.book_id.to_string())
            .fetch_one(extracted.pool())
            .await
            .unwrap();
        let language: String =
            sqlx::query_scalar("SELECT ui_language FROM app_settings WHERE id = 1")
                .fetch_one(extracted.pool())
                .await
                .unwrap();
        let remote_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM provider_remote_resources")
                .fetch_one(extracted.pool())
                .await
                .unwrap();
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(extracted.pool())
            .await
            .unwrap();
        assert_eq!(title, BOOK_TITLE_SENTINEL);
        assert_eq!(language, "zh-TW");
        assert_eq!(remote_count, 0);
        assert_eq!(integrity, "ok");
    });
}

#[test]
fn active_normal_operation_is_reported_without_cancellation_or_output() {
    let fixture = BackupFixture::new();
    let destination = fixture.destination("busy.tlbackup");
    let permit = fixture
        .gate
        .try_acquire_normal(ActiveOperationKind::Import)
        .unwrap();

    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("active import blocks backup");

    let BackupError::Gate(GateAcquireError::Busy(status)) = error else {
        panic!("backup must preserve the gate's bounded busy status");
    };
    assert_eq!(status.code, MaintenanceStatusCode::NormalOperationsActive);
    assert_eq!(status.active_operations.len(), 1);
    assert_eq!(
        status.active_operations[0].kind,
        ActiveOperationKind::Import
    );
    assert!(!destination.exists());
    assert_eq!(fixture.gate.status().active_operations.len(), 1);
    drop(permit);
    assert_eq!(
        fixture.gate.status().code,
        MaintenanceStatusCode::MaintenanceAvailable
    );
}

#[test]
fn invalid_and_existing_destinations_fail_closed_without_exposing_paths() {
    let fixture = BackupFixture::new();
    let layout = ValidatedAppLayout::from_paths(&fixture.paths).unwrap();
    let existing = fixture.destination("existing.tlbackup");
    fs::write(&existing, b"keep-existing-backup").unwrap();
    let invalid = [
        PathBuf::from("relative.tlbackup"),
        fixture.destination("wrong.zip"),
        fixture.paths.root.join("inside.tlbackup"),
        fixture.destination("bad*.tlbackup"),
        fixture.destination("%TEMP%.tlbackup"),
        fixture.destination(&format!("{}.tlbackup", "a".repeat(33_000))),
        PathBuf::from(r"\\server\share\backup.tlbackup"),
        PathBuf::from(r"\\?\C:\backup.tlbackup"),
    ];
    for candidate in invalid {
        let error = ValidatedDestination::new(&candidate, &layout)
            .err()
            .expect("unsafe destination rejected");
        assert_eq!(error.code, MaintenanceErrorCode::BackupDestinationInvalid);
        let rendered = format!("{error:?}");
        assert!(!rendered.contains(&candidate.to_string_lossy().to_string()));
    }
    let error = ValidatedDestination::new(&existing, &layout).err().unwrap();
    assert_eq!(error.code, MaintenanceErrorCode::BackupDestinationExists);
    assert_eq!(fs::read(&existing).unwrap(), b"keep-existing-backup");

    let dto = MaintenanceErrorDto::from(BackupError::Archive(error));
    assert_eq!(
        serde_json::to_value(dto).unwrap(),
        serde_json::json!({
            "code": "BACKUP_DESTINATION_EXISTS",
            "activeOperations": []
        })
    );
    let summary = BackupSummaryDto {
        format_version: 1,
        archive_bytes: 200,
        entry_count: 3,
    };
    assert_eq!(
        serde_json::to_value(summary).unwrap(),
        serde_json::json!({
            "formatVersion": 1,
            "archiveBytes": 200,
            "entryCount": 3
        })
    );
}

#[test]
fn every_backup_fault_boundary_cleans_output_and_releases_the_exclusive_lease() {
    for boundary in [
        BackupBoundary::DatabaseSnapshotReady,
        BackupBoundary::SourcesValidated,
        BackupBoundary::HeaderWritten,
        BackupBoundary::EntriesWritten,
        BackupBoundary::ArchiveSynced,
        BackupBoundary::ArchiveVerified,
        BackupBoundary::BeforePublish,
    ] {
        let fixture = BackupFixture::new();
        let destination = fixture.destination("faulted.tlbackup");
        let fault = Arc::new(FailAt(boundary));

        let error = tauri::async_runtime::block_on(
            fixture
                .service_with_fault(fault)
                .create_backup(destination.clone()),
        )
        .expect_err("injected failure");

        assert_eq!(
            archive_error_code(error),
            MaintenanceErrorCode::BackupWriteFailed,
            "wrong error at {boundary:?}"
        );
        assert!(!destination.exists(), "published output at {boundary:?}");
        assert_no_temporary_archives(fixture.temporary.path());
        assert_eq!(
            fixture.gate.status().code,
            MaintenanceStatusCode::MaintenanceAvailable
        );
    }
}

#[test]
fn publication_race_never_overwrites_the_winning_destination() {
    let fixture = BackupFixture::new();
    let destination = fixture.destination("claimed.tlbackup");
    let fault = Arc::new(CreateDestinationAtPublish {
        destination: destination.clone(),
        claimed: Mutex::new(false),
    });

    let error = tauri::async_runtime::block_on(
        fixture
            .service_with_fault(fault)
            .create_backup(destination.clone()),
    )
    .expect_err("racing destination wins without overwrite");

    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupDestinationExists
    );
    assert_eq!(fs::read(&destination).unwrap(), b"race-winner");
    assert_no_temporary_archives(fixture.temporary.path());
}

#[test]
fn source_identity_change_and_hard_link_alias_are_rejected() {
    let fixture = BackupFixture::new();
    let replacement = fixture.destination("replacement.bin");
    fs::write(&replacement, b"MALICIOUS_REPLACEMENT_SENTINEL").unwrap();
    let destination = fixture.destination("raced.tlbackup");
    let fault = Arc::new(ReplaceSourceAfterScan {
        source: fixture.source.clone(),
        replacement,
        replaced: Mutex::new(false),
    });
    let error = tauri::async_runtime::block_on(
        fixture
            .service_with_fault(fault)
            .create_backup(destination.clone()),
    )
    .expect_err("identity replacement rejected");
    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupSourceUnsafe
    );
    assert!(!destination.exists());
    assert_no_temporary_archives(fixture.temporary.path());

    let fixture = BackupFixture::new();
    let outside = fixture.destination("outside-owned-alias.bin");
    fs::write(&outside, b"OUTSIDE_HARD_LINK_SENTINEL").unwrap();
    let linked = fixture
        .paths
        .books
        .join(fixture.book_id.to_string())
        .join("derived")
        .join("alias.bin");
    fs::hard_link(&outside, &linked).unwrap();
    let destination = fixture.destination("hardlink.tlbackup");
    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("hard-linked source is not exclusively app-owned");
    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupSourceUnsafe
    );
    assert_eq!(fs::read(&outside).unwrap(), b"OUTSIDE_HARD_LINK_SENTINEL");
    assert!(!destination.exists());
}

#[test]
fn unknown_owned_root_entries_are_rejected_instead_of_silently_omitted() {
    let fixture = BackupFixture::new();
    fs::write(
        fixture
            .paths
            .books
            .join(fixture.book_id.to_string())
            .join("unexpected.bin"),
        b"UNCLASSIFIED_PRIVATE_SENTINEL",
    )
    .unwrap();
    let destination = fixture.destination("unsafe-source.tlbackup");

    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("unknown durable entries fail closed");

    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupSourceUnsafe
    );
    assert!(!destination.exists());
    assert_no_temporary_archives(fixture.temporary.path());

    let fixture = BackupFixture::new();
    fs::write(
        fixture
            .paths
            .books
            .join(fixture.book_id.to_string())
            .join("derived")
            .join(".document.html.partial"),
        b"INTERRUPTED_DERIVED_SENTINEL",
    )
    .unwrap();
    let destination = fixture.destination("partial-derived.tlbackup");
    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("derived partial is temporary, not durable backup data");
    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupSourceUnsafe
    );
    assert!(!destination.exists());

    let fixture = BackupFixture::new();
    let orphan = fixture
        .paths
        .books
        .join("b31e17b7-a18e-477d-8021-af4529057296");
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("original.pdf"), b"ORPHAN_SOURCE_SENTINEL").unwrap();
    let destination = fixture.destination("orphan-book.tlbackup");
    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("UUID-shaped orphan is not database-owned");
    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupSourceUnsafe
    );
    assert!(!destination.exists());

    let fixture = BackupFixture::new();
    fs::write(&fixture.source, b"TAMPERED_OWNED_SOURCE_SENTINEL").unwrap();
    let destination = fixture.destination("tampered-source.tlbackup");
    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("database source hash is authoritative");
    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupSourceUnsafe
    );
    assert!(!destination.exists());
}

#[test]
fn manifest_contract_is_strict_bounded_and_traversal_free() {
    let book_id = "7cc38f8b-3db0-4728-9821-9da5aa3ce35d";
    let mut manifest = BackupManifest {
        format: FORMAT_IDENTIFIER.to_owned(),
        version: BACKUP_FORMAT_VERSION,
        entry_count: 2,
        total_bytes: 5,
        entries: vec![
            BackupManifestEntry {
                path: "library.sqlite3".to_owned(),
                offset: HEADER_BYTES,
                size: 4,
                sha256: "0".repeat(64),
            },
            BackupManifestEntry {
                path: format!("books/{book_id}/derived/document.html"),
                offset: HEADER_BYTES + 4,
                size: 1,
                sha256: "a".repeat(64),
            },
        ],
    };
    validate_manifest(&manifest, HEADER_BYTES + 5).unwrap();

    for unsafe_path in [
        "../library.sqlite3",
        "/absolute/library.sqlite3",
        r"books\7cc38f8b-3db0-4728-9821-9da5aa3ce35d\original.pdf",
        "books/7cc38f8b-3db0-4728-9821-9da5aa3ce35d/derived/../escape",
        "books/7cc38f8b-3db0-4728-9821-9da5aa3ce35d/derived/CON.txt",
        "books/7cc38f8b-3db0-4728-9821-9da5aa3ce35d/derived/section.txt",
        "books/7cc38f8b-3db0-4728-9821-9da5aa3ce35d/derived/nested/document.html",
        "books/7CC38F8B-3DB0-4728-9821-9DA5AA3CE35D/original.pdf",
        "cache/private.bin",
        "logs/private.log",
    ] {
        manifest.entries[1].path = unsafe_path.to_owned();
        assert_eq!(
            validate_manifest(&manifest, HEADER_BYTES + 5)
                .unwrap_err()
                .code,
            MaintenanceErrorCode::BackupVerificationFailed,
            "unsafe path should fail"
        );
    }
    manifest.entries[1].path = format!("books/{book_id}/derived/document.html");
    manifest.entries[1].offset += 1;
    assert!(validate_manifest(&manifest, HEADER_BYTES + 5).is_err());
    manifest.entries[1].offset -= 1;
    manifest.entries[1].sha256 = "A".repeat(64);
    assert!(validate_manifest(&manifest, HEADER_BYTES + 5).is_err());
    manifest.entries[1].sha256 = "a".repeat(64);
    manifest.total_bytes += 1;
    assert!(validate_manifest(&manifest, HEADER_BYTES + 5).is_err());

    let mut duplicate = BackupManifest {
        format: FORMAT_IDENTIFIER.to_owned(),
        version: BACKUP_FORMAT_VERSION,
        entry_count: 3,
        total_bytes: 6,
        entries: vec![
            BackupManifestEntry {
                path: "library.sqlite3".to_owned(),
                offset: HEADER_BYTES,
                size: 4,
                sha256: "0".repeat(64),
            },
            BackupManifestEntry {
                path: format!("books/{book_id}/derived/document.html"),
                offset: HEADER_BYTES + 4,
                size: 1,
                sha256: "a".repeat(64),
            },
            BackupManifestEntry {
                path: format!("books/{book_id}/derived/document.html"),
                offset: HEADER_BYTES + 5,
                size: 1,
                sha256: "b".repeat(64),
            },
        ],
    };
    assert!(validate_manifest(&duplicate, HEADER_BYTES + 6).is_err());
    duplicate.entries[2].path = format!("books/{book_id}/original.pdf");
    duplicate.entries[2].size = MAX_ENTRY_BYTES + 1;
    duplicate.total_bytes = 5 + MAX_ENTRY_BYTES + 1;
    assert!(validate_manifest(&duplicate, HEADER_BYTES + duplicate.total_bytes).is_err());

    let unknown_field = br#"{
        "format":"textbooklens-local-backup",
        "version":1,
        "entryCount":0,
        "totalBytes":0,
        "entries":[],
        "sourcePath":"private"
    }"#;
    assert!(serde_json::from_slice::<BackupManifest>(unknown_field).is_err());
}

#[test]
fn verifier_rejects_header_payload_footer_manifest_tail_and_truncation_corruption() {
    let fixture = BackupFixture::new();
    let valid = fixture.destination("valid.tlbackup");
    tauri::async_runtime::block_on(fixture.service().create_backup(valid.clone())).unwrap();
    let bytes = fs::read(&valid).unwrap();

    let mut payload = bytes.clone();
    payload[usize::try_from(HEADER_BYTES).unwrap()] ^= 0x01;
    let corrupted_payload = fixture.destination("corrupt-payload.tlbackup");
    fs::write(&corrupted_payload, payload).unwrap();
    assert_verification_failure(&corrupted_payload);

    let mut footer = bytes.clone();
    let footer_start = footer.len() - usize::try_from(FOOTER_BYTES).unwrap();
    footer[footer_start] ^= 0x01;
    let corrupted_footer = fixture.destination("corrupt-footer.tlbackup");
    fs::write(&corrupted_footer, footer).unwrap();
    assert_verification_failure(&corrupted_footer);

    let truncated = fixture.destination("truncated.tlbackup");
    fs::write(&truncated, &bytes[..bytes.len() - 1]).unwrap();
    assert_verification_failure(&truncated);

    let mut header = bytes.clone();
    header[8..12].copy_from_slice(&(BACKUP_FORMAT_VERSION + 1).to_le_bytes());
    let incompatible = fixture.destination("incompatible-version.tlbackup");
    fs::write(&incompatible, header).unwrap();
    assert_verification_failure(&incompatible);

    let mut flags = bytes.clone();
    flags[12..16].copy_from_slice(&1_u32.to_le_bytes());
    let unsupported_flags = fixture.destination("unsupported-flags.tlbackup");
    fs::write(&unsupported_flags, flags).unwrap();
    assert_verification_failure(&unsupported_flags);

    let mut manifest_length = bytes.clone();
    let footer_start = manifest_length.len() - usize::try_from(FOOTER_BYTES).unwrap();
    manifest_length[footer_start + 8..footer_start + 16]
        .copy_from_slice(&(MAX_MANIFEST_BYTES + 1).to_le_bytes());
    let oversized_manifest = fixture.destination("oversized-manifest.tlbackup");
    fs::write(&oversized_manifest, manifest_length).unwrap();
    assert_verification_failure(&oversized_manifest);

    let mut tailed = bytes;
    tailed.extend_from_slice(b"FORBIDDEN_TRAILING_SENTINEL");
    let trailing = fixture.destination("trailing-data.tlbackup");
    fs::write(&trailing, tailed).unwrap();
    assert_verification_failure(&trailing);
}

#[cfg(windows)]
#[test]
fn source_and_destination_junctions_are_rejected_without_touching_decoys() {
    let fixture = BackupFixture::new();
    let outside = fixture.destination("outside-books");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("private-decoy.bin"), b"OUTSIDE_UNTOUCHED").unwrap();
    fs::remove_dir_all(&fixture.paths.books).unwrap();
    create_windows_junction(&outside, &fixture.paths.books);
    let destination = fixture.destination("junction-source.tlbackup");

    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("source junction rejected");

    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupSourceUnsafe
    );
    assert_eq!(
        fs::read(outside.join("private-decoy.bin")).unwrap(),
        b"OUTSIDE_UNTOUCHED"
    );
    assert!(!destination.exists());

    let fixture = BackupFixture::new();
    let outside = fixture.destination("outside-destination");
    fs::create_dir(&outside).unwrap();
    fs::write(
        outside.join("existing.bin"),
        b"OUTSIDE_DESTINATION_UNTOUCHED",
    )
    .unwrap();
    let junction = fixture.destination("destination-junction");
    create_windows_junction(&outside, &junction);
    let destination = junction.join("backup.tlbackup");
    let error =
        tauri::async_runtime::block_on(fixture.service().create_backup(destination.clone()))
            .expect_err("destination junction rejected");
    assert_eq!(
        archive_error_code(error),
        MaintenanceErrorCode::BackupDestinationInvalid
    );
    assert_eq!(
        fs::read(outside.join("existing.bin")).unwrap(),
        b"OUTSIDE_DESTINATION_UNTOUCHED"
    );
    assert!(!outside.join("backup.tlbackup").exists());
}

#[derive(Clone, Copy)]
struct FailAt(BackupBoundary);

impl BackupFaultInjector for FailAt {
    fn checkpoint(&self, boundary: BackupBoundary) -> Result<(), BackupInjectedFailure> {
        if boundary == self.0 {
            Err(BackupInjectedFailure)
        } else {
            Ok(())
        }
    }
}

struct CreateDestinationAtPublish {
    destination: PathBuf,
    claimed: Mutex<bool>,
}

impl BackupFaultInjector for CreateDestinationAtPublish {
    fn checkpoint(&self, boundary: BackupBoundary) -> Result<(), BackupInjectedFailure> {
        if boundary == BackupBoundary::BeforePublish {
            let mut claimed = self.claimed.lock().unwrap();
            if !*claimed {
                fs::write(&self.destination, b"race-winner").unwrap();
                *claimed = true;
            }
        }
        Ok(())
    }
}

struct ReplaceSourceAfterScan {
    source: PathBuf,
    replacement: PathBuf,
    replaced: Mutex<bool>,
}

impl BackupFaultInjector for ReplaceSourceAfterScan {
    fn checkpoint(&self, boundary: BackupBoundary) -> Result<(), BackupInjectedFailure> {
        if boundary == BackupBoundary::SourcesValidated {
            let mut replaced = self.replaced.lock().unwrap();
            if !*replaced {
                fs::remove_file(&self.source).unwrap();
                fs::rename(&self.replacement, &self.source).unwrap();
                *replaced = true;
            }
        }
        Ok(())
    }
}

fn archive_error_code(error: BackupError) -> MaintenanceErrorCode {
    match error {
        BackupError::Archive(error) => error.code,
        BackupError::Gate(_) => panic!("expected archive error"),
    }
}

fn read_archive(path: &Path) -> (Vec<u8>, BackupManifest) {
    let bytes = fs::read(path).unwrap();
    let footer_start = bytes.len() - usize::try_from(FOOTER_BYTES).unwrap();
    let manifest_len = u64::from_le_bytes(
        bytes[footer_start + 8..footer_start + 16]
            .try_into()
            .unwrap(),
    );
    let manifest_start = footer_start - usize::try_from(manifest_len).unwrap();
    let manifest = serde_json::from_slice(&bytes[manifest_start..footer_start]).unwrap();
    (bytes, manifest)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn assert_no_temporary_archives(parent: &Path) {
    let leftovers = fs::read_dir(parent)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
        .into_iter()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(TEMP_FILE_PREFIX)
        })
        .count();
    assert_eq!(leftovers, 0);
}

fn assert_verification_failure(path: &Path) {
    assert_eq!(
        verify_archive_file(path).unwrap_err().code,
        MaintenanceErrorCode::BackupVerificationFailed
    );
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
        "junction creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
