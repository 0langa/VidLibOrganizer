#!/usr/bin/env python3
"""Verify offline or explicitly hydrate pinned build resources; never used at runtime."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import sys
import urllib.request
import zipfile


def verify(data, digest, label):
    if hashlib.sha256(data).hexdigest() != digest:
        raise ValueError(f"SHA-256 mismatch: {label}")
    return data


def fetch_bytes(url):
    if not url.startswith("https://"):
        raise ValueError("HTTPS required")
    # urllib preserves inherited proxy and CA configuration. No direct fallback.
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read(32 * 1024 * 1024 + 1)
    if len(data) > 32 * 1024 * 1024:
        raise ValueError("download exceeds 32 MiB")
    return data


def verify_pe(data):
    if len(data) < 64 or data[:2] != b"MZ":
        raise ValueError("invalid PE executable")
    offset = int.from_bytes(data[60:64], "little")
    if data[offset:offset + 6] != b"PE\0\0\x64\x86":
        raise ValueError("expected x86_64 PE executable")


def run(root, fetch=False, archive=None):
    manifest = json.loads((root / "tools/media-info-manifest.json").read_text())
    if manifest["schema_version"] != 1:
        raise ValueError("unsupported manifest")
    for artifact in manifest["artifacts"]:
        directory = root / artifact["destination"]
        if fetch or archive:
            data = Path(archive).read_bytes() if archive else fetch_bytes(artifact["url"])
            verify(data, artifact["sha256"], "official archive")
            prepared = {}
            with zipfile.ZipFile(io.BytesIO(data)) as source:
                for name, item in artifact["files"].items():
                    prepared[name] = verify(source.read(item["archive_member"]), item["sha256"], name)
            for item in artifact["notices"]:
                path = directory / item["file"]
                if path.exists():
                    data = path.read_bytes()
                else:
                    data = fetch_bytes(item["url"])
                    if item["extraction"] == "first_c_comment":
                        data = data[data.index(b"/*"):data.index(b"*/") + 2] + b"\n"
                prepared[item["file"]] = verify(data, item["sha256"], item["file"])
            verify_pe(prepared["MediaInfo.exe"])
            # Validate the whole resource set before any replacement.
            directory.mkdir(parents=True, exist_ok=True)
            for name, data in prepared.items():
                (directory / name).write_bytes(data)
        expected = {name: item["sha256"] for name, item in artifact["files"].items()}
        expected.update({item["file"]: item["sha256"] for item in artifact["notices"]})
        for name, digest in expected.items():
            data = verify((directory / name).read_bytes(), digest, name)
            if name == "MediaInfo.exe":
                verify_pe(data)
        mappings = json.loads((root / "src-tauri/tauri.conf.json").read_text())["bundle"]["resources"]
        if mappings.get("resources/mediainfo/") != "mediainfo/" or mappings.get("../tools/media-info-manifest.json") != "mediainfo/manifest.json":
            raise ValueError("Tauri resource mapping omits metadata or manifest")
        print(f"verified MediaInfo {manifest['version']} {artifact['architecture']}: {len(expected)} resources")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--check", action="store_true", help="offline verification (default)")
    parser.add_argument("--fetch", action="store_true", help="explicit build-time download")
    parser.add_argument("--archive", type=Path)
    args = parser.parse_args()
    try:
        run(args.root, args.fetch, args.archive)
    except (ValueError, OSError, KeyError, zipfile.BadZipFile) as error:
        print(f"MediaInfo vendoring failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
