"""Exercise offline pack admission without downloading model assets."""

import argparse
import hashlib
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import fleetworks_qa_pack as pack


TARGET = "x86_64-unknown-linux-gnu"
ELF = b"\x7fELF\x02\x01" + bytes(12) + (62).to_bytes(2, "little")


class PackTest(unittest.TestCase):
    def test_pack_progress_is_private_and_clears_after_interrupted_download(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            marker = state / "qa-pack-progress.json"
            with self.assertRaisesRegex(RuntimeError, "interrupted"):
                with pack.pack_progress(state, "downloading_pack"):
                    self.assertEqual(marker.stat().st_mode & 0o777, 0o600)
                    self.assertEqual(json.loads(marker.read_text()), {"stage": "downloading_pack"})
                    raise RuntimeError("interrupted")
            self.assertFalse(marker.exists())

    def test_import_checks_frozen_inventory_and_registers_private_pointer(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model = root / "source-model"
            model.mkdir()
            refs = {}
            for relative in pack.MODEL:
                name = Path(relative).name
                data = name.encode()
                (model / name).write_bytes(data)
                refs[relative] = hashlib.sha256(data).hexdigest()
            worker = root / "worker"
            worker.write_bytes(ELF)
            libs = []
            for name in ("libtorch_cpu.so", "libc10.so"):
                lib = root / name
                lib.write_bytes(ELF)
                libs.append(lib)
            archive = root / "worker.tar.gz"
            state = root / "state"
            state.mkdir(mode=0o700)
            with patch.object(pack, "MODEL", refs):
                receipt = pack.build(argparse.Namespace(
                    model=model, worker=worker, listener=None, lib=libs, license=[],
                    output=archive, target=TARGET,
                ))
                installed = root / "installed"
                options = argparse.Namespace(
                    archive=archive, archive_sha256=receipt["archive_sha256"],
                    manifest_sha256="0" * 64, output=installed, target=TARGET,
                    state_dir=state,
                )
                with self.assertRaisesRegex(ValueError, "manifest SHA-256"):
                    pack.install(options)
                self.assertFalse(installed.exists())
                pointer = state / "qa-pack.json"
                self.assertFalse(pointer.exists())
                self.assertFalse((state / "qa-pack-progress.json").exists())
                options.manifest_sha256 = receipt["manifest_sha256"]
                pack.install(options)
                self.assertFalse((state / "qa-pack-progress.json").exists())
                self.assertEqual(pointer.stat().st_mode & 0o777, 0o600)
                self.assertEqual(json.loads(pointer.read_text()), {
                    "root": str(installed.resolve()), "manifest_sha256": options.manifest_sha256,
                })
                (installed / "lib/libc10.so").write_bytes(ELF + b"replacement")
                with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
                    pack.verify_directory(installed, TARGET, options.manifest_sha256)

    def test_archive_rejects_escape_link_and_duplicate(self):
        for name, kind in (("../escape", tarfile.REGTYPE), (pack.WORKER, tarfile.SYMTYPE),
                           (pack.WORKER, tarfile.REGTYPE)):
            with self.subTest(name=name, kind=kind), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                archive = root / "archive.tar.gz"
                with tarfile.open(archive, "w:gz") as target:
                    for _ in range(2):
                        info = tarfile.TarInfo(name)
                        info.type = kind
                        info.size = 1
                        target.addfile(info, io.BytesIO(b"x"))
                output = root / "output"
                output.mkdir()
                with self.assertRaisesRegex(ValueError, "archive entry"):
                    pack.extract(archive, output, TARGET)
                self.assertFalse((root / "escape").exists())


if __name__ == "__main__":
    unittest.main()
