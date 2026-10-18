import importlib.util
import json
import hashlib
import zipfile
from unittest.mock import patch
from pathlib import Path
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("vendor", ROOT / "tools/vendor_media_info.py")
VENDOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VENDOR)

class PackageIntegrity(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "tools").mkdir()
        shutil.copy(ROOT / "tools/media-info-manifest.json", self.root / "tools")
        shutil.copytree(ROOT / "src-tauri/resources", self.root / "src-tauri/resources")
        shutil.copy(ROOT / "src-tauri/tauri.conf.json", self.root / "src-tauri")

    def test_offline_package_and_notice_integrity(self):
        VENDOR.run(self.root)
        notice = self.root / "src-tauri/resources/mediainfo/windows-x86_64/LICENSE"
        notice.write_text("missing required redistribution terms")
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            VENDOR.run(self.root)

    def test_wrong_executable_and_installer_resource_mapping_fail(self):
        exe = self.root / "src-tauri/resources/mediainfo/windows-x86_64/MediaInfo.exe"
        exe.write_bytes(b"not the pinned executable")
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            VENDOR.run(self.root)
        shutil.copy(ROOT / "src-tauri/resources/mediainfo/windows-x86_64/MediaInfo.exe", exe)
        path = self.root / "src-tauri/tauri.conf.json"
        config = json.loads(path.read_text())
        config["bundle"]["resources"] = {}
        path.write_text(json.dumps(config))
        with self.assertRaisesRegex(ValueError, "resource"):
            VENDOR.run(self.root)

    def test_local_archive_without_fetch_never_downloads_missing_notice(self):
        manifest_path = self.root / "tools/media-info-manifest.json"
        manifest = json.loads(manifest_path.read_text())
        artifact = manifest["artifacts"][0]
        directory = self.root / artifact["destination"]
        archive = self.root / "local.zip"
        with zipfile.ZipFile(archive, "w") as output:
            for name, item in artifact["files"].items():
                output.writestr(item["archive_member"], (directory / name).read_bytes())
        artifact["sha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
        manifest_path.write_text(json.dumps(manifest))
        (directory / artifact["notices"][0]["file"]).unlink()
        with patch.object(VENDOR, "fetch_bytes", side_effect=AssertionError("unexpected network request")) as download:
            with self.assertRaisesRegex(ValueError, "--fetch"):
                VENDOR.run(self.root, archive=archive)
            download.assert_not_called()

    def test_untrusted_archive_never_replaces_packaged_executable(self):
        archive = self.root / "wrong.zip"
        archive.write_bytes(b"untrusted archive")
        exe = self.root / "src-tauri/resources/mediainfo/windows-x86_64/MediaInfo.exe"
        original = exe.read_bytes()
        with self.assertRaisesRegex(ValueError, "SHA-256"):
            VENDOR.run(self.root, archive=archive)
        self.assertEqual(exe.read_bytes(), original)

if __name__ == "__main__":
    unittest.main()
