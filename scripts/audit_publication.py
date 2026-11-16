"""Check publication inputs without printing any matching secret values."""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import subprocess


def audit(staged=False, denied=()):
    command = ["git", "ls-files", "-z", "--cached"]
    if not staged:
        command += ["--others", "--exclude-standard"]
    files = sorted(set(subprocess.check_output(command).decode().strip("\0").split("\0")))
    public = json.loads(Path("packaging/public-crypto-material.json").read_text())
    patterns = {
        "private key": r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----",
        "GitHub token": r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,})",
        "credential URL": r"https?://[^\s/]+:[^\s/@]+@",
        "wish authorization URL": r"authkey=[A-Za-z0-9%+/=_-]{20,}",
    }
    errors = []
    for name in files:
        path = Path(name)
        if not path.is_file():
            continue
        if name.startswith(("local-data/", "target/", "dist/", "docs/", "beta_log/", "beta_logs/", "web/node_modules/")) or "__pycache__" in path.parts or path.suffix in (".pyc", ".pdb", ".sqlite3", ".etl", ".pcapng", ".voraw", ".visionbackup"):
            errors.append(f"Generated/private artifact: {name}")
        raw = subprocess.check_output(["git", "show", ":" + name]) if staged else path.read_bytes()
        approved = name in public
        if approved and hashlib.sha256(raw).hexdigest() != public[name]["sha256"]:
            errors.append(f"Public cryptographic material changed: {name}")
        if path.suffix == ".pem" and not approved:
            errors.append(f"Unreviewed PEM file: {name}")
        if name == "packaging/catalog/bootstrap.visioncatalog.gz":
            source = json.loads(Path("packaging/catalog/source.json").read_text())
            if hashlib.sha256(raw).hexdigest() != source["compressed_sha256"]:
                errors.append("Bootstrap archive hash changed")
            raw = gzip.decompress(raw)
            if hashlib.sha256(raw).hexdigest() != source["bundle_sha256"]:
                errors.append("Bootstrap payload hash changed")
            bundle = json.loads(raw)
            if set(bundle) != {"manifest", "payload"}:
                errors.append("Unexpected bootstrap envelope")
        try:
            text = raw.decode("utf-8-sig")
        except UnicodeDecodeError:
            continue
        for label, pattern in patterns.items():
            if approved and label == "private key":
                continue
            for match in re.finditer(pattern, text):
                errors.append(f"{label}: {name}:{text.count(chr(10), 0, match.start()) + 1}")
        for value in denied:
            if value in text:
                errors.append(f"Local identifier present: {name}")
    if errors:
        raise SystemExit("\n".join(errors))
    print(f"Publication input check passed: {len(files)} files; reviewed public crypto exceptions use exact hashes")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--staged", action="store_true")
    parser.add_argument("--deny-value", action="append", default=[])
    args = parser.parse_args()
    audit(args.staged, args.deny_value)
