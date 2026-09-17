#!/usr/bin/env python3
"""Exercise real Steel APIs and IPC in a PTY, without injecting any editor keys.
Usage: scripts/check-helix.py /path/to/steel/hx /path/to/helix/runtime [tandem]
All repositories, editor configuration, and logs are isolated temporary fixtures.
"""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import termios
import time

hx, runtime = map(lambda p: str(Path(p).resolve()), sys.argv[1:3])
binary = str(Path(sys.argv[3] if len(sys.argv) > 3 else "target/debug/tandem").resolve())
plugin = Path(__file__).resolve().parents[1] / "helix/tandem/tandem.scm"
root = Path(tempfile.mkdtemp(prefix="tandem-helix-check-"))
repo = root / "repo"
repo.mkdir()
subprocess.run(["git", "init", "-q", str(repo)], check=True)
(repo / "app.txt").write_text("entry point\nrequest handler\ndatabase\n")
subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
subprocess.run(["git", "-C", str(repo), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"], check=True)
session = root / "session"
sock = session / "controller.sock"
controller = subprocess.Popen([binary, "start", str(repo), "--mock", "--session", str(session)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
editor = None
master = None
capture = bytearray()

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

def wait_for(predicate, name):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        drain()
        if b"error[E" in capture:
            raise RuntimeError("Steel error; inspect terminal.bin in the retained fixture")
        if sock.exists():
            view = request("status")
            if predicate(view):
                return view
        if controller.poll() is not None:
            raise RuntimeError(controller.stderr.read().decode())
        if editor is not None and editor.poll() is not None:
            raise RuntimeError(f"editor exited: {editor.returncode}")
        time.sleep(0.1)
    raise RuntimeError(f"timed out: {name}")

def wait_render(text):
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        drain()
        if text.encode() in capture:
            return
        if b"error[E" in capture:
            raise RuntimeError("Steel render error")
        time.sleep(0.05)
    raise RuntimeError(f"narration was not rendered: {text}")

try:
    wait_for(lambda v: True, "controller start")
    config = root / "config"
    config.mkdir()
    shutil.copy(plugin, config / "tandem.scm")
    (config / "helix.scm").write_text("")
    (config / "init.scm").write_text(f'''(require "tandem.scm")
(require "helix/ext.scm")
(require (prefix-in test. "helix/commands.scm"))
(require-builtin helix/core/static as test-editor.)
(require-builtin steel/time)
(require-builtin steel/filesystem)
(tandem-connect)
;; This test driver invokes native editor APIs, never terminal key sequences.
(spawn-native-thread
  (lambda ()
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
          (begin (time/sleep-ms 50) (wait-save))))))
''')
    (config / "config.toml").write_text('[editor]\ntrue-color = true\n')
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 35, 100, 0, 0))
    env = dict(os.environ, HELIX_STEEL_CONFIG=str(config), HELIX_RUNTIME=runtime,
               STEEL_HOME=str(root / "steel"), TANDEM_SOCKET=str(sock), TERM="xterm-256color",
               PATH=str(Path(binary).parent) + ":" + os.environ["PATH"])
    editor = subprocess.Popen([hx, "-c", str(config / "config.toml"), "--log", str(root / "helix.log"), str(repo / "app.txt")], stdin=slave, stdout=slave, stderr=slave, env=env, cwd=repo, start_new_session=True)
    os.close(slave)
    wait_for(lambda v: v["editor"]["file"] == str(repo / "app.txt"), "automatic editor context")
    request("message", text="Give me a tour of this repo.")
    view = wait_for(lambda v: v["tour"] is not None, "repository tour")
    assert view["stage"] == "discuss" and view["proposal"] is None
    wait_render("Repository tour (offline demo)")
    request("tour_next")
    wait_for(lambda v: v["tour"]["current_stop"] == 1, "first tour stop")
    wait_render("Explore existing code")
    request("tour_next")
    wait_for(lambda v: v["tour"]["current_stop"] == 2, "revisited location")
    request("tour_close")
    request("plan", text="Add the demo file")
    wait_for(lambda v: not v["busy"], "plan completion")
    request("begin")
    wait_for(lambda v: v["stage"] == "tour", "proposal tour")
    assert not (repo / "tandem-example.txt").exists()
    request("tour_next")
    wait_for(lambda v: v["editor"]["file"] == str(session / "worktree/tandem-example.txt"), "native shadow navigation")
    wait_render("The proposed file")
    request("apply")
    wait_for(lambda v: v["editor"]["file"] == str(repo / "tandem-example.txt"), "native real-worktree navigation")
    assert (repo / "tandem-example.txt").exists()
    request("revise", feedback="Add a revision to the demonstration")
    wait_for(lambda v: not v["busy"], "revision plan")
    request("begin")
    wait_for(lambda v: v["stage"] == "tour" and v["proposal"] == 2, "revised proposal")
    request("tour_next")
    wait_for(lambda v: "Offline revision 2" in v["editor"]["nearby"], "refresh existing shadow buffer")
    request("apply")
    wait_for(lambda v: v["editor"]["file"] == str(repo / "tandem-example.txt") and "Offline revision 2" in v["editor"]["nearby"], "refresh existing real buffer")
    request("switch", proposal=1)
    request("tour_next")
    wait_for(lambda v: v["editor"]["file"] == str(session / "worktree/tandem-example.txt") and "Offline revision 2" not in v["editor"]["nearby"], "restore previous preview")
    request("apply")
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
    assert b"Explore existing code" in capture, "repository narration was not rendered"
    assert b"The proposed file" in capture, "proposal narration was not rendered"
    print(f"PASS: native editor context, repository/proposal tour rendering, navigation, revisions, history, ordinary editing, apply, revert. Logs: {root}")
finally:
    if editor is not None:
        editor.terminate()
        try:
            editor.wait(timeout=5)
        except subprocess.TimeoutExpired:
            editor.kill()
    drain()
    (root / "terminal.bin").write_bytes(capture)
    controller.terminate()
    controller.wait(timeout=5)
    if master is not None:
        os.close(master)
    print(f"Fixture retained for inspection: {root}")
