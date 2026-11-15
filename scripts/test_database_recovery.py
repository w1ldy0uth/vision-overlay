import sqlite3
import tempfile
import unittest
from contextlib import closing
from pathlib import Path

from verify_database_recovery import check, verify


class DatabaseRecoveryTests(unittest.TestCase):
    def database(self, path):
        writer = sqlite3.connect(path)
        writer.execute("PRAGMA journal_mode=WAL")
        writer.execute("PRAGMA wal_autocheckpoint=0")
        writer.executescript("""
            CREATE TABLE _sqlx_migrations(version INTEGER,success INTEGER);
            INSERT INTO _sqlx_migrations VALUES(1,1),(2,1);
            CREATE TABLE store_meta(revision INTEGER);
            INSERT INTO store_meta VALUES(123);
            CREATE TABLE profiles(profile_id TEXT PRIMARY KEY,uid TEXT,server_region TEXT);
            INSERT INTO profiles VALUES('p','700000001','os_euro');
            CREATE TABLE facts(profile_id TEXT REFERENCES profiles(profile_id),value BLOB);
        """)
        writer.execute("INSERT INTO facts VALUES('p',?)", (b"committed-in-wal",))
        writer.commit()
        return writer

    def test_online_wal_backup_and_recovery_without_source_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with closing(self.database(root / "source.db")) as writer:
                self.assertGreater((root / "source.db-wal").stat().st_size, 0)
                report = verify(root / "source.db", root / "check")
                self.assertEqual(report["snapshot_revision"], "123")
                with closing(sqlite3.connect(report["restored"])) as restored:
                    self.assertEqual(restored.execute("SELECT value FROM facts").fetchone()[0], b"committed-in-wal")
                    self.assertIsNone(restored.execute("SELECT name FROM sqlite_schema WHERE name='recovery_probe'").fetchone())
                writer.execute("UPDATE store_meta SET revision=124")
                writer.commit()
                with closing(sqlite3.connect(report["backup"])) as backup:
                    self.assertEqual(backup.execute("SELECT revision FROM store_meta").fetchone()[0], 123)

    def test_existing_output_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with closing(self.database(root / "source.db")):
                output = root / "existing"
                output.mkdir()
                marker = output / "backup.sqlite3"
                marker.write_bytes(b"user-owned file")
                with self.assertRaises(FileExistsError):
                    verify(root / "source.db", output)
                self.assertEqual(marker.read_bytes(), b"user-owned file")

    def test_foreign_keys_and_unknown_schema_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            with closing(self.database(Path(directory) / "source.db")) as connection:
                connection.execute("INSERT INTO facts VALUES('missing',NULL)")
                connection.commit()
                with self.assertRaisesRegex(ValueError, "foreign keys"):
                    check(connection)
                connection.execute("DELETE FROM facts WHERE profile_id='missing'")
                connection.execute("INSERT INTO _sqlx_migrations VALUES(999,1)")
                connection.commit()
                with self.assertRaisesRegex(ValueError, "Unsupported"):
                    check(connection)


if __name__ == "__main__":
    unittest.main()
