"""Fail before build/publish on mismatched version, tag or release notes."""
import json
import os
from pathlib import Path
import re
import tomllib

root = Path(__file__).resolve().parent.parent
version = json.loads((root / "plugin.json").read_text())["version"]
assert re.fullmatch(r"\d+\.\d+\.\d+", version), "Only stable release versions are supported"
assert tomllib.loads((root / "backend/Cargo.toml").read_text())["package"]["version"] == version
assert json.loads((root / "frontend/package.json").read_text())["version"] == version
assert (root / "docs/release-notes.md").read_text(encoding="utf-8").splitlines()[0] == f"# v{version}"
if os.environ.get("GITHUB_REF_TYPE") == "tag":
    assert os.environ["GITHUB_REF_NAME"] == f"v{version}", "Tag does not match the package version"
print(f"Validated v{version}")
