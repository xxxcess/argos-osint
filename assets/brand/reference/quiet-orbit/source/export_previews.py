#!/usr/bin/env python3
"""Encode Fleetworks splash frame sequences with FFmpeg; no image painting.

Examples:
  python export_previews.py export ./frames/01 --name 01-obsidian --output ./deliverables
  python export_previews.py contact ./deliverables/*.mp4 --output ./deliverables

The renderer owns the images. This helper only orders, scales, encodes, and
composites them, then records container metadata for review.
"""

from __future__ import annotations

import argparse
import json
import math
import re
import shutil
import subprocess
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path


def run(command: list[str]) -> str:
    completed = subprocess.run(command, capture_output=True, text=True)
    if completed.returncode:
        raise RuntimeError("Command failed:\n" + " ".join(command) + "\n" + completed.stderr[-6000:])
    return completed.stdout


def ffmpeg(*arguments: str) -> None:
    run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-threads", "2",
         "-filter_threads", "1", "-filter_complex_threads", "1", *map(str, arguments)])


def probe(path: Path, count_frames: bool = False) -> dict:
    arguments = ["ffprobe", "-v", "error", "-select_streams", "v:0"]
    if count_frames:
        arguments += ["-count_frames"]
    arguments += ["-show_entries",
                  "stream=codec_name,width,height,nb_frames,nb_read_frames,avg_frame_rate,duration:format=duration,size",
                  "-of", "json", str(path)]
    raw = json.loads(run(arguments))
    stream = raw.get("streams", [{}])[0]
    duration = stream.get("duration", raw.get("format", {}).get("duration"))
    return {
        "path": str(path.resolve()),
        "bytes": path.stat().st_size,
        "size_mib": round(path.stat().st_size / 1024**2, 3),
        "width": stream.get("width"),
        "height": stream.get("height"),
        "codec": stream.get("codec_name"),
        "frames": stream.get("nb_read_frames", stream.get("nb_frames")),
        "average_frame_rate": stream.get("avg_frame_rate"),
        "duration_seconds": float(duration) if duration is not None else None,
    }


def natural_key(path: Path) -> list:
    return [int(item) if item.isdigit() else item.lower()
            for item in re.split(r"(\d+)", path.name)]


@contextmanager
def sequence_input(directory: Path, output: Path, input_fps: float):
    frames = sorted((p for p in directory.iterdir()
                     if p.is_file() and p.suffix.lower() == ".png"), key=natural_key)
    if len(frames) < 2:
        raise ValueError(f"Expected at least two PNG frames in {directory}; got {len(frames)}")
    with tempfile.TemporaryDirectory(prefix=".export-", dir=output) as temp:
        staged = Path(temp)
        for index, frame in enumerate(frames):
            (staged / f"frame-{index:06d}.png").symlink_to(frame.resolve())
        yield ["-framerate", str(input_fps), "-start_number", "0", "-i",
               str(staged / "frame-%06d.png")], frames, staged


def size_filter(width: int) -> str:
    # Do not upscale. Force even output dimensions for H.264 4:2:0.
    return f"scale=w='min({width},iw)':h=-2:flags=lanczos"


def encode_gif(input_arguments: list[str], destination: Path, temporary: Path,
               fps: float, width: int, maximum_mib: float,
               duration: float | None = None) -> dict:
    attempts = []
    widths = list(dict.fromkeys([width, min(width, 720), min(width, 640), min(width, 560)]))
    widths = [w for w in widths if w > 0]
    timings = ["-t", f"{duration:.6f}"] if duration is not None else []
    palette = temporary / "palette.png"
    for candidate_width in widths:
        prefilter = f"fps={fps},{size_filter(candidate_width)}"
        ffmpeg(*input_arguments, *timings, "-vf",
               prefilter + ",palettegen=max_colors=256:reserve_transparent=1:stats_mode=full",
               "-frames:v", "1", "-update", "1", str(palette))
        ffmpeg(*input_arguments, "-i", str(palette), *timings, "-filter_complex",
               f"[0:v]{prefilter}[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle",
               "-loop", "0", "-gifflags", "+transdiff", str(destination))
        information = probe(destination, count_frames=True)
        attempts.append({"max_width": candidate_width, "size_mib": information["size_mib"]})
        if information["bytes"] <= maximum_mib * 1024**2:
            break
    information["size_attempts"] = attempts
    information["size_limit_mib"] = maximum_mib
    information["within_size_limit"] = information["bytes"] <= maximum_mib * 1024**2
    if not information["within_size_limit"]:
        raise RuntimeError(f"{destination.name} exceeds {maximum_mib} MiB even at {widths[-1]}px; "
                           "shorten the supplied preview or explicitly raise --max-mib.")
    return information


