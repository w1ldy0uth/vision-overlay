"""Verify a release ZIP without relying on host-specific ZIP path normalization."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import zipfile


def normalized_name(name):
    name = name.replace("\\", "/")
    path = PurePosixPath(name)
    if not name or not path.parts or "\0" in name or path.is_absolute() or any(
            part in ("..",) or ":" in part for part in path.parts):
        raise ValueError("Unsafe ZIP entry name")
    return path.as_posix()


def validate(directory, tag, commit):
    archives = sorted(directory.glob("*.zip"))
    if len(archives) != 1:
        raise ValueError(f"Expected exactly one portable ZIP, found {len(archives)}")
    archive = archives[0]
    checksum_path = Path(str(archive) + ".sha256")
    fields = checksum_path.read_text(encoding="utf-8-sig").split()
    if len(fields) != 2 or not re.fullmatch(r"[a-fA-F0-9]{64}", fields[0]) or fields[1].lstrip("*") != archive.name:
        raise ValueError("Invalid ZIP checksum sidecar")
    if hashlib.sha256(archive.read_bytes()).hexdigest() != fields[0].lower():
        raise ValueError("ZIP SHA256 mismatch")
    with zipfile.ZipFile(archive) as z:
        entries = {}
        folded = set()
        for info in z.infolist():
            # orig_filename is the central-directory name. filename may already
            # have backslashes normalized on Windows, hiding Linux-only failures.
            name = normalized_name(info.orig_filename)
            if info.is_dir() or info.orig_filename.replace("\\", "/").endswith("/"):
                continue
            if name.casefold() in folded:
                raise ValueError("Duplicate normalized ZIP entry")
            folded.add(name.casefold())
            entries[name] = info
        manifests = [name for name in entries if PurePosixPath(name).name == "package-manifest.json"]
        if len(manifests) != 1:
            raise ValueError(f"Expected exactly one package-manifest.json, found {len(manifests)}")
        manifest_name = manifests[0]
        manifest = json.loads(z.read(entries[manifest_name]))
        if manifest.get("build_commit") != commit:
            raise ValueError("Artifact build_commit does not match the resolved tag commit")
        if "v" + manifest.get("version", "") != tag:
            raise ValueError("Artifact version does not match the release tag")
        parent = PurePosixPath(manifest_name).parent
        expected = {manifest_name}
        for row in manifest["files"]:
            relative = normalized_name(row["path"])
            name = (parent / relative).as_posix()
            if name in expected:
                raise ValueError("Duplicate package manifest path")
            expected.add(name)
            if name not in entries:
                raise ValueError(f"Manifest file missing: {relative}")
            raw = z.read(entries[name])
            if len(raw) != row["bytes"] or hashlib.sha256(raw).hexdigest() != row["sha256"]:
                raise ValueError(f"Manifest file checksum mismatch: {relative}")
        if set(entries) != expected:
            raise ValueError("ZIP contains files absent from its package manifest")
    return {"archive": archive.name, "version": manifest["version"], "build_commit": commit,
            "files": len(manifest["files"])}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    print(json.dumps(validate(args.directory, args.tag, args.commit)))
