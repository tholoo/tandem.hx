#!/usr/bin/env python3
"""Render real Steel components against a deterministic local transport.
Run in nix develop: python scripts/check-helix-ui.py HX RUNTIME [PLUGIN_DIRECTORY]
Requires pyte. Never opens a desktop window or touches a real repository.
"""
import codecs
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time
import pyte
from terminal_screen import Screen

hx, runtime = sys.argv[1:3]
plugin = Path(sys.argv[3]) if len(sys.argv) > 3 else Path(__file__).resolve().parents[1] / "helix/tandem"
root = Path(tempfile.mkdtemp(prefix="tandem-ui-check-", dir="/tmp"))
config = root / "config/helix"
config.mkdir(parents=True)
shutil.copytree(plugin, config / "tandem", copy_function=shutil.copyfile)
with (config / "tandem/tandem.scm").open("a") as out:
    out.write("\n(provide publish-context)\n")
source = root / "story.txt"
source.write_text("".join(f"source line {n:03d}\n" for n in range(1, 241)))
subprocess.run(["git", "init", "-q", str(root)], check=True)
subprocess.run(["git", "-C", str(root), "add", "story.txt"], check=True)
subprocess.run(["git", "-C", str(root), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"], check=True)
source.write_text(source.read_text().replace("source line 120", "source line 120 changed")
                  .replace("source line 122", "source line 122 " + "wrapped text " * 12))
view = dict(stage="discuss", busy=False, generation=1, real=str(root), shadow=str(root / "preview"),
            navigation=dict(file=str(source), line=120),
            tour=dict(id=1, source=dict(kind="repository"), current_stop=1,
                      title="Fixture tour", overview="Overview", stops=[dict(title="The request handler", body="Credentials are parsed here. Run `:tandem` to ask a question.", file="story.txt", line=120, end_line=135)]))
helper = root / "bin"
helper.mkdir()
(helper / "tandem").write_text("#!" + sys.executable + "\n" +
    "import sys,json,time\n" +
    "events=" + repr([dict(event="connected", project=str(root)), dict(event="state",view=view),
                       dict(event="message",text="You: First question"),
                       dict(event="message",text="You: Two lines\nSecond line"),
                       dict(event="message",text="You: /next"),
                       dict(event="message",text="Assistant: **Output:** Try `:tandem` here.\n**Read `src.rs` now**\n`**literal**` and **unfinished")]) + "\n" +
    "for event in events: print(json.dumps(event),flush=True)\n" +
    "comparison=" + repr(dict(file="story.txt", current_file=str(source),
        old="".join(f"source line {n:03d}\n" for n in range(1, 241)), current=source.read_text(),
        old_line=120, current_line=120)) + "\n" +
    "peeks=0\n" +
    "for line in sys.stdin:\n" +
    " with open(" + repr(str(root / "requests.jsonl")) + ", 'a') as log: log.write(line)\n" +
    " request=json.loads(line)\n" +
    " if request['action']=='peek':\n" +
    "  peeks+=1\n" +
    "  reply=dict(text='saved change') if peeks==1 else dict(comparison=comparison)\n" +
    "  print(json.dumps(dict(id=request['id'],**reply)),flush=True)\n")
(helper / "tandem").chmod(0o700)
(config / "themes").mkdir()
# Reproduce Stylix's identical ui.text/ui.text.focus, with its distinct accents.
(config / "themes/fixture.toml").write_text('''"ui.background" = { bg = "#0b0e14" }
"ui.text" = "#e6e1cf"
"ui.text.focus" = "#e6e1cf"
"ui.popup" = { bg = "#131721" }
"ui.cursorline.primary" = { bg = "#131721" }
"ui.cursor.primary" = { modifiers = ["reversed"] }
"ui.selection" = { bg = "#202229" }
"ui.menu.selected" = { fg = "#59c2ff", bg = "#202229" }
"ui.window" = "#636a76"
"function" = "#59c2ff"
"keyword" = "#d2a6ff"
"constant" = "#ff8f40"
"markup.raw" = "#aad94c"
"diff.delta" = "#ff8f40"
"error" = "#f07178"
"warning" = "#ff8f40"
''')
(config / "config.toml").write_text('theme = "fixture"\n[editor]\ntrue-color = true\nline-number = "relative"\ninsecure = true\n[editor.soft-wrap]\nenable = true\n')
(config / "helix.scm").write_text("")
(config / "init.scm").write_text(f'''(require "tandem/tandem.scm")
(require "helix/misc.scm")
(require "helix/ext.scm")
(require (prefix-in cmd. "helix/commands.scm"))
(require-builtin helix/core/static as edit.)
(require "helix/editor.scm")
(require "tandem/rich.scm")
(require-builtin steel/filesystem)
(require-builtin steel/time)
(define (wrapped text width)
  (map (lambda (line) (list->string (map car line))) (rich-lines text width)))
(enqueue-thread-local-callback (lambda ()
  (assert! (equal? (wrapped "update the caller" 9) '("update" "the" "caller")))
  (assert! (equal? (wrapped "abcde f" 5) '("abcde" "f")))
  (assert! (equal? (wrapped "abcdefghij" 4) '("abcd" "efgh" "ij")))
  (assert! (equal? (wrapped "a\\n\\nb" 4) '("a" "" "b")))
  (assert! (equal? (wrapped "  hello world" 8) '("  hello" "world")))
  (assert! (equal? (wrapped "a b" 1) '("a" "b")))
  (assert! (equal? (wrapped "aa **the** /begin" 7) '("aa the" "/begin")))
  (assert! (equal? (map cdr (car (rich-lines "aa **the** /begin" 7))) '(text text text bold bold bold)))
  (assert! (equal? (map cdr (cadr (rich-lines "aa **the** /begin" 7))) '(code code code code code code)))
  (cmd.theme "fixture") (tandem)))
(spawn-native-thread (lambda ()
  (let wait-scroll ()
    (if (path-exists? {json.dumps(str(root / "scroll"))})
        (hx.block-on-task (lambda ()
          (cmd.goto-line 130) (edit.align_view_top) (set-status! "RANGE-MOVED")))
        (begin (time/sleep-ms 25) (wait-scroll))))))
''')
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
process = subprocess.Popen([hx, "-c", str(config / "config.toml"), "--log", str(root / "helix.log"), str(source)], stdin=slave, stdout=slave, stderr=slave,
    cwd=root, start_new_session=True,
    env=dict(os.environ, XDG_CONFIG_HOME=str(root / "config"), XDG_DATA_HOME=str(root / "data"),
             HELIX_STEEL_CONFIG=str(config), HELIX_RUNTIME=runtime, STEEL_HOME=str(root / "steel"),
             TERM="xterm-256color", RUST_BACKTRACE="1", PATH=str(helper)+":"+os.environ["PATH"]))
os.close(slave)
screen = Screen(120, 40)
stream = pyte.Stream(screen)
decoder = codecs.getincrementaldecoder("utf8")("replace")
capture = bytearray()
def drain():
    while select.select([master], [], [], 0)[0]:
        try: data = os.read(master, 65536)
        except OSError: break
        if not data: break
        capture.extend(data)
        stream.feed(decoder.decode(data))
def locate(text):
    for row,line in enumerate(screen.display):
        col=line.find(text)
        if col >= 0: return row,col
    return None
errors=[]
try:
    deadline=time.monotonic()+8
    while time.monotonic()<deadline:
        drain()
        if locate("Assistant:") and locate("The request handler"): break
        if process.poll() is not None or b"error[E" in capture: break
        time.sleep(.05)
    time.sleep(.3)
    drain()
    if b"error[E" in capture or b"panicked" in capture:
        raise RuntimeError(f"Steel initialization check failed; inspect {root / 'terminal.bin'}")
    for label in ["You:", "Assistant:", "/next", ":tandem"]:
        pos=locate(label)
        if pos is None: errors.append(f"missing rendered {label}")
    if all(locate(t) for t in ["You:", "Assistant:", "/next", ":tandem"]):
        colors={t:screen.buffer[locate(t)[0]][locate(t)[1]].fg for t in ["You:", "Assistant:", "/next", ":tandem"]}
        (root / "colors.json").write_text(json.dumps(colors))
        if colors["You:"] == colors["Assistant:"]: errors.append(f"role labels share a color: {colors}")
        if colors["/next"] == "e6e1cf" or colors[":tandem"] == "e6e1cf": errors.append(f"commands have plain-text color: {colors}")
    for text in ["Output:", "Read", "src.rs", "now"]:
        pos = locate(text)
        if pos is None or not screen.buffer[pos[0]][pos[1]].bold:
            errors.append(f"Markdown strong text is not bold: {text}")
    if locate("**Output:**") or locate("**Read"):
        errors.append("Markdown bold delimiters are visible")
    if not locate("**literal**") or not locate("**unfinished"):
        errors.append("literal code or unmatched bold delimiters were lost")
    if not locate("Repository · 1 / 1") or not locate("story.txt:120–135 · 16 lines"):
        errors.append("missing tour source or inclusive reading range")
    if not locate("Context: story.txt:120"):
        errors.append("missing current code context")
    target=locate("source line 120")
    if target is None or not 5 <= target[0] <= 10: errors.append(f"reading range is not framed from its start: {target}")
    end=locate("source line 135")
    if target is None or screen.buffer[target[0]][target[1] + 2].bg != "202229":
        errors.append("reading range start is not highlighted")
    if end is None or screen.buffer[end[0]][end[1]].bg != "202229":
        errors.append("reading range end is not highlighted")
    after=locate("source line 136")
    if after is None or screen.buffer[after[0]][after[1]].bg == "202229":
        errors.append("highlight extends past the reading range")
    if target and end and end[0] - target[0] <= 15:
        errors.append("wrapped reading range did not occupy extra visual rows")
    if target:
        before=screen.display[target[0]][:target[1]]
        if not any(mark in before for mark in ["▌", "▎", "▍", "▏", "▐", "┆"]):
            errors.append(f"Git change indicator missing beside reading range: {before!r}")
    narration=locate("The request handler")
    if narration is None or narration[0] > 2: errors.append(f"narration is not above the code: {narration}")
    (root / "screen.txt").write_text("\n".join(screen.display))
    def wait_screen(predicate, label):
        deadline=time.monotonic()+6
        while time.monotonic()<deadline:
            drain()
            if predicate(): return
            time.sleep(.05)
        errors.append(label)
    (root / "scroll").touch()
    wait_screen(lambda: locate("source line 130") is not None and locate("source line 130")[0] < 10, "scroll did not move range")
    if not locate("story.txt:120–135 · 16 lines"):
        errors.append("reading range label disappeared after moving")
    end=locate("source line 135")
    if end and screen.buffer[end[0]][end[1]].bg == "202229":
        errors.append("tour selection did not clear after moving")
    os.write(master, b"Unsent draft\x01\x1b[C\x1b[C")
    wait_screen(lambda: screen.display[-1].rstrip().endswith("> Unsent draft")
                and screen.cursor.x == screen.display[-1].index("> Unsent draft") + 4,
                "chat draft and cursor were not entered")
    draft_cursor=screen.cursor.x
    os.write(master, b"\x1b[A")
    wait_screen(lambda: screen.display[-1].rstrip().endswith("> /next"), "Up did not recall the newest message")
    os.write(master, b"\x1b[A")
    wait_screen(lambda: screen.display[-1].rstrip().endswith("> Two lines↵Second line"), "Up did not recall multiline message past a slash command")
    os.write(master, b"\x1b[A edited")
    wait_screen(lambda: screen.display[-1].rstrip().endswith("> First question edited"), "recalled message could not be edited")
    os.write(master, b"\x1b[B\x1b[A")
    wait_screen(lambda: screen.display[-1].rstrip().endswith("> First question edited"), "history browsing lost edits")
    os.write(master, b"\x1b[B\x1b[B\x1b[B")
    wait_screen(lambda: screen.display[-1].rstrip().endswith("> Unsent draft") and screen.cursor.x == draft_cursor,
                "Down did not restore unsent draft and cursor")
    os.write(master, b"\x15\x1b[A\x1b[A!\r")
    def resubmitted():
        return any(json.loads(line).get("text") == "Two lines\nSecond line!"
                   for line in (root / "requests.jsonl").read_text().splitlines())
    wait_screen(resubmitted, "edited multiline message was not resubmitted intact")
    os.write(master, b"/\x1b[B\t")
    wait_screen(lambda: screen.display[-1].rstrip().endswith("> /apply"), "Down in slash menu recalled history instead of selecting a command")
    os.write(master, b"\x15")
    os.write(master, b"\x1b")
    time.sleep(.15)
    os.write(master, b" t")
    wait_screen(lambda: locate("Return to tour stop") is not None, "Tandem key menu has no descriptions")
    (root / "key-menu.txt").write_text("\n".join(screen.display))
    for description in ["Compare saved change", "Return to tour stop", "Search tour stops",
                        "Edit message", "Send message", "Keep draft and return to chat"]:
        if not locate(description): errors.append(f"missing key description: {description}")
    os.write(master, b"\x1b")
    wait_screen(lambda: locate("Return to tour stop") is None and locate("Undocumented plugin command") is None,
                "key menu did not close")
    os.write(master, b" tj")
    wait_screen(lambda: locate("2 / 2 stops") is not None, "tour picker did not open")
    os.write(master, b"zzzzzz")
    wait_screen(lambda: locate("No matching stops") is not None, "picker did not show empty search")
    os.write(master, b"\x15rqhd")
    wait_screen(lambda: locate("1 / 2 stops") is not None, "picker did not fuzzy-match the stop")
    (root / "tour-picker.txt").write_text("\n".join(screen.display))
    borders=[(row,line.find("╭")) for row,line in enumerate(screen.display) if "╭" in line]
    if not borders: errors.append("picker has no border")
    if not locate("Enter jump"): errors.append("picker keyboard hint missing")
    os.write(master, b"\x1b")
    wait_screen(lambda: locate("1 / 2 stops") is None, "picker did not close on Escape")
    os.write(master, b":set-language rust\r")
    time.sleep(.15)
    os.write(master, b" tp")
    wait_screen(lambda: locate("Comparison unavailable") is not None,
                "successful response without comparison was silently ignored")
    os.write(master, b" tp")
    wait_screen(lambda: locate("Old · story.txt") is not None, "native comparison did not open")
    wait_screen(lambda: locate("Tandem ·") is None, "comparison did not hide chat")
    (root / "comparison.txt").write_text("\n".join(screen.display))
    if sum("source line 120" in line for line in screen.display) != 1:
        errors.append("comparison sides did not start at the same changed block")
    row = next((line for line in screen.display if "source line 120" in line), "")
    if row.count("source line 120") != 2:
        errors.append("old and current code were not both visible side by side")
    os.write(master, b"iEDITED \x1b")
    wait_screen(lambda: locate("EDITED source") is not None, "current comparison side is not editable")
    os.write(master, b"\x17h")
    time.sleep(.15)
    os.write(master, b":q!\r")
    time.sleep(.15)
    os.write(master, b" tp")
    wait_screen(lambda: locate("Tandem ·") is not None, "closing the baseline window broke comparison cleanup")
    wait_screen(lambda: locate("EDITED source") is not None, "closing comparison lost current edits")
    dirty = [json.loads(line).get("context", {}).get("dirty", [])
             for line in (root / "requests.jsonl").read_text().splitlines()]
    if any("[unsaved buffer]" in entry for entry in dirty):
        errors.append("baseline snapshot leaked into dirty editor context")
    if not any(str(source) in entry for entry in dirty):
        errors.append("editable comparison source missing from dirty editor context")
    os.write(master, b":q!\r")
    deadline=time.monotonic()+5
    while process.poll() is None and time.monotonic()<deadline:
        drain(); time.sleep(.05)
    drain()
    if process.poll() != 0 or b"panicked" in capture: errors.append(f"q! did not exit cleanly: {process.poll()}")
finally:
    if process.poll() is None:
        process.terminate()
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait(timeout=5)
    drain()
    (root / "terminal.bin").write_bytes(capture)
    os.close(master)
print(f"Fixture: {root}")
for error in errors: print("FAIL:",error)
if errors: sys.exit(1)
print("PASS: narration range, native range selection, Git coexistence, wrapping, movement, styled chat, message recall/resend, shortcut descriptions, editable native comparison/cleanup, and q! shutdown")