def export_frames(arguments: argparse.Namespace) -> dict:
    directory = arguments.frames.resolve()
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    name = re.sub(r"[^A-Za-z0-9._-]+", "-", arguments.name or directory.name).strip(".-")
    if not name:
        raise ValueError("Output name must contain letters or numbers")
    input_fps = arguments.input_fps or arguments.fps
    if input_fps <= 0 or arguments.fps <= 0:
        raise ValueError("Frame rates must be positive")
    paths = {suffix: output / f"{name}.{suffix}" for suffix in ("gif", "mp4", "png", "json")}
    with sequence_input(directory, output, input_fps) as (inputs, frames, temporary):
        duration = len(frames) / input_fps
        video_filter = f"fps={arguments.fps},{size_filter(arguments.video_width)},format=yuv420p"
        ffmpeg(*inputs, "-vf", video_filter, "-an", "-c:v", "libx264", "-preset", "medium",
               "-crf", "18", "-movflags", "+faststart", str(paths["mp4"]))
        video = probe(paths["mp4"], count_frames=True)
        gif = encode_gif(inputs, paths["gif"], temporary, arguments.fps,
                         arguments.width, arguments.max_mib, duration)
        poster_time = (arguments.poster_time if arguments.poster_time is not None
                       else duration * 0.62)
        poster_time = max(0, min(poster_time, max(0, duration - 1 / input_fps)))
        ffmpeg(*inputs, "-ss", f"{poster_time:.6f}", "-vf", size_filter(arguments.video_width),
               "-frames:v", "1", "-update", "1", str(paths["png"]))
        report = {
            "name": name,
            "source_directory": str(directory),
            "source_frame_count": len(frames),
            "source_first_frame": frames[0].name,
            "source_last_frame": frames[-1].name,
            "input_fps": input_fps,
            "requested_output_fps": arguments.fps,
            "source_duration_seconds": duration,
            "gif": gif,
            "mp4": video,
            "poster": {"path": str(paths["png"]), "time_seconds": poster_time},
            "encoding": {
                "gif_palette": "global, 255 opaque colors plus one transparency entry",
                "gif_dither": "deterministic Bayer, scale 4; changed-rectangle processing",
                "gif_loop": "infinite",
                "mp4": "H.264, CRF 18, yuv420p, faststart",
            },
        }
        expected_frames = round(duration * arguments.fps)
        actual_frames = int(gif.get("frames") or 0)
        if actual_frames < 2:
            raise RuntimeError(f"GIF does not contain an animation: {gif}")
        report["qa"] = {
            "expected_output_frames": expected_frames,
            "gif_decoded_frames": actual_frames,
            "frame_count_matches": abs(actual_frames - expected_frames) <= 1,
            "duration_difference_seconds": round((gif["duration_seconds"] or 0) - duration, 4),
            "decoded_successfully": True,
        }
        paths["json"].write_text(json.dumps(report, indent=2) + "\n")
    return report


def filter_path(path: Path) -> str:
    # Paths are arguments to the filter parser, never shell snippets.
    return str(path.resolve()).replace("\\", "\\\\").replace(":", "\\:").replace("'", "'\\''")


