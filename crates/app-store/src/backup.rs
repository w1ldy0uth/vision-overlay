//! Versioned, bounded portable backups. Database copies use SQLite Backup API.
use crate::{Store, core::Record, legacy::Action};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const MAX_DB: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_ARCHIVE: usize = 512 * 1024 * 1024;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub locale: String,
    pub selected_profile_id: Option<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            locale: "ru".into(),
            selected_profile_id: None,
        }
    }
}
impl Settings {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.locale == "ru",
            "Unsupported settings format"
        );
        if let Some(id) = &self.selected_profile_id {
            Uuid::parse_str(id)?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileInfo {
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: String,
    pub format_version: u32,
    pub application_version: String,
    pub database_schema_version: i64,
    pub created_at: String,
    pub revision: String,
    pub profiles: Value,
    pub files: BTreeMap<String, FileInfo>,
}
pub struct Validated {
    pub manifest: Manifest,
    pub settings: Settings,
    pub catalog: Value,
    temp: tempfile::TempDir,
}
impl Validated {
    pub fn database(&self) -> PathBuf {
        self.temp.path().join("database.sqlite3")
    }
}
fn file_info(path: &Path) -> Result<FileInfo> {
    let mut f = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = f.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(FileInfo {
        size: f.metadata()?.len(),
        sha256: format!("{:x}", hash.finalize()),
    })
}
async fn connect(path: &Path, read_only: bool) -> Result<SqliteConnection> {
    Ok(SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(path)
            .read_only(read_only)
            .create_if_missing(!read_only)
            .busy_timeout(std::time::Duration::from_secs(5)),
    )
    .await?)
}
/// Handles remain exclusively locked for the entire atomic SQLite copy. finish
/// rolls the destination transaction back if step fails (including SQLITE_BUSY).
async fn copy(source: &mut SqliteConnection, dest: &mut SqliteConnection) -> Result<()> {
    let mut source = source.lock_handle().await?;
    let mut dest = dest.lock_handle().await?;
    // SAFETY: both connections are alive, exclusively locked and their main DB
    // names are static NUL-terminated strings. The backup is finished exactly once.
    let (step, finish) = unsafe {
        let backup = libsqlite3_sys::sqlite3_backup_init(
            dest.as_raw_handle().as_ptr(),
            c"main".as_ptr(),
            source.as_raw_handle().as_ptr(),
            c"main".as_ptr(),
        );
        ensure!(
            !backup.is_null(),
            "Cannot start SQLite backup (database may be busy)"
        );
        let step = libsqlite3_sys::sqlite3_backup_step(backup, -1);
        (step, libsqlite3_sys::sqlite3_backup_finish(backup))
    };
    ensure!(
        step == libsqlite3_sys::SQLITE_DONE && finish == libsqlite3_sys::SQLITE_OK,
        "SQLite backup failed: step={step}, finish={finish}"
    );
    Ok(())
}
async fn schema(db: &mut SqliteConnection) -> Result<Vec<(String, String, Option<String>)>> {
    Ok(sqlx::query_as(
        "SELECT type,name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",
    )
    .fetch_all(db)
    .await?)
}
async fn validate_database(path: &Path, version: i64) -> Result<()> {
    ensure!(
        (2..=crate::DATABASE_SCHEMA_VERSION).contains(&version),
        "Unsupported database version"
    );
    let expected_dir = tempfile::tempdir()?;
    let mut expected = Store::open(&expected_dir.path().join("expected.sqlite3")).await?;
    let mut expected_schema = schema(&mut expected.connection).await?;
    expected.close().await?;
    if version == 2 {
        expected_schema.retain(|(_, name, _)| {
            !matches!(
                name.as_str(),
                "portable_preferences" | "restored_capture_guards" | "restore_barrier"
            )
        });
    }
    if version < 4 {
        expected_schema.retain(|(_, name, _)| !crate::retention::TABLES.contains(&name.as_str()));
    }
    if version < 5 {
        expected_schema
            .retain(|(_, name, _)| !matches!(name.as_str(), "wish_records" | "wish_imports"));
    }
    if version < 6 {
        expected_schema.retain(|(_, name, _)| name != "wish_sync_state");
    }
    if version < 7 {
        expected_schema.retain(|(_, name, _)| name != "planner_goals");
    }
    if version < 8 {
        expected_schema
            .retain(|(_, name, _)| !matches!(name.as_str(), "wish_live" | "wish_live_api_match"));
    }
    if version < 9 {
        expected_schema.retain(|(_, name, _)| {
            !matches!(name.as_str(), "planner_presets" | "planner_preset_goals")
        });
    }
    let mut db = connect(path, true).await?;
    ensure!(
        schema(&mut db).await? == expected_schema,
        "Database schema differs from supported application schema"
    );
    let integrity: Vec<(String,)> = sqlx::query_as("PRAGMA integrity_check")
        .fetch_all(&mut db)
        .await?;
    ensure!(
        integrity == vec![("ok".into(),)],
        "Database integrity check failed"
    );
    ensure!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut db)
            .await?
            .is_empty(),
        "Broken foreign keys"
    );
    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success=1 ORDER BY version")
            .fetch_all(&mut db)
            .await?;
    ensure!(
        versions == (1..=version).collect::<Vec<_>>(),
        "Migration version mismatch"
    );
    db.close().await?;
    // SQLx also verifies migration checksums; upgrade supported v2 copies in staging.
    Store::open(path).await?.close().await?;
    Ok(())
}
pub async fn validate(path: &Path) -> Result<Validated> {
    ensure!(
        fs::metadata(path)?.len() <= MAX_ARCHIVE as u64,
        "Backup archive too large"
    );
    let mut archive = ZipArchive::new(fs::File::open(path)?)?;
    ensure!(archive.len() == 4, "Expected exactly four archive entries");
    let mut names = std::collections::BTreeSet::new();
    for i in 0..archive.len() {
        let f = archive.by_index(i)?;
        ensure!(
            matches!(
                f.name(),
                "manifest.json" | "database.sqlite3" | "settings.json" | "catalog.json"
            ) && names.insert(f.name().to_owned()),
            "Unknown or duplicate archive entry"
        );
    }
    let mut bytes = Vec::new();
    archive
        .by_name("manifest.json")?
        .take(65537)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "Manifest too large");
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    ensure!(
        manifest.format == "vision-overlay-backup"
            && manifest.format_version == 1
            && manifest.files.len() == 3,
        "Unsupported backup format"
    );
    let temp = tempfile::tempdir()?;
    for name in ["database.sqlite3", "settings.json", "catalog.json"] {
        let expected = manifest.files.get(name).context("Missing manifest entry")?;
        let file = archive.by_name(name)?;
        let limit = if name == "database.sqlite3" {
            MAX_DB
        } else {
            65536
        };
        ensure!(
            file.size() == expected.size && file.size() <= limit,
            "Invalid archive entry size"
        );
        let path = temp.path().join(name);
        let mut out = fs::File::create(&path)?;
        let copied = std::io::copy(&mut file.take(limit + 1), &mut out)?;
        out.sync_all()?;
        ensure!(
            copied == expected.size && file_info(&path)?.sha256 == expected.sha256,
            "Backup checksum mismatch"
        );
    }
    let settings: Settings = serde_json::from_slice(&fs::read(temp.path().join("settings.json"))?)?;
    settings.validate()?;
    let catalog: Value = serde_json::from_slice(&fs::read(temp.path().join("catalog.json"))?)?;
    ensure!(
        catalog["schema_version"] == 1 && catalog.is_object(),
        "Unsupported catalog metadata"
    );
    validate_database(
        &temp.path().join("database.sqlite3"),
        manifest.database_schema_version,
    )
    .await?;
    let mut db = Store::open(&temp.path().join("database.sqlite3")).await?;
    ensure!(
        db.profiles().await? == manifest.profiles,
        "Manifest profile list differs from database"
    );
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
        .fetch_one(&mut db.connection)
        .await?;
    ensure!(
        revision.to_string() == manifest.revision,
        "Manifest revision differs from database"
    );
    if let Some(id) = &settings.selected_profile_id {
        ensure!(
            manifest
                .profiles
                .as_array()
                .context("Invalid profiles")?
                .iter()
                .any(|p| p["profile_id"] == *id),
            "Selected profile is absent"
        );
    }
    db.close().await?;
    Ok(Validated {
        manifest,
        settings,
        catalog,
        temp,
    })
}
pub async fn package(
    temp: tempfile::TempDir,
    output: &Path,
    settings: Settings,
    catalog: Value,
) -> Result<Manifest> {
    settings.validate()?;
    ensure!(!output.exists(), "Backup destination already exists");
    ensure!(
        catalog["schema_version"] == 1 && serde_json::to_vec(&catalog)?.len() <= 65536,
        "Invalid catalog metadata"
    );
    let database = temp.path().join("database.sqlite3");
    let mut snapshot = Store::open(&database).await?;
    let profiles = snapshot.profiles().await?;
    if let Some(id) = &settings.selected_profile_id {
        ensure!(
            profiles
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["profile_id"] == *id),
            "Selected profile is absent"
        );
    }
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
        .fetch_one(&mut snapshot.connection)
        .await?;
    snapshot.close().await?;
    fs::write(
        temp.path().join("settings.json"),
        serde_json::to_vec(&settings)?,
    )?;
    fs::write(
        temp.path().join("catalog.json"),
        serde_json::to_vec(&catalog)?,
    )?;
    let mut files = BTreeMap::new();
    for name in ["database.sqlite3", "settings.json", "catalog.json"] {
        files.insert(name.into(), file_info(&temp.path().join(name))?);
    }
    ensure!(
        files["database.sqlite3"].size <= MAX_DB,
        "Database too large"
    );
    let manifest = Manifest {
        format: "vision-overlay-backup".into(),
        format_version: 1,
        application_version: env!("CARGO_PKG_VERSION").into(),
        database_schema_version: crate::DATABASE_SCHEMA_VERSION,
        created_at: time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)?,
        revision: revision.to_string(),
        profiles,
        files,
    };
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut archive_file = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut zip = ZipWriter::new(archive_file.as_file_mut());
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("manifest.json", options)?;
        zip.write_all(&serde_json::to_vec(&manifest)?)?;
        for name in ["database.sqlite3", "settings.json", "catalog.json"] {
            zip.start_file(name, options)?;
            std::io::copy(&mut fs::File::open(temp.path().join(name))?, &mut zip)?;
        }
        zip.finish()?;
    }
    archive_file.as_file().sync_all()?;
    validate(archive_file.path()).await?;
    archive_file
        .persist_noclobber(output)
        .map_err(|e| e.error)?;
    Ok(manifest)
}

