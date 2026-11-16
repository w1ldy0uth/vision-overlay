import hashlib
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

from package_portable import package
from validate_release_asset import normalized_name, validate


class ReleaseAssetTests(unittest.TestCase):
    def fixture(self, root, separator="/", commit="a" * 40, duplicate=False, extra=False):
        archive = root / "VisionOverlay-test.zip"
        data = b"readme"
        manifest = {"version": "0.1.0-beta.1", "build_commit": commit, "files": [
            {"path": "README.txt", "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()}]}
        rows = [("bundle/package-manifest.json", json.dumps(manifest).encode()), ("bundle/README.txt", data)]
        if duplicate:
            rows.append(("bundle//package-manifest.json", json.dumps(manifest).encode()))
        if extra:
            rows.append(("bundle/unknown.txt", b"extra"))
        with zipfile.ZipFile(archive, "w") as z:
            for name, raw in rows:
                info = zipfile.ZipInfo("placeholder")
                # Construct the actual legacy wire form, even on Windows where
                # ZipInfo(name) would normalize it before writing the fixture.
                info.filename = info.orig_filename = name.replace("/", separator)
                z.writestr(info, raw)
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        Path(str(archive) + ".sha256").write_text(checksum + "  " + archive.name)
        return archive

    def test_windows_backslash_wire_names_and_posix_names_validate_identically(self):
        for separator in ("/", "\\", "\\\\"):
            with self.subTest(separator=separator), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                archive = self.fixture(root, separator)
                with zipfile.ZipFile(archive) as z:
                    self.assertIn(separator, z.infolist()[0].orig_filename)
                self.assertEqual(validate(root, "v0.1.0-beta.1", "a" * 40)["files"], 1)

    def test_mismatched_commit_version_checksum_and_unlisted_files_fail(self):
        for case in ("commit", "version", "checksum", "duplicate", "extra"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                archive = self.fixture(root, duplicate=case == "duplicate", extra=case == "extra")
                if case == "checksum":
                    with archive.open("ab") as f:
                        f.write(b"modified")
                with self.assertRaises(ValueError):
                    validate(root, "v9" if case == "version" else "v0.1.0-beta.1",
                             "b" * 40 if case == "commit" else "a" * 40)

    def test_traversal_and_absolute_paths_are_rejected(self):
        for name in ("../secret", "bundle/../../secret", "C:\\secret", "/absolute", "x\0hidden"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                normalized_name(name)

    def test_writer_emits_forward_slashes_in_raw_zip_directory(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            source = root / "bundle"
            (source / "nested").mkdir(parents=True)
            (source / "nested/README.txt").write_text("test")
            archive = root / "output.zip"
            package(source, archive)
            with zipfile.ZipFile(archive) as z:
                self.assertEqual(z.infolist()[0].orig_filename, "bundle/nested/README.txt")
            with self.assertRaises(ValueError):
                package(source, archive)


if __name__ == "__main__":
    unittest.main()