def contact_sheet(arguments: argparse.Namespace) -> dict:
    if len(arguments.videos) != 5:
        raise ValueError("The comparison expects exactly five MP4 inputs")
    videos = [path.resolve() for path in arguments.videos]
    output = arguments.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    labels = arguments.labels or [p.stem.replace("-", " ") for p in videos]
    if len(labels) != 5:
        raise ValueError("Provide exactly five --labels values")
    font = Path(arguments.font)
    if not font.is_file():
        raise ValueError(f"Missing font: {font}")
    information = [probe(path) for path in videos]
    duration = min(item["duration_seconds"] for item in information)
    cell_width = arguments.cell_width
    image_height = int(math.ceil((cell_width * 9 / 16) / 2) * 2)
    cell_height = image_height + 28
    mp4 = output / "fleetworks-five-approaches.mp4"
    gif = output / "fleetworks-five-approaches.gif"
    poster = output / "fleetworks-five-approaches.png"
    with tempfile.TemporaryDirectory(prefix=".contact-", dir=output) as temp:
        temporary = Path(temp)
        input_arguments = []
        filters = []
        for index, (video, label) in enumerate(zip(videos, labels)):
            input_arguments += ["-i", str(video)]
            text_file = temporary / f"label-{index}.txt"
            text_file.write_text(label)
            filters.append(
                f"[{index}:v]fps={arguments.fps},"
                f"scale={cell_width}:{image_height}:force_original_aspect_ratio=decrease:flags=lanczos,"
                f"pad={cell_width}:{image_height}:(ow-iw)/2:(oh-ih)/2:color=0x050507,"
                f"pad={cell_width}:{cell_height}:0:0:color=0x050507,setsar=1,"
                f"drawtext=fontfile='{filter_path(font)}':textfile='{filter_path(text_file)}':"
                f"fontcolor=0xDED9CD:fontsize=13:x=12:y={image_height + 7}[v{index}]"
            )
        input_arguments += ["-f", "lavfi", "-i",
                            f"color=c=0x050507:s={cell_width}x{cell_height}:r={arguments.fps}:d={duration}"]
        filters.append(
            f"[5:v]drawtext=fontfile='{filter_path(font)}':text='FLEETWORKS':"
            f"fontcolor=0xDED9CD:fontsize=21:x=(w-tw)/2:y=(h-th)/2-13,"
            f"drawtext=fontfile='{filter_path(font)}':text='FIVE SPLASH CONCEPTS':"
            f"fontcolor=0x8E8A82:fontsize=11:x=(w-tw)/2:y=(h-th)/2+18[v5]"
        )
        layout = (f"0_0|{cell_width}_0|{cell_width * 2}_0|"
                  f"0_{cell_height}|{cell_width}_{cell_height}|{cell_width * 2}_{cell_height}")
        filters.append("[v0][v1][v2][v3][v4][v5]"
                       f"xstack=inputs=6:layout={layout}:shortest=1,format=yuv420p[out]")
        ffmpeg(*input_arguments, "-filter_complex", ";".join(filters), "-map", "[out]",
               "-t", f"{duration:.6f}", "-an", "-c:v", "libx264", "-preset", "medium",
               "-crf", "18", "-movflags", "+faststart", str(mp4))
        gif_information = encode_gif(["-i", str(mp4)], gif, temporary,
                                     arguments.fps, cell_width * 3, arguments.max_mib, duration)
        ffmpeg("-ss", f"{duration * 0.62:.6f}", "-i", str(mp4),
               "-frames:v", "1", "-update", "1", str(poster))
        report = {"sources": information, "labels": labels, "gif": gif_information,
                  "mp4": probe(mp4, True), "poster": str(poster)}
        (output / "fleetworks-five-approaches.json").write_text(json.dumps(report, indent=2) + "\n")
        return report


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    subcommands = result.add_subparsers(dest="command", required=True)
    frames = subcommands.add_parser("export", help="Encode one directory of PNG frames")
    frames.add_argument("frames", type=Path)
    frames.add_argument("--output", type=Path, required=True)
    frames.add_argument("--name", help="Safe output basename; defaults to frame directory name")
    frames.add_argument("--fps", type=float, default=20)
    frames.add_argument("--input-fps", type=float, help="Defaults to --fps")
    frames.add_argument("--width", type=int, default=800, help="Maximum GIF width")
    frames.add_argument("--video-width", type=int, default=960, help="Maximum MP4/poster width")
    frames.add_argument("--max-mib", type=float, default=15, help="GIF size ceiling before width retries")
    frames.add_argument("--poster-time", type=float, help="Seconds from start; default 62%% of duration")
    frames.set_defaults(handler=export_frames)
    contact = subcommands.add_parser("contact", help="Encode labeled comparison from exactly five MP4s")
    contact.add_argument("videos", nargs="+", type=Path)
    contact.add_argument("--labels", nargs=5)
    contact.add_argument("--output", type=Path, required=True)
    contact.add_argument("--fps", type=float, default=20)
    contact.add_argument("--cell-width", type=int, default=320)
    contact.add_argument("--max-mib", type=float, default=15)
    contact.add_argument("--font", default="/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
    contact.set_defaults(handler=contact_sheet)
    return result


def main() -> int:
    arguments = parser().parse_args()
    for executable in ("ffmpeg", "ffprobe"):
        if not shutil.which(executable):
            raise RuntimeError(f"Required executable missing: {executable}")
    report = arguments.handler(arguments)
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (RuntimeError, ValueError, OSError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
