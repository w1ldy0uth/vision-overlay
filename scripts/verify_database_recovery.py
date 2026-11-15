"""Verify online SQLite backup and isolated recovery without replacing the live DB.

This diagnostic is not the portable user backup format or the in-app restore flow.
Only the database is backed up; collector credentials, raw packets and settings
are not copied. The output directory must not exist.
"""
import argparse
from contextlib import closing
import hashlib
import json
import sqlite3
from pathlib import Path


def readonly(path):
    return sqlite3.connect(path.resolve().as_uri() + "?mode=ro", uri=True, timeout=10)


def check(connection):
    result = connection.execute("PRAGMA integrity_check").fetchall()
    if result != [("ok",)]:
        raise ValueError(f"SQLite integrity check failed: {result[:5]}")
    foreign_keys = connection.execute("PRAGMA foreign_key_check").fetchall()
    if foreign_keys:
        raise ValueError(f"Broken foreign keys: {foreign_keys[:5]}")
    versions = [r[0] for r in connection.execute("SELECT version FROM _sqlx_migrations WHERE success=1 ORDER BY version")]
    if versions not in ([1, 2], [1, 2, 3], [1, 2, 3, 4]):
        raise ValueError(f"Unsupported database schema: {versions}")


def logical_digest(connection):
    """Hash schema and every row, preserving exact SQL values and ignoring page layout."""
    schema = connection.execute("SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE sql IS NOT NULL ORDER BY type,name").fetchall()
    tables = {}
    for (name,) in connection.execute("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"):
        quoted = '"' + name.replace('"', '""') + '"'
        # Sort row hashes so equality does not depend on SQL scan/index order.
        hashes = []
        for row in connection.execute(f"SELECT * FROM {quoted}"):
            values = [{"blob": value.hex()} if isinstance(value, bytes) else value for value in row]
            hashes.append(hashlib.sha256(json.dumps(values, ensure_ascii=False, separators=(",", ":")).encode()).digest())
        tables[name] = {"rows": len(hashes), "sha256": hashlib.sha256(b"".join(sorted(hashes))).hexdigest()}
    return {"schema_sha256": hashlib.sha256(json.dumps(schema, ensure_ascii=False).encode()).hexdigest(), "tables": tables}


def verify(source, output):
    source = source.resolve(strict=True)
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    backup_path = output / "backup.sqlite3"
    restored_path = output / "restored.sqlite3"
    with closing(readonly(source)) as live:
        live.execute("BEGIN")
        # Establish a WAL read snapshot before backup; concurrent capture writes continue.
        revision = live.execute("SELECT revision FROM store_meta").fetchone()[0]
        reference = logical_digest(live)
        with closing(sqlite3.connect(backup_path)) as backup:
            live.backup(backup, pages=256, sleep=0.05)
            check(backup)
            if logical_digest(backup) != reference:
                raise ValueError("Backup differs from the pinned source snapshot")
        live.rollback()
    backup_sha = hashlib.sha256(backup_path.read_bytes()).hexdigest()
    # Simulate a lost installation in an explicitly isolated destination.
    with closing(sqlite3.connect(restored_path)) as restored:
        restored.execute("CREATE TABLE recovery_probe(marker TEXT)")
        restored.execute("INSERT INTO recovery_probe VALUES('empty installation test')")
        restored.commit()
        with closing(readonly(backup_path)) as backup:
            backup.backup(restored, pages=256, sleep=0.05)
        check(restored)
        if logical_digest(restored) != reference:
            raise ValueError("Restored database differs from the backup")
        profiles = [dict(zip(("profile_id", "uid", "server_region"), row)) for row in restored.execute("SELECT profile_id,uid,server_region FROM profiles ORDER BY uid,server_region")]
    # Closed WAL connections must leave each artifact usable as a standalone DB.
    with closing(readonly(backup_path)) as backup, closing(readonly(restored_path)) as restored:
        check(backup)
        check(restored)
        if logical_digest(backup) != reference or logical_digest(restored) != reference:
            raise ValueError("Closed/reopened database differs")
    report = {
        "result": "passed", "source": str(source), "snapshot_revision": str(revision),
        "backup": str(backup_path), "restored": str(restored_path),
        "backup_sha256": backup_sha, "logical_digest": reference, "profiles": profiles,
        "source_was_replaced": False, "api_validation": "pending",
        "scope": "isolated database recovery diagnostic, not a portable backup package",
    }
    (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=Path("local-data/vision-overlay.sqlite3"))
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    report = verify(args.source, args.output_dir)
    print(json.dumps({k: report[k] for k in ("result", "snapshot_revision", "backup", "restored", "profiles")}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
