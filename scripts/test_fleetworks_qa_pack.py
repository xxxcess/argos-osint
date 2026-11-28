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

import fleetworks_qa_ci as ci
import fleetworks_qa_pack as pack


TARGET = "x86_64-unknown-linux-gnu"
ELF = b"\x7fELF\x02\x01" + bytes(12) + (62).to_bytes(2, "little")


class PackTest(unittest.TestCase):
    def test_linux_worker_libraries_verify_installed_paths_and_copy_only_from_the_wheel(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            torch_root = root / "torch"
            library = torch_root / "lib/libtorch_cpu.so"
            library.parent.mkdir(parents=True)
            library.write_bytes(ELF)
            installed = root / "pack/lib"
            installed.mkdir(parents=True)
            worker = root / "worker"
            env = {}

            with patch.object(ci, "run", side_effect=[f"libtorch_cpu.so => {library} (0xf)", ""]):
                copied, = ci.linux_libraries(worker, installed, env, torch_root=torch_root)
            self.assertEqual(copied.read_bytes(), ELF)

            with patch.object(ci, "run", side_effect=[f"libtorch_cpu.so => {copied} (0xf)", ""]):
                self.assertEqual(ci.linux_libraries(worker, installed, env, installed=True), [copied])
            with patch.object(ci, "run", return_value=f"libtorch_cpu.so => {library} (0xf)"):
                with self.assertRaisesRegex(ValueError, "outside its expected root"):
                    ci.linux_libraries(worker, installed, env, installed=True)
            with patch.object(ci, "run", return_value=f"libtorch_cpu.so => {copied} (0xf)"):
                with self.assertRaisesRegex(ValueError, "outside its expected root"):
                    ci.linux_libraries(worker, installed, env, torch_root=torch_root)

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

    def test_generic_install_accepts_new_default_and_historical_flan_pack(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            worker = root / "worker"
            worker.write_bytes(ELF)
            libraries = [root / name for name in ("libtorch_cpu.so", "libc10.so")]
            for library in libraries:
                library.write_bytes(ELF)
            state = root / "state"
            state.mkdir(mode=0o700)
            pointer = state / "qa-pack.json"
            for model, requested in (("gpt2-small", None), ("flan-t5-small", "flan-t5-small")):
                with self.subTest(model=model):
                    model_dir = root / model
                    generator = model_dir / "generator"
                    generator.mkdir(parents=True)
                    pinned = {}
                    for relative in pack.GENERATOR_FILES[model]:
                        body = f"{model}/{relative}".encode()
                        (generator / Path(relative).name).write_bytes(body)
                        pinned[relative] = len(body), hashlib.sha256(body).hexdigest()
                    archive = root / f"{model}.tar.gz"
                    with patch.dict(pack.GENERATOR_FILES, {model: pinned}):
                        receipt = pack.build(argparse.Namespace(
                            model=model_dir, worker=worker, listener=None, lib=libraries, license=[],
                            output=archive, target=TARGET, chat_model=requested,
                        ))
                        self.assertEqual(receipt["chat_model"], model)
                        self.assertEqual(receipt["model_revision"], pack.CHAT_MODELS[model]["revision"])
                        output = root / f"installed-{model}"
                        options = argparse.Namespace(
                            archive=archive, output=output, state_dir=state, target=TARGET,
                            archive_sha256=receipt["archive_sha256"],
                            manifest_sha256=receipt["manifest_sha256"],
                        )
                        installed = pack.install(options)
                        self.assertEqual(installed["chat_model"], model)
                        self.assertEqual(json.loads(pointer.read_text())["root"], str(output.resolve()))
                        previous = pointer.read_text()
                        options.output = root / f"named-{model}"
                        options.chat_model = model
                        self.assertEqual(pack.install(options)["chat_model"], model)
                        named = state / "qa-model-packs" / f"{model}.json"
                        self.assertEqual(json.loads(named.read_text())["root"], str(options.output.resolve()))
                        self.assertEqual(pointer.read_text(), previous)

    def test_generic_import_rejects_mixed_pinned_model_entries(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "archive.tar.gz"
            gpt2 = "model/generator/merges.txt"
            flan = "model/generator/spiece.model"
            with tarfile.open(archive, "w:gz") as target:
                for name in (gpt2, flan):
                    info = tarfile.TarInfo(name)
                    info.size = 1
                    target.addfile(info, io.BytesIO(b"x"))
            output = root / "output"
            output.mkdir()
            with patch.dict(pack.GENERATOR_FILES, {
                "gpt2-small": {gpt2: (1, None)},
                "flan-t5-small": {flan: (1, None)},
            }):
                with self.assertRaisesRegex(ValueError, "does not match the pinned generator size"):
                    pack.extract(archive, output, TARGET)
            self.assertFalse((output / flan).exists())


if __name__ == "__main__":
    unittest.main()
