"""Check the version contract and pinned resource/notice hashes."""
import hashlib
import json
from pathlib import Path
import sys
import tomllib

root = Path(__file__).resolve().parent.parent
workspace = tomllib.loads((root / "Cargo.toml").read_text())
version = workspace["workspace"]["package"]["version"]
assert json.loads((root / "web/package.json").read_text())["version"] == version
lock = json.loads((root / "web/package-lock.json").read_text())
assert lock["version"] == version and lock["packages"][""]["version"] == version
for crate in (root / "crates").glob("*/Cargo.toml"):
    assert tomllib.loads(crate.read_text())["package"]["version"]["workspace"] is True
for source in json.loads((root / "packaging/license-overrides/sources.json").read_text()):
    path = root / "packaging/license-overrides" / source["package"] / source["path"]
    assert hashlib.sha256(path.read_bytes()).hexdigest() == source["sha256"], str(path)
if len(sys.argv) > 1:
    assert sys.argv[1] == "v" + version, "Tag must equal workspace release version"
print("Release metadata verified:", version)
