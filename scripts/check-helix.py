#!/usr/bin/env python3
"""Exercise real Steel APIs and IPC in a PTY, without injecting any editor keys.
Usage: scripts/check-helix.py /path/to/steel/hx /path/to/helix/runtime [tandem]
All repositories, editor configuration, and logs are isolated temporary fixtures.
"""
import codecs
import pyte
from terminal_screen import Screen
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import shlex
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time

hx, runtime = map(lambda p: str(Path(p).resolve()), sys.argv[1:3])
binary = str(Path(sys.argv[3] if len(sys.argv) > 3 else "target/debug/tandem").resolve())
plugin = Path(binary).resolve().parent.parent / "share/tandem/helix/tandem/tandem.scm"
if not plugin.exists():
    plugin = Path(__file__).resolve().parents[1] / "helix/tandem/tandem.scm"
root = Path(tempfile.mkdtemp(prefix="tandem-helix-check-"))
repo = root / "repo"
repo.mkdir()
subprocess.run(["git", "init", "-q", str(repo)], check=True)
(repo / "app.txt").write_text("entry point\nrequest handler\ndatabase\n")
subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
subprocess.run(["git", "-C", str(repo), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"], check=True)
session = None
sock = None
runtime_dir = root / "run"
helper_dir = root / "bin"
helper_dir.mkdir()
(helper_dir / "tandem").write_text(
    "#!/bin/sh\nif [ \"$1\" = editor ]; then exec " + shlex.quote(binary) + " \"$@\" --mock; fi\nexec " + shlex.quote(binary) + " \"$@\"\n"
)
(helper_dir / "tandem").chmod(0o700)
editor = None
master = None
capture = bytearray()
screen = Screen(100, 35)
terminal_stream = pyte.Stream(screen)
decoder = codecs.getincrementaldecoder("utf8")("replace")

def request(action, **kwargs):
    with socket.socket(socket.AF_UNIX) as conn:
        conn.settimeout(5)
        conn.connect(str(sock))
        conn.sendall((json.dumps(dict(version=1, id=77, action=action, **kwargs)) + "\n").encode())
        reader = conn.makefile("rb")
        while True:
            line = reader.readline()
            if not line:
                raise RuntimeError("controller disconnected")
            reply = json.loads(line)
            if reply.get("id") == 77:
                if reply.get("error"):
                    raise RuntimeError(reply["error"])
                return reply["view"]

def drain():
    if master is not None:
        while select.select([master], [], [], 0)[0]:
            try:
                data = os.read(master, 65536)
            except OSError:
                break
            if not data:
                break
            capture.extend(data)
            terminal_stream.feed(decoder.decode(data))

def wait_for(predicate, name):
    global sock, session
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        drain()
        if b"error[E" in capture:
            raise RuntimeError("Steel error; inspect terminal.bin in the retained fixture")
        if sock is None:
            addresses = list(runtime_dir.glob("checkout-*/active.json"))
            if addresses:
                sock = Path(json.loads(addresses[0].read_text())["socket"])
                session = sock.parent
        if sock is not None and sock.exists():
            view = request("status")
            if predicate(view):
                return view
        if editor is not None and editor.poll() is not None:
            raise RuntimeError(f"editor exited: {editor.returncode}")
        time.sleep(0.1)
    raise RuntimeError(f"timed out: {name}")

def wait_render(text):
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        drain()
        if text.encode() in capture or text in "\n".join(screen.display):
            return
        if b"error[E" in capture:
            raise RuntimeError("Steel render error")
        time.sleep(0.05)
    raise RuntimeError(f"narration was not rendered: {text}")

try:
    config = root / "config"
    config.mkdir()
    shutil.copytree(plugin.parent, config / "tandem", copy_function=shutil.copyfile)
    # Exercise the component's own input/submit functions, never Helix keystrokes.
    with (config / "tandem/chat.scm").open("a") as stream:
        stream.write("\n(provide insert submit focused pulse ticking session-view history draft caret completions complete-command)\n")
    with (config / "tandem/overlay.scm").open("a") as stream:
        stream.write("\n(provide query accept)\n")
    with (config / "tandem/comparison.scm").open("a") as stream:
        stream.write("\n(provide views state)\n")
    (config / "helix.scm").write_text("")
    (config / "init.scm").write_text(f'''(require "tandem/tandem.scm")
(require "helix/ext.scm")
(require "helix/misc.scm")
(require "helix/keymaps.scm")
(require (prefix-in chat. "tandem/chat.scm"))
(require (prefix-in overlay. "tandem/overlay.scm"))
(require (prefix-in compare. "tandem/comparison.scm"))
(require-builtin helix/core/editor as core.)
(require (prefix-in compose. "tandem/composer.scm"))
(require "helix/editor.scm")
(require (prefix-in test. "helix/commands.scm"))
(require-builtin helix/core/static as test-editor.)
(require-builtin steel/time)
(require-builtin steel/filesystem)
(enqueue-thread-local-callback tandem)
;; This test driver invokes native editor APIs, never terminal key sequences.
(spawn-native-thread
  (lambda ()
    (let wait-ui ()
      (if (path-exists? {json.dumps(str(root / "ui-now"))})
          (hx.block-on-task (lambda ()
            (define saved (unbox chat.session-view))
            (chat.chat-receive (hash 'view (hash-insert saved 'busy #t)))
            (chat.chat-receive (hash 'event "activity" 'text "Inspecting repository"))
            (chat.insert "Keep this draft while working")
            (chat.submit)
            (assert! (equal? (unbox chat.draft) "Keep this draft while working"))
            (assert! (= (unbox chat.caret) (string-length (unbox chat.draft))))
            (set-box! chat.draft "")
            (set-box! chat.caret 0)
            (enqueue-thread-local-callback-with-delay 650 (lambda ()
              (assert! (> (unbox chat.pulse) 0))
              (assert! (not (member "Inspecting repository" (unbox chat.history))))
              (chat.chat-receive (hash 'view saved))
              (assert! (unbox chat.focused))
              (enqueue-thread-local-callback-with-delay 250 (lambda ()
                (assert! (not (unbox chat.ticking)))
                (set-status! "NATIVE-UI-CHECK-PASS")))))))
          (begin (time/sleep-ms 50) (wait-ui))))
    (let wait-completion ()
      (if (path-exists? {json.dumps(str(root / "completion-now"))})
          (hx.block-on-task (lambda () (chat.chat-draft! "/")))
          (begin (time/sleep-ms 50) (wait-completion))))
    (let wait-completion-check ()
      (if (path-exists? {json.dumps(str(root / "completion-check-now"))})
          (hx.block-on-task (lambda ()
            (chat.chat-draft! "/ap") (chat.complete-command)
            (assert! (equal? (chat.chat-draft) "/apply"))
            (chat.submit)
            (assert! (equal? (chat.chat-draft) "/apply"))
            (chat.chat-draft! "")
            (set-status! "COMPLETION-CHECK-PASS")))
          (begin (time/sleep-ms 50) (wait-completion-check))))
    (let wait-chat ()
      (if (path-exists? {json.dumps(str(root / "chat-now"))})
          (hx.block-on-task (lambda ()
            (test.goto-line 2)
            (test-editor.set-current-selection-object! (test-editor.range->selection (test-editor.range 31 12)))
            (chat.chat-draft! "Give me a tour of this repo.")
            (tandem-compose)
            (test-editor.insert_string "\\nUse a short story.")
            (test-editor.normal_mode)))
          (begin (time/sleep-ms 50) (wait-chat))))
    (let wait-composer-close ()
      (if (path-exists? {json.dumps(str(root / "composer-close-now"))})
          (hx.block-on-task (lambda ()
            (test.buffer-close!)
            (assert! (not (compose.composer-active?)))
            (assert! (string-contains? (chat.chat-draft) "Use a short story."))
            (tandem-compose)
            (tandem-compose-close)
            (assert! (string-contains? (chat.chat-draft) "Use a short story."))
            (assert! (= (test-editor.get-current-line-number) 1))
            (tandem-compose)
            (tandem-send)))
          (begin (time/sleep-ms 50) (wait-composer-close))))
    (let wait-next ()
      (if (path-exists? {json.dumps(str(root / "next-now"))})
          (hx.block-on-task (lambda ()
            (assert! (equal? (query-global-keymap "normal" '("[" "t")) "tandem-previous"))
            (assert! (equal? (query-global-keymap "normal" '("]" "t")) "tandem-next"))
            (assert! (equal? (query-global-keymap "normal" '("space" "t" "p")) "tandem-peek"))
            ;; A custom command sequence remains a sequence, not a Tandem command.
            (assert! (not (query-global-keymap "normal" '("space" "t" "e"))))
            (tandem-next)))
          (begin (time/sleep-ms 50) (wait-next))))
    (let wait-previous ()
      (if (path-exists? {json.dumps(str(root / "previous-now"))})
          (hx.block-on-task tandem-previous)
          (begin (time/sleep-ms 50) (wait-previous))))
    (let wait-picker ()
      (if (path-exists? {json.dumps(str(root / "picker-now"))})
          (hx.block-on-task (lambda () (tandem-stops) (set-box! overlay.query "Return")))
          (begin (time/sleep-ms 50) (wait-picker))))
    (let wait-picker-accept ()
      (if (path-exists? {json.dumps(str(root / "picker-accept-now"))})
          (hx.block-on-task overlay.accept)
          (begin (time/sleep-ms 50) (wait-picker-accept))))
    (let wait-return ()
      (if (path-exists? {json.dumps(str(root / "return-now"))})
          (hx.block-on-task (lambda () (test.goto-line 3) (tandem-return) (set-status! "RETURN-REQUESTED")))
          (begin (time/sleep-ms 50) (wait-return))))
    (let wait-preview-edit ()
      (if (path-exists? {json.dumps(str(root / "preview-edit-now"))})
          (hx.block-on-task (lambda ()
            (test-editor.insert_mode)
            (test-editor.insert_string "human preview edit\\n")
            (test-editor.normal_mode)))
          (begin (time/sleep-ms 50) (wait-preview-edit))))
    (let wait-preview-save ()
      (if (path-exists? {json.dumps(str(root / "preview-save-now"))})
          (hx.block-on-task test.write)
          (begin (time/sleep-ms 50) (wait-preview-save))))
    (let wait-peek ()
      (if (path-exists? {json.dumps(str(root / "peek-now"))})
          (hx.block-on-task (lambda ()
            (core.editor-switch-action! (editor->doc-id (editor-focus)) (Action/VerticalSplit))
            (test.goto-line 2)
            (tandem-peek)))
          (begin (time/sleep-ms 50) (wait-peek))))
    (let wait-peek-close ()
      (if (path-exists? {json.dumps(str(root / "peek-close-now"))})
          (hx.block-on-task (lambda ()
            (assert! (compare.comparison-active?))
            (assert! (= (length (compare.views)) 3))
            (assert! (= (chat.chat-width) 0))
            (assert! (= (length (filter editor-document-dirty? (editor-all-documents))) 0))
            (define original (hash-ref (unbox compare.state) 'origin))
            (define left (hash-ref (unbox compare.state) 'left))
            ;; Toggle works from the baseline pane, too.
            (editor-set-focus! left)
            (tandem-peek)
            (assert! (not (compare.comparison-active?)))
            (assert! (= (length (compare.views)) 2))
            (assert! (equal? left (editor-focus)))
            (assert! (equal? (hash-ref original 'doc) (editor->doc-id (editor-focus))))
            (assert! (= (+ 1 (test-editor.get-current-line-number)) 2))
            (assert! (car (chat.chat-state)))
            (set-status! "PEEK-CLOSED")))
          (begin (time/sleep-ms 50) (wait-peek-close))))
    (let wait-edit ()
      (if (path-exists? {json.dumps(str(root / "edit-now"))})
          (hx.block-on-task (lambda ()
            (test.open {json.dumps(str(repo / "app.txt"))})
            (test-editor.insert_mode)
            (test-editor.insert_string "human edit\\n")
            (test-editor.normal_mode)
            (tandem-next)))
          (begin (time/sleep-ms 50) (wait-edit))))
    (let wait-save ()
      (if (path-exists? {json.dumps(str(root / "save-now"))})
          (hx.block-on-task (lambda ()
            (test.open {json.dumps(str(repo / "app.txt"))})
            (test.write)))
          (begin (time/sleep-ms 50) (wait-save))))
    (let wait-stop ()
      (if (path-exists? {json.dumps(str(root / "stop-now"))})
          (hx.block-on-task (lambda ()
            (tandem-stop)
            (enqueue-thread-local-callback-with-delay 1000 (lambda ()
              (assert! (equal? (query-global-keymap "normal" '("[" "t")) "goto_prev_class"))
              (assert! (equal? (query-global-keymap "normal" '("]" "t")) "goto_next_class"))
              (set-status! "NAVIGATION-KEYS-RESTORED")))))
          (begin (time/sleep-ms 50) (wait-stop))))))
''')
    (config / "config.toml").write_text('[editor]\ntrue-color = true\ninsecure = true\n[keys.normal.space.t]\ne = ["move_char_left", "move_char_right"]\n')
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 100, 0, 0))
    env = dict(os.environ, HELIX_STEEL_CONFIG=str(config), HELIX_RUNTIME=runtime,
               STEEL_HOME=str(root / "steel"), TANDEM_RUNTIME_DIR=str(runtime_dir), TERM="xterm-256color",
               PATH=str(helper_dir) + ":" + os.environ["PATH"])
    for key in ["TANDEM_SOCKET", "ZELLIJ", "ZELLIJ_SESSION_NAME", "ZELLIJ_PANE_ID"]:
        env.pop(key, None)
    editor = subprocess.Popen([hx, "-c", str(config / "config.toml"), "--log", str(root / "helix.log"), str(repo / "app.txt")], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=repo, start_new_session=True)
    os.close(slave)
    wait_for(lambda v: v["editor"]["file"] == str(repo / "app.txt"), "automatic editor context")
    wait_render("Tandem")
    (root / "ui-now").touch()
    wait_render("Working")
    wait_render("NATIVE-UI-CHECK-PASS")
    assert sum(glyph.encode() in capture for glyph in ["⠋", "⠙", "⠹", "⠸", "⠼"]) >= 2, "spinner did not animate on screen"
    (root / "completion-now").touch()
    wait_render("Tab complete")
    wait_render("no pending preview")
    (root / "completion-check-now").touch()
    wait_render("COMPLETION-CHECK-PASS")
    (root / "chat-now").touch()
    wait_render("Tandem message")
    pinned = request("status")["editor"]
    assert pinned["file"] == str(repo / "app.txt") and pinned["line"] == 2
    assert pinned["selection_start_line"] == 2 and pinned["selection_end_line"] == 3
    wait_render("app.txt:2–3 · selection")
    assert not pinned["dirty"], "message buffer polluted dirty code context"
    (root / "composer-close-now").touch()
    view = wait_for(lambda v: v["tour"] is not None, "repository tour")
    assert view["stage"] == "discuss" and view["proposal"] is None
    wait_render("Repository tour (offline demo)")
    (root / "next-now").touch()
    wait_for(lambda v: v["tour"]["current_stop"] == 1, "first tour stop")
    wait_render("Explore existing code")
    (root / "previous-now").touch()
    wait_for(lambda v: v["tour"]["current_stop"] == 0, "previous shortcut")
    request("tour_next")
    request("tour_next")
    wait_for(lambda v: v["tour"]["current_stop"] == 2, "revisited location")
    (root / "picker-now").touch()
    wait_render("> Return")
    (root / "picker-accept-now").touch()
    wait_for(lambda v: v["tour"]["current_stop"] == 2, "picker jump")
    (root / "return-now").touch()
    wait_render("RETURN-REQUESTED")
    wait_for(lambda v: v["editor"]["line"] == 1 and v["tour"]["current_stop"] == 2, "return to current stop")
    request("tour_close")
    restored = wait_for(lambda v: v["editor"]["line"] == 2, "restore location before repository tour")
    assert restored["editor"]["selection"] == pinned["selection"]
    request("input", text="Add the demo file")
    wait_for(lambda v: not v["busy"], "discussion completion")
    request("input", text="/begin")
    wait_for(lambda v: v["stage"] == "tour", "proposal tour")
    assert not (repo / "tandem-example.txt").exists()
    request("tour_next")
    wait_for(lambda v: v["editor"]["file"] == str(session / "worktree/tandem-example.txt"), "native shadow navigation")
    wait_render("The proposed file")
    (root / "preview-edit-now").touch()
    wait_for(lambda v: v["editor"]["dirty"], "unsaved preview edit")
    try:
        request("input", text="/apply")
        raise AssertionError("apply should require saved preview buffers")
    except RuntimeError as error:
        assert "save your Helix buffers" in str(error)
    (root / "preview-save-now").touch()
    wait_for(lambda v: not v["editor"]["dirty"], "saved preview edit")
    (root / "peek-now").touch()
    wait_render("Old (new file)")
    wait_render("human previe")
    assert not (repo / "tandem-example.txt").exists()
    (root / "peek-close-now").touch()
    wait_render("PEEK-CLOSED")
    request("input", text="Why is this file here?")
    unchanged = wait_for(lambda v: not v["busy"], "proposal question")
    assert unchanged["proposal"] == 1 and unchanged["tour"]["current_stop"] == 1
    request("input", text="Change this proposal by adding a revision")
    wait_for(lambda v: not v["busy"] and v["proposal"] == 2, "conversational proposal revision")
    assert not (repo / "tandem-example.txt").exists()
    request("tour_next")
    wait_for(lambda v: "Offline revision 2" in v["editor"]["nearby"], "refresh existing shadow buffer")
    assert (session / "worktree/tandem-example.txt").read_text().startswith("human preview edit")
    request("input", text="/apply")
    wait_for(lambda v: v["stage"] == "applied" and v["editor"]["file"] == str(repo / "tandem-example.txt") and "Offline revision 2" in v["editor"]["nearby"], "apply refined preview")
    assert (repo / "tandem-example.txt").read_text().startswith("human preview edit")
    request("switch", proposal=1)
    request("tour_next")
    wait_for(lambda v: v["editor"]["file"] == str(session / "worktree/tandem-example.txt") and "Offline revision 2" not in v["editor"]["nearby"], "restore previous preview")
    request("input", text="/apply")
    wait_for(lambda v: v["editor"]["file"] == str(repo / "tandem-example.txt") and "Offline revision 2" not in v["editor"]["nearby"], "restore previous real version")
    (root / "edit-now").touch()
    wait_for(lambda v: str(repo / "app.txt") in v["editor"]["dirty"], "unsaved editor context")
    try:
        request("revert", change=0)
        raise AssertionError("revert should require saving the dirty buffer")
    except RuntimeError as error:
        assert "save your Helix buffers" in str(error)
    (root / "save-now").touch()
    wait_for(lambda v: not v["editor"]["dirty"], "ordinary editor save")
    assert (repo / "app.txt").read_text().startswith("human edit")
    request("revert", change=0)
    assert not (repo / "tandem-example.txt").exists()
    assert (repo / "app.txt").read_text().startswith("human edit")
    drain()
    (root / "stop-now").touch()
    deadline = time.monotonic() + 10
    while sock.exists() and time.monotonic() < deadline:
        drain()
        time.sleep(0.05)
    assert not sock.exists(), "native stop did not shut down the controller"
    assert editor.poll() is None, "stopping Tandem should leave Helix open"
    wait_render("NAVIGATION-KEYS-RESTORED")
    print(f"PASS: native chat/completion, multiline draft retention, selection context/restoration, tour picker/return, native comparison/layout restoration, custom keys, refinement, apply/revert, and stop. Logs: {root}")
finally:
    if editor is not None:
        editor.terminate()
        try:
            editor.wait(timeout=5)
        except subprocess.TimeoutExpired:
            editor.kill()
    drain()
    (root / "terminal.bin").write_bytes(capture)
    if sock is not None and sock.exists():
        try:
            request("cancel")
            request("shutdown")
        except (OSError, RuntimeError):
            pass
    if master is not None:
        os.close(master)
    print(f"Fixture retained for inspection: {root}")
