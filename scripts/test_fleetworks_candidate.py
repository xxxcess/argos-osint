"""Offline contracts for the immutable native CLI candidate archive."""

import io
import os
import tarfile
import tempfile
import unittest
from pathlib import Path

from fleetworks_candidate import BINARIES, checksum_line, extract, package, sha256, tar_member


class CandidateArchiveTests(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name)
        self.version = "0.2.1"
        self.target = "x86_64-unknown-linux-gnu"
        self.revision = "1" * 40
        self.binaries = self.root / "binaries"
        self.binaries.mkdir()
        for name in BINARIES:
            path = self.binaries / name
            path.write_bytes(f"fixture payload {name}\n".encode())
            path.chmod(0o755)
        self.license = self.root / "LICENSE"
        self.license.write_bytes(b"fixture license\n")

    def pack(self, name="archive"):
        return package(self.root / name, self.binaries, self.license, self.version, self.target, self.revision)

    def extract(self, archive, digest, revision=None):
        destination = self.root / "install"
        extract(archive, destination, digest, self.version, self.target, revision or self.revision)
        return destination

    def test_reproducible_archive_preserves_every_binary_and_its_mode(self):
        archive, digest = self.pack()
        duplicate, duplicate_digest = self.pack("duplicate")
        self.assertEqual(archive.read_bytes(), duplicate.read_bytes())
        self.assertEqual(digest, duplicate_digest)
        self.assertEqual(archive.with_suffix(".gz.sha256").read_text(), checksum_line(archive, digest))
        install = self.extract(archive, digest)
        self.assertEqual((install / "LICENSE").read_bytes(), self.license.read_bytes())
        for name in BINARIES:
            path = install / "bin" / name
            self.assertEqual(path.read_bytes(), (self.binaries / name).read_bytes())
            self.assertTrue(os.access(path, os.X_OK))

    def test_rejects_digest_sidecar_and_manifest_mismatches(self):
        archive, digest = self.pack()
        with self.assertRaisesRegex(ValueError, "packaging job's SHA-256"):
            self.extract(archive, "0" * 64)
        with self.assertRaisesRegex(ValueError, "manifest does not match"):
            self.extract(archive, digest, "2" * 40)
        archive.with_suffix(".gz.sha256").write_text("wrong\n")
        with self.assertRaisesRegex(ValueError, "sidecar"):
            self.extract(archive, digest)
        self.assertFalse((self.root / "install").exists())

    def test_rejects_unexpected_paths_before_extraction(self):
        archive, _ = self.pack()
        with tarfile.open(archive, "w:gz") as tar:
            tar.addfile(tar_member("../escape", 1, 0o644), io.BytesIO(b"x"))
        digest = sha256(archive)
        archive.with_suffix(".gz.sha256").write_text(checksum_line(archive, digest))
        with self.assertRaisesRegex(ValueError, "unexpected"):
            self.extract(archive, digest)
        self.assertFalse((self.root / "escape").exists())
        self.assertFalse((self.root / "install").exists())


if __name__ == "__main__":
    unittest.main()
