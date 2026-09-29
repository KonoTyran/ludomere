#!/usr/bin/env python3
"""Build a pacman package from current product files, including uncommitted edits."""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--source-only", action="store_true")
args = parser.parse_args()
version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
name = f"ludomere-{version}"
output = root / "dist"
output.mkdir(exist_ok=True)
files = [root / item for item in (
    "Cargo.toml", "Cargo.lock", "LICENSE", "README.md", "THIRD_PARTY_NOTICES.md", "PKGBUILD",
)]
# Deliberately restrict untracked inclusion to product files. Never archive .git,
# account data, .project-manager, build caches, or arbitrary untracked root files.
for directory in ("src", "resources", "tests", "tools", ".github"):
    for path in (root / directory).rglob("*"):
        if path.is_symlink():
            raise RuntimeError(f"Source snapshot refuses symlink: {path.relative_to(root)}")
        if path.is_file() and "__pycache__" not in path.parts and (
            path.suffix in {".rs", ".toml", ".json", ".py", ".sh", ".yml", ".yaml", ".svg", ".png", ".txt", ".desktop", ".xml", ".md"}
            or path.name in {"umu-run", "Containerfile"}
        ):
            files.append(path)
archive = output / f"{name}.tar.gz"
with tarfile.open(archive, "w:gz") as tar:
    for path in sorted(files):
        tar.add(path, arcname=str(Path(name) / path.relative_to(root)), recursive=False)
with archive.open("rb") as source:
    digest = hashlib.file_digest(source, "sha256").hexdigest()
(output / f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n")
print(f"Created {archive}", flush=True)
if not args.source_only:
    (root / "target").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="pacman-", dir=root / "target") as work:
        work = Path(work)
        shutil.copyfile(archive, work / archive.name)
        (work / "PKGBUILD").write_text((root / "PKGBUILD").read_text().replace("@SOURCE_SHA256@", digest))
        environment = os.environ.copy()
        environment["PKGDEST"] = str(output)
        environment.setdefault("CARGO_TARGET_DIR", str(root / "target/package-build"))
        environment["LUDOMERE_HELPER_CACHE"] = str(root / "target/helper-downloads")
        subprocess.run(["makepkg", "--force", "--cleanbuild"], cwd=work, env=environment, check=True)
