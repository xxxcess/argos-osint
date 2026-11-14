"""Offline contracts for privileged UI visual-review artifact handling."""

import importlib.util
import json
import struct
import tempfile
import unittest
import zlib
from argparse import Namespace
from pathlib import Path
from unittest.mock import patch

MODULE = Path(__file__).with_name("ui_visual_review.py")
SPEC = importlib.util.spec_from_file_location("ui_visual_review", MODULE)
review = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(review)
HEAD = "a" * 40


def chunk(kind, payload):
    return (struct.pack(">I", len(payload)) + kind + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF))


def png():
    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(b"\x00" + b"\x00" * 192) )
            + chunk(b"IEND", b""))


class UiVisualReviewTests(unittest.TestCase):
    def test_scene_names_are_bounded_and_unique(self):
        self.assertEqual(review.scenes("registration-ready,jobs-ready"),
                         ["registration-ready", "jobs-ready"])
        for value in ("", "registration-ready,registration-ready", "scene;echo bad", ",jobs-ready"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                review.scenes(value)

    def test_png_crc_and_symlink_escape_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "capture.png"
            path.write_bytes(png())
            self.assertEqual(review.png_dimensions(path), (64, 64))
            damaged = bytearray(path.read_bytes())
            damaged[25] ^= 1
            path.write_bytes(damaged)
            with self.assertRaisesRegex(ValueError, "CRC mismatch"):
                review.png_dimensions(path)
            outside = root.parent / "untrusted.png"
            (root / "escape.png").symlink_to(outside)
            with self.assertRaisesRegex(ValueError, "escapes"):
                review.safe_file(root, Path("escape.png"))

    def test_prepare_requires_exact_manifest_and_writes_local_markdown_references(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            fixture = root / f"Fleetworks-ui-native-{HEAD}" / "fixtures"
            fixture.mkdir(parents=True)
            name = "registration-ready-1440x860-1x.png"
            (fixture / name).write_bytes(png())
            manifest = {
                "source_revision": HEAD, "dirty": False, "state": "generated",
                "captures": [{"scene": "registration-ready", "size": "1440x860",
                              "scale": 1, "script": None, "generated": True, "png": name}],
            }
            (fixture / "manifest.json").write_text(json.dumps(manifest))
            output = root / "comment"
            args = Namespace(artifact_root=root, head=HEAD, scope="native",
                             native_scenes="registration-ready", native_script="none",
                             web_surface="all", run_url="https://github.com/a/b/actions/runs/1",
                             body=output / "body.md", attachments=output / "attachments.json",
                             archive=output / "images.zip")
            review.prepare(args)
            self.assertIn("![Native registration-ready at 1440×860](images/",
                          args.body.read_text())
            paths = json.loads(args.attachments.read_text())
            self.assertEqual(len(paths), 1)
            self.assertTrue(Path(paths[0]).is_file())
            manifest["source_revision"] = "b" * 40
            (fixture / "manifest.json").write_text(json.dumps(manifest))
            with self.assertRaisesRegex(ValueError, "exact clean source"):
                review.prepare(args)

    def test_dispatch_rejects_stale_pr_head_before_artifact_work(self):
        args = Namespace(repo="example/repo", pr="7", head=HEAD, scope="native",
                         native_scenes="registration-ready", native_script="none",
                         web_surface="all", release_tag="")
        pr = {"state": "open", "head": {"sha": "b" * 40,
                                        "repo": {"full_name": "example/repo"}},
              "base": {"ref": "nightly", "repo": {"full_name": "example/repo"}}}
        with patch.dict("os.environ", {"GITHUB_REPOSITORY": "example/repo"}), \
             patch.object(review, "api_json", return_value=pr), \
             self.assertRaisesRegex(ValueError, "exact selected head"):
            review.validate(args)


if __name__ == "__main__":
    unittest.main()
