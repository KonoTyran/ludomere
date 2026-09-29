#!/usr/bin/env python3
"""Stage verified private UMU and Comet helpers; never install host packages."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import tarfile
import tempfile
import urllib.parse
import urllib.request

SOURCES = {
    "umu-1.4.4.tar.gz": (
        "https://codeload.github.com/Open-Wine-Components/umu-launcher/tar.gz/refs/tags/1.4.4",
        "19767e226b0b937b1dcdcd00282eb31d1a7bb42ee01d54f6dae78e44ab8e3284",
    ),
    "comet-v0.3.2.tar.gz": (
        "https://codeload.github.com/imLinguin/comet/tar.gz/refs/tags/v0.3.2",
        "5d619f3d801b5aceb6fe35489f94b8c59668b3b381048a65d7d30be21bc70838",
    ),
    "comet": (
        "https://github.com/imLinguin/comet/releases/download/v0.3.2/comet-x86_64-unknown-linux-gnu",
        "2d6694d544fd3155d90d540e70bc1be767a6b9fdda130275f2b79616ff14e843",
    ),
    "GalaxyCommunication.exe": (
        "https://github.com/imLinguin/comet/releases/download/v0.3.2/GalaxyCommunication-dummy.exe",
        "c7695267da363a861af99db95cafe68b732ae743e5830b4feea1bc7ee745f99d",
    ),
}


class PublisherRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, url):
        parsed = urllib.parse.urlparse(url)
        if parsed.scheme != "https" or parsed.hostname not in {
            "github.com", "codeload.github.com", "release-assets.githubusercontent.com",
            "objects.githubusercontent.com",
        }:
            raise RuntimeError("Helper redirected outside its publisher")
        return super().redirect_request(request, response, code, message, headers, url)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--destination", type=Path, default=Path("target/helpers"))
    parser.add_argument("--cache", type=Path, default=Path("target/helper-downloads"))
    args = parser.parse_args()
    args.cache.mkdir(parents=True, exist_ok=True)
    args.destination.mkdir(parents=True, exist_ok=True)
    if args.destination.is_symlink() or any(args.destination.iterdir()):
        raise RuntimeError("Helper destination must be an empty, real directory; choose a new staging directory")
    opener = urllib.request.build_opener(PublisherRedirect())
    for name, (url, digest) in SOURCES.items():
        path = args.cache / name
        if not path.is_file() or hashlib.file_digest(path.open("rb"), "sha256").hexdigest() != digest:
            with tempfile.NamedTemporaryFile(dir=args.cache, delete=False) as partial:
                temporary = Path(partial.name)
                try:
                    with opener.open(url, timeout=30) as response:
                        total = 0
                        while block := response.read(1024 * 1024):
                            total += len(block)
                            if total > 64 * 1024 * 1024:
                                raise RuntimeError("Helper download exceeded 64 MiB")
                            partial.write(block)
                    partial.flush()
                    with temporary.open("rb") as downloaded:
                        if hashlib.file_digest(downloaded, "sha256").hexdigest() != digest:
                            raise RuntimeError(f"Integrity verification failed for {name}")
                    temporary.replace(path)
                finally:
                    temporary.unlink(missing_ok=True)
        print(f"Verified {name}")
    with tempfile.TemporaryDirectory(dir=args.destination) as staging:
        for name, prefix in (("umu-1.4.4.tar.gz", "umu-launcher-1.4.4"), ("comet-v0.3.2.tar.gz", "comet-0.3.2")):
            with tarfile.open(args.cache / name) as archive:
                members = [member for member in archive.getmembers() if
                    member.name in {f"{prefix}/LICENSE", f"{prefix}/README.md"}
                    or (name.startswith("umu-") and member.name.startswith(f"{prefix}/umu/"))]
                archive.extractall(staging, members=members, filter="data")
        umu = Path(staging) / "umu-launcher-1.4.4"
        comet = Path(staging) / "comet-0.3.2"
        shutil.copytree(umu / "umu", args.destination / "umu/vendor/umu")
        shutil.copyfile(Path(__file__).resolve().parents[1] / "resources/helpers/umu-run", args.destination / "umu/umu-run")
        (args.destination / "umu/umu-run").chmod(0o755)
        (args.destination / "licenses").mkdir()
        shutil.copyfile(umu / "LICENSE", args.destination / "licenses/UMU-LICENSE")
        shutil.copyfile(umu / "README.md", args.destination / "licenses/UMU-README.md")
        shutil.copyfile(comet / "LICENSE", args.destination / "licenses/Comet-LICENSE")
    (args.destination / "comet").mkdir()
    shutil.copyfile(args.cache / "comet", args.destination / "comet/comet")
    (args.destination / "comet/comet").chmod(0o755)
    shutil.copyfile(args.cache / "GalaxyCommunication.exe", args.destination / "comet/GalaxyCommunication.exe")
    (args.destination / "comet/GalaxyCommunication.exe").chmod(0o644)
    metadata = {"version": "0.3.2", "files": {}}
    for name in ("comet", "GalaxyCommunication.exe"):
        with (args.destination / "comet" / name).open("rb") as binary:
            metadata["files"][name] = hashlib.file_digest(binary, "sha256").hexdigest()
    (args.destination / "comet/build.json").write_text(json.dumps(metadata, indent=2) + "\n")
    (args.destination / "sources").mkdir()
    for name in ("umu-1.4.4.tar.gz", "comet-v0.3.2.tar.gz"):
        shutil.copyfile(args.cache / name, args.destination / "sources" / name)
    (args.destination / "sources/manifest.json").write_text(json.dumps(SOURCES, indent=2) + "\n")
    print(f"Helpers staged at {args.destination.resolve()}")


if __name__ == "__main__":
    main()
