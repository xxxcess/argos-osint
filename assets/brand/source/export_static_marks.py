"""Export the fixed crown identity for the repository and app resources.

Run from any directory with Pillow and Inkscape installed. The approved
photograph supplies the large JPEG; the canonical optical-size SVG supplies
the transparent or matte-backed marks, including every Mac icon resolution.
Quiet Orbit motion assets have an independent geometry and pipeline.
"""

import io
import struct
import subprocess
import tempfile
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
BRAND = ROOT / "assets/brand"
WEB = ROOT / "apps/web-desktop"
RESOURCES = ROOT / "apps/fleetworks-desktop/resources"


def main():
    mark = (BRAND / "fleetworks-mark.svg").read_text()
    header, body = mark.split("\n", 1)
    backed = header + '\n  <rect width="256" height="256" fill="#060709"/>\n' + body
    (WEB / "public/fleetworks-mark.svg").write_text(mark)
    for output in (BRAND / "fleetworks-logo.svg", WEB / "app/icon.svg", WEB / "app/apple-icon.svg"):
        output.write_text(backed)
    # Desktop launchers supply their own background. Preserve transparent corners.
    launcher = backed.replace('<rect width="256" height="256"', '<rect rx="56" width="256" height="256"', 1)
    (RESOURCES / "fleetworks.svg").write_text(launcher)

    component = ROOT / "packages/ui/src/FleetworksMark.tsx"
    before, remainder = component.read_text().split("const MARK = `", 1)
    _, after = remainder.split("`;", 1)
    component.write_text(before + "const MARK = `" + mark.rstrip() + "`;" + after)

    with Image.open(BRAND / "source/fleetworks-liquid-metal-glass.png") as source:
        source.thumbnail((1024, 1024), Image.Resampling.LANCZOS)
        source.save(BRAND / "fleetworks-logo.jpg", quality=90, optimize=True)

    # Each chunk carries a PNG at its native resolution. Render directly at each
    # size so small highlights align to the pixel grid without a large source bitmap.
    with tempfile.TemporaryDirectory() as scratch:
        raster = Path(scratch) / "icon.png"
        chunks = []
        for size, kinds in (
            (16, (b"icp4",)), (32, (b"icp5", b"ic11")), (64, (b"icp6", b"ic12")),
            (128, (b"ic07",)), (256, (b"ic08", b"ic13")), (512, (b"ic09", b"ic14")),
            (1024, (b"ic10",)),
        ):
            subprocess.run([
                "inkscape", str(RESOURCES / "fleetworks.svg"), f"--export-width={size}",
                f"--export-height={size}", f"--export-filename={raster}",
            ], check=True)
            with Image.open(raster) as rendered:
                png = io.BytesIO()
                rendered.convert("RGBA").save(png, format="PNG", optimize=True)
            payload = png.getvalue()
            chunks.extend(kind + struct.pack(">I", len(payload) + 8) + payload for kind in kinds)
        contents = b"".join(chunks)
        (RESOURCES / "fleetworks.icns").write_bytes(b"icns" + struct.pack(">I", len(contents) + 8) + contents)


if __name__ == "__main__":
    main()
