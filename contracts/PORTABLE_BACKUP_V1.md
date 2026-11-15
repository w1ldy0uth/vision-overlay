# Portable backup v1 (.visionbackup)

The file is a ZIP archive with exactly four unique, fixed entries:

- manifest.json: format=vision-overlay-backup, format_version=1,
  application_version, database_schema_version, RFC3339 created_at, decimal-string
  revision, profiles (all UID/server identities), files (size and SHA256 for each payload).
- database.sqlite3: a consistent SQLite Backup API snapshot, including current
  facts, archived snapshots/events/wishes when stored, and future persisted planner data.
- settings.json: schema_version=1, locale=ru, selected_profile_id=null or an existing
  profile UUID. Currently these are the implemented portable preferences. Future
  view/overlay preferences require a compatible schema extension.
- catalog.json: schema_version=1, sources with the bundled item/avatar/achievement
  catalog revisions. Catalog assets are supplied by the application, not embedded here.

Maximum compressed archive512MiB; database2GiB; other entries64KiB each.
Unknown names, duplicate entries, invalid sizes, format versions, hashes, database
schema, foreign keys and migration checksums are rejected. Entry names are never
used as extraction paths. Schemas 2, 3 and 4 are supported; older schemas are upgraded in staging.
Schema 4 includes retention bookkeeping. On restore, local artifact references
are protected/marked missing and cleanup jobs/adoption state are reset.
SHA256 detects file damage; the archive is not signed or encrypted.

No collector token, dispatch keys, wish authkey, executable, raw packet recording,
OS task or machine-specific absolute configuration is added to the archive.
The full historical DB can contain source metadata/paths already stored there.

## Creation

A writer mutex is held only for the native SQLite snapshot in the HTTP export
path. Compression and full validation run on a blocking worker after releasing
that mutex. Facts already committed to SQLite are included; pending collector
journal data is not promised to be included. Manual archives are not rotated.
CLI creation never overwrites an existing archive.

## Restore

The UI uploads and validates the archive before displaying its profiles/date.
Applying it requires explicit confirmation and the SHA256 of that inspected
file. Canonical Origin and X-Vision-Control are mandatory. Capture must be stopped.
No machine autostart tasks or privileged services are installed/enabled.

Before replacement, a validated before-restore-<UUID>.visionbackup is created in
local-data/restore-rollback (never automatically deleted). The candidate DB is
prepared separately: live leases cleared, checkpoints closed, active profile
cleared, domain freshness=saved/reason=restored_backup, preferences stored,
old source IDs blocked, restore timestamp barrier installed. Only then does a
native atomic SQLite backup transaction replace the existing writer database.
SQLite rolls that replacement back on copy failure, including BUSY. Old incoming
sources cannot resume even if absent from the restored snapshot. A fresh capture
and game login are required. Native ingestion checks both blocked UUIDs and the
restore timestamp barrier. Global revision advances beyond both previous DBs;
SSE readers can request resync instead of accepting an older revision.

Region mappings are reconstructed from unique UID/server identities. Ambiguous
UIDs need explicit server configuration. Failure to update the mapping returns a
warning with successful restoration; no false all-or-nothing claim is made for
that external derived configuration. The original collector token remains local.

The CLI can restore only into a new --db path. Replacement of an existing
installation uses the guarded API/UI flow. The database backup is installation-wide;
merging archives or importing only one profile is not supported.

## CLI and HTTP

vision-store --db local-data/vision-overlay.sqlite3 backup PATH.visionbackup
vision-store inspect-backup PATH.visionbackup
vision-store --db NEW.sqlite3 restore-backup PATH.visionbackup

POST /api/v1/backups/export JSON {settings,catalog} -> archive bytes.
POST /api/v1/backups/inspect archive bytes -> {manifest,settings,sha256}.
POST /api/v1/backups/restore archive bytes; X-Vision-Restore: replace-all and
X-Backup-Sha256 are required -> restoration result and rollback_path.

The browser uses showSaveFilePicker when available (Chrome/Edge on localhost);
otherwise it downloads the file through browser settings. Restore file selection
uses a standard file input. A browser refusing save-file access should use the
CLI output path; the fallback download path depends on browser settings.

## Схема5 (15.11.2026)

Переносимые копии включают wish_records и wish_imports: историю молитв и
метаданные импортов по профилям. Текущий exporter указывает database_schema_version=5;
импорт поддерживает схемы2–5. Для старых копий таблицы молитв создаются миграцией.
Подтверждение непрерывности live-истории из старой копии не выводится.

## Схема6 (15.11.2026)

wish_sync_state сохраняет последнее состояние/время загрузки по профилю
и исходной игровой сессии. Экспорт указывает схему6, импорт поддерживает2–6.
Состояние synced не подтверждает актуальность после restore: live-контекст
и leases очищаются, а активная сессия повторно проверяется при чтении статуса.

## Схема7 (15.11.2026)

planner_goals хранит пользовательские цели по profile_id, GUID сущности и
версии записи. Полностью входит в копию; снимки игры цели не заменяют.
Exporter использует схему7, importer принимает2–7. При восстановлении старой
копии таблица целей создаётся пустой миграцией.
