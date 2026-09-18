#!/usr/bin/env python3
"""Capture actual Steelix PTY output with scripted demo responses, no credentials."""

import argparse
import codecs
import fcntl
import json
import os
import pty
import select
import shutil
import struct
import subprocess
import tempfile
import termios
import time
from pathlib import Path

import pyte
from PIL import Image, ImageDraw, ImageFont
from terminal_screen import Screen

COLS, ROWS = 112, 32
CELL_W, CELL_H = 10, 20
PAD = 18
BACKGROUND, FOREGROUND = "#0b0e14", "#e6e1cf"
ANSI = {
    "black": "#0b0e14",
    "red": "#f07178",
    "green": "#aad94c",
    "brown": "#e6b450",
    "blue": "#59c2ff",
    "magenta": "#d2a6ff",
    "cyan": "#95e6cb",
    "white": "#e6e1cf",
}


def color(value, default):
    if value == "default":
        return default
    return ANSI.get(value, "#" + value)


def render(screen, font):
    image = Image.new(
        "RGB", (COLS * CELL_W + 2 * PAD, ROWS * CELL_H + 2 * PAD), BACKGROUND
    )
    draw = ImageDraw.Draw(image)
    for y in range(ROWS):
        for x in range(COLS):
            cell = screen.buffer[y][x]
            fg, bg = color(cell.fg, FOREGROUND), color(cell.bg, BACKGROUND)
            if cell.reverse:
                fg, bg = bg, fg
            px, py = PAD + x * CELL_W, PAD + y * CELL_H
            if bg != BACKGROUND:
                draw.rectangle((px, py, px + CELL_W - 1, py + CELL_H - 1), fill=bg)
            if cell.data.strip():
                draw.text((px, py), cell.data, font=font, fill=fg, stroke_width=0)
    return image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("hx", type=Path)
    parser.add_argument("runtime", type=Path)
    parser.add_argument(
        "--font", type=Path, required=True, help="Monospace TTF for rendering PTY cells"
    )
    parser.add_argument("--output", type=Path, default=Path("docs/media"))
    args = parser.parse_args()
    project = Path(__file__).resolve().parents[1]
    args.output.mkdir(parents=True, exist_ok=True)
    font = ImageFont.truetype(str(args.font), 16)
    with tempfile.TemporaryDirectory(prefix="tandem-demo-") as directory:
        root = Path(directory)
        repo, config = root / "retry-demo", root / "config/helix"
        (repo / "src").mkdir(parents=True)
        config.mkdir(parents=True)
        (config / "runtime").symlink_to(
            args.runtime.resolve(), target_is_directory=True
        )
        for name in ("main.rs", "retry.rs"):
            shutil.copyfile(project / "scripts/demo" / name, repo / "src" / name)
        shutil.copytree(repo / "src", repo / ".preview/src")
        (repo / ".gitignore").write_text(".preview/\n")
        subprocess.run(["git", "init", "-q", str(repo)], check=True)
        shutil.copytree(project / "helix/tandem", config / "tandem")
        helper = root / "bin"
        helper.mkdir()
        backend = (project / "scripts/demo/backend.py").read_text()
        (helper / "tandem").write_text(backend)
        (helper / "tandem").chmod(0o755)
        (config / "config.toml").write_text(
            'theme = "ayu_dark"\n[editor]\ntrue-color = true\ninsecure = true\n[editor.soft-wrap]\nenable = true\n'
        )
        (config / "languages.toml").write_text(
            '[[language]]\nname = "rust"\nlanguage-servers = []\n'
        )
        (config / "helix.scm").write_text("")
        (config / "init.scm").write_text(
            '(require "tandem/tandem.scm")\n(require "helix/misc.scm")\n(require (prefix-in cmd. "helix/commands.scm"))\n(enqueue-thread-local-callback (lambda () (cmd.set-language "rust") (tandem)))\n'
        )
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        env = dict(
            os.environ,
            HELIX_RUNTIME=str(args.runtime.resolve()),
            HELIX_STEEL_CONFIG=str(config),
            XDG_CONFIG_HOME=str(root / "config"),
            XDG_DATA_HOME=str(root / "data"),
            STEEL_HOME=str(root / "steel"),
            TERM="xterm-256color",
            PATH=str(helper) + ":" + os.environ["PATH"],
        )
        process = subprocess.Popen(
            [
                str(args.hx.resolve()),
                "-c",
                str(config / "config.toml"),
                "--log",
                str(root / "editor.log"),
                "src/main.rs",
            ],
            cwd=repo,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            start_new_session=True,
        )
        os.close(slave)
        screen = Screen(COLS, ROWS)
        stream = pyte.Stream(screen)
        decoder = codecs.getincrementaldecoder("utf8")("replace")
        start = time.monotonic()
        events, frames = [], []
        last_frame = 0.0

        def pump(seconds):
            nonlocal last_frame
            end = time.monotonic() + seconds
            while time.monotonic() < end:
                if select.select([master], [], [], 0.04)[0]:
                    try:
                        raw = os.read(master, 65536)
                    except OSError:
                        break
                    if not raw:
                        break
                    value = decoder.decode(raw)
                    if "panicked" in value or "error[E" in value:
                        raise RuntimeError(value)
                    events.append([round(time.monotonic() - start, 3), "o", value])
                    stream.feed(value)
                elapsed = time.monotonic() - start
                if elapsed - last_frame >= 0.25:
                    frames.append(render(screen, font))
                    last_frame = elapsed

        def send(value, pause=0.6):
            os.write(master, value.encode())
            pump(pause)

        def type_message(value):
            for index in range(0, len(value), 3):
                send(value[index : index + 3], 0.06)
            send("\r", 1.4)

        def expect(value):
            if value not in "\n".join(screen.display):
                raise RuntimeError(
                    f"Missing demo frame: {value}\n" + "\n".join(screen.display)
                )

        try:
            pump(2)
            expect("Tandem")
            if "Failed to compile highlights" in (root / "editor.log").read_text():
                raise RuntimeError(
                    "Rust highlight queries do not match the grammar. Supply a matching editor runtime."
                )
            frames.clear()
            type_message("Walk me through the retry flow.")
            expect("Start with the caller")
            pump(1.8)
            send("\x1b", 0.2)
            send("]t", 2.4)
            expect("The wait doubles each time")
            render(screen, font).save(args.output / "tour.png")
            tour_frames = list(frames)
            send(":tandem\r", 0.3)
            type_message("/begin Cap retry delays at 30 seconds.")
            expect("Put a ceiling on the wait")
            send("\x1b", 0.2)
            send(" tp", 1.0)
            send(":redraw\r", 0.4)
            expect("Old · src/retry.rs")
            render(screen, font).save(args.output / "comparison.png")
            pump(2)
            send(" tp", 0.5)
            send(":tandem\r", 0.3)
            type_message("Why clamp the exponent too?")
            expect("Clamping the exponent")
            render(screen, font).save(args.output / "conversation.png")
            pump(2)
            header = dict(
                version=2,
                width=COLS,
                height=ROWS,
                title="Tandem: guided tours and native comparison (scripted demo)",
                env=dict(TERM="xterm-256color"),
            )
            with (args.output / "walkthrough.cast").open("w") as recording:
                recording.write(json.dumps(header) + "\n")
                for event in events:
                    recording.write(json.dumps(event) + "\n")
            # GIF is a rendering of the same captured terminal cells, not a UI mockup.
            palette = tour_frames[-1].quantize(colors=128)
            animated = [
                frame.quantize(palette=palette, dither=Image.Dither.NONE)
                for frame in tour_frames
            ]
            animated[0].save(
                args.output / "tour.gif",
                save_all=True,
                append_images=animated[1:],
                duration=250,
                loop=0,
                optimize=True,
            )
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            os.close(master)
    print(f"Recorded Steelix demo in {args.output}")


if __name__ == "__main__":
    main()