impl Store {
    pub async fn backup_snapshot(&mut self) -> Result<tempfile::TempDir> {
        let temp = tempfile::tempdir()?;
        let mut dest = connect(&temp.path().join("database.sqlite3"), false).await?;
        copy(&mut self.connection, &mut dest).await?;
        dest.close().await?;
        Ok(temp)
    }
    pub async fn backup(
        &mut self,
        output: &Path,
        settings: Settings,
        catalog: Value,
    ) -> Result<Manifest> {
        ensure!(!output.exists(), "Backup destination already exists");
        let snapshot = self.backup_snapshot().await?;
        package(snapshot, output, settings, catalog).await
    }
    pub async fn capture_blocked(&mut self, id: &str) -> Result<bool> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM restored_capture_guards WHERE capture_id=?",
        )
        .bind(id)
        .fetch_one(&mut self.connection)
        .await?
            > 0)
    }
    pub async fn restore(
        &mut self,
        archive: &Validated,
        rollback: &Path,
        blocked: &[String],
    ) -> Result<Value> {
        let active: i64 = sqlx::query_scalar("SELECT count(*) FROM live_leases WHERE expires_ms>?")
            .bind(crate::ingest::now_ms())
            .fetch_one(&mut self.connection)
            .await?;
        ensure!(active == 0, "Stop capture before restoring");
        let previous_revision: i64 =
            sqlx::query_scalar("SELECT revision FROM store_meta WHERE singleton=1")
                .fetch_one(&mut self.connection)
                .await?;
        let mut guards: Vec<String> = sqlx::query_scalar(
            "SELECT capture_id FROM captures UNION SELECT capture_id FROM restored_capture_guards",
        )
        .fetch_all(&mut self.connection)
        .await?;
        guards.extend(blocked.iter().cloned());
        self.backup(
            rollback,
            Settings::default(),
            json!({"schema_version":1,"reason":"before_restore"}),
        )
        .await?;
        // Prepare every restoration mutation before touching the live destination.
        let prepared = tempfile::tempdir()?;
        let path = prepared.path().join("prepared.sqlite3");
        let mut source = connect(&archive.database(), true).await?;
        let mut target = connect(&path, false).await?;
        copy(&mut source, &mut target).await?;
        source.close().await?;
        target.close().await?;
        let mut staged = Store::open(&path).await?;
        let mut tx = staged.connection.begin().await?;
        guards.extend(sqlx::query_scalar::<_,String>("SELECT capture_id FROM captures UNION SELECT capture_id FROM restored_capture_guards").fetch_all(&mut *tx).await?);
        sqlx::query("INSERT OR REPLACE INTO restore_barrier VALUES(1,?)")
            .bind(crate::ingest::now_ms())
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE retention_artifacts SET pinned=1,state='missing'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE automatic_backups SET state='missing'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM retention_jobs")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM retention_meta")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM live_leases")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE live_checkpoints SET closed=1,current_epoch=NULL")
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE active_context SET profile_id=NULL,capture_id=NULL,epoch=NULL WHERE singleton=1").execute(&mut *tx).await?;
        sqlx::query("UPDATE domain_state SET freshness='saved',reason='restored_backup'")
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT OR REPLACE INTO portable_preferences VALUES(1,?)")
            .bind(serde_json::to_string(&archive.settings)?)
            .execute(&mut *tx)
            .await?;
        for guard in guards {
            Uuid::parse_str(&guard)?;
            sqlx::query("INSERT OR IGNORE INTO restored_capture_guards VALUES(?)")
                .bind(guard)
                .execute(&mut *tx)
                .await?;
        }
        let mut revision = previous_revision.max(archive.manifest.revision.parse()?);
        sqlx::query("UPDATE store_meta SET revision=? WHERE singleton=1")
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        let system = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO captures VALUES(?,?,?,?,?,1,'system',strftime('%Y-%m-%dT%H:%M:%fZ','now'))").bind(&system).bind(format!("system:{system}")).bind("{}").bind("{\"mode\":\"system\"}").bind("backup-v1").execute(&mut *tx).await?;
        sqlx::query("INSERT INTO epochs VALUES(?,'0',NULL,'backup_restore')")
            .bind(&system)
            .execute(&mut *tx)
            .await?;
        let record = Record {
            event: json!({"kind":"diagnostic.notice","payload":{"code":"backup_restored"}}),
            sequence: 1,
            epoch: "0".into(),
            observed_ms: Some(crate::ingest::now_ms()),
            actions: vec![Action::Notice {
                topics: vec![
                    "profiles".into(),
                    "active_profile".into(),
                    "freshness".into(),
                ],
            }],
        };
        crate::apply_record(
            &mut tx,
            &system,
            &record,
            None,
            &mut revision,
            &mut BTreeMap::new(),
            false,
        )
        .await?;
        sqlx::query("UPDATE store_meta SET revision=? WHERE singleton=1")
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        staged.close().await?;
        let mut source = connect(&path, true).await?;
        copy(&mut source, &mut self.connection).await?;
        source.close().await?;
        self.connection.clear_cached_statements().await?;
        Ok(
            json!({"restored":true,"revision":revision.to_string(),"rollback_path":rollback,"settings":archive.settings,"requires_fresh_capture":true}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn populate(store: &mut Store, count: u32) -> String {
        let events = [
            json!({"schema_version":1,"sequence":1,"session":"0","event":"source_config","data":{"mode":"live"}}),
            json!({"schema_version":1,"sequence":2,"session":"1","observed_unix_ms":"1002","event":"inventory_snapshot","data":{"completeness":"full","materials":[{"guid":"42","item_id":100011,"count":count}],"items":[]}}),
            json!({"schema_version":1,"sequence":3,"session":"1","observed_unix_ms":"1003","event":"status","data":{"state":"stopped_game_exited"}}),
        ];
        let bytes = events.iter().map(|v| format!("{v}\n")).collect::<String>();
        let bindings = crate::legacy::Bindings::from([(
            "1".into(),
            crate::legacy::Identity {
                uid: "700000001".into(),
                server_region: "os_euro".into(),
            },
        )]);
        store.import(bytes.as_bytes(), &bindings).await.unwrap()["capture_id"]
            .as_str()
            .unwrap()
            .into()
    }
    async fn count(store: &mut Store) -> String {
        let profiles = store.profiles().await.unwrap();
        let id = profiles[0]["profile_id"].as_str().unwrap();
        store.state(id).await.unwrap()["material_totals"][0]["count"]
            .as_str()
            .unwrap()
            .into()
    }
    #[tokio::test]
    async fn archive_restores_all_facts_keeps_rollback_and_blocks_old_sources() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let mut original = Store::open(&root.join("original.db")).await.unwrap();
        let source = populate(&mut original, 353).await;
        let path = root.join("copy.visionbackup");
        original
            .backup(
                &path,
                Settings::default(),
                json!({"schema_version":1,"sources":{}}),
            )
            .await
            .unwrap();
        let archive_hash = sha256(&path);
        let archive = validate(&path).await.unwrap();
        let mut target = Store::open(&root.join("target.db")).await.unwrap();
        let old = populate(&mut target, 12).await;
        let rollback = root.join("rollback.visionbackup");
        let before_revision: i64 = sqlx::query_scalar("SELECT revision FROM store_meta")
            .fetch_one(&mut target.connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO live_leases SELECT profile_id,?,'1',? FROM profiles")
            .bind(&old)
            .bind(crate::ingest::now_ms() + 60000)
            .execute(&mut target.connection)
            .await
            .unwrap();
        assert!(target.restore(&archive, &rollback, &[]).await.is_err());
        assert!(!rollback.exists());
        assert_eq!(count(&mut target).await, "12");
        sqlx::query("DELETE FROM live_leases")
            .execute(&mut target.connection)
            .await
            .unwrap();
        let result = target.restore(&archive, &rollback, &[]).await.unwrap();
        assert_eq!(count(&mut target).await, "353");
        assert!(result["revision"].as_str().unwrap().parse::<i64>().unwrap() > before_revision);
        assert!(target.capture_blocked(&old).await.unwrap());
        assert!(target.capture_blocked(&source).await.unwrap());
        assert!(
            target
                .ingest(&old, vec![json!({})])
                .await
                .unwrap_err()
                .to_string()
                .contains("Old capture")
        );
        let old_archive = validate(&rollback).await.unwrap();
        let mut old_store = Store::open(&old_archive.database()).await.unwrap();
        assert_eq!(count(&mut old_store).await, "12");
        old_store.close().await.unwrap();
        assert!(
            sqlx::query_scalar::<_, Option<String>>("SELECT profile_id FROM active_context")
                .fetch_one(&mut target.connection)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(sha256(&path), archive_hash);
        target.close().await.unwrap();
        original.close().await.unwrap();
    }
    fn sha256(path: &Path) -> String {
        format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
    }
    #[tokio::test]
    async fn existing_archive_is_not_overwritten_and_bad_schema_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("s.db")).await.unwrap();
        let path = temp.path().join("existing.visionbackup");
        fs::write(&path, b"keep me").unwrap();
        assert!(
            store
                .backup(&path, Settings::default(), json!({"schema_version":1}))
                .await
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"keep me");
        sqlx::query(
            "CREATE TRIGGER unexpected AFTER INSERT ON profiles BEGIN DELETE FROM profiles; END",
        )
        .execute(&mut store.connection)
        .await
        .unwrap();
        assert!(
            store
                .backup(
                    &temp.path().join("bad.visionbackup"),
                    Settings::default(),
                    json!({"schema_version":1})
                )
                .await
                .is_err()
        );
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn tampered_manifest_and_path_entries_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("s.db")).await.unwrap();
        let path = temp.path().join("good.visionbackup");
        store
            .backup(&path, Settings::default(), json!({"schema_version":1}))
            .await
            .unwrap();
        for (index, mode) in ["version", "hash", "path"].iter().enumerate() {
            let mut input = ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
            let bad = temp.path().join(format!("bad{index}.visionbackup"));
            let mut output = ZipWriter::new(fs::File::create(&bad).unwrap());
            for i in 0..input.len() {
                let mut file = input.by_index(i).unwrap();
                let mut bytes = vec![];
                file.read_to_end(&mut bytes).unwrap();
                let mut name = file.name().to_owned();
                if name == "manifest.json" {
                    let mut manifest: Value = serde_json::from_slice(&bytes).unwrap();
                    if *mode == "version" {
                        manifest["format_version"] = json!(99);
                    }
                    if *mode == "hash" {
                        manifest["files"]["settings.json"]["sha256"] = json!("wrong");
                    }
                    bytes = serde_json::to_vec(&manifest).unwrap();
                }
                if name == "catalog.json" && *mode == "path" {
                    name = "../catalog.json".into();
                }
                output
                    .start_file(name, SimpleFileOptions::default())
                    .unwrap();
                output.write_all(&bytes).unwrap();
            }
            output.finish().unwrap();
            assert!(validate(&bad).await.is_err(), "{mode}");
        }
        store.close().await.unwrap();
    }
}
