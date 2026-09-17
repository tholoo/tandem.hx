#!/usr/bin/env python3
"""Opt-in live Codex smoke check; uses existing login and incurs normal model usage.
Creates only an isolated fixture repository. Needs Bubblewrap and a working Codex CLI.
Usage: scripts/check-codex.py [tandem]
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

binary = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/tandem").resolve())
root = Path(tempfile.mkdtemp(prefix="tandem-codex-check-"))
repo = root / "repo"
repo.mkdir()
subprocess.run(["git", "init", "-q", str(repo)], check=True)
(repo / "app.txt").write_text("hello\n")
subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
subprocess.run(["git", "-C", str(repo), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "-qm", "fixture"], check=True)
session = root / "session"
process = subprocess.Popen([binary, "start", str(repo), "--session", str(session)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
sock = session / "controller.sock"
conn = socket.socket(socket.AF_UNIX)
conn.settimeout(240)
serial = 0

try:
    for _ in range(100):
        if sock.exists():
            break
        if process.poll() is not None:
            raise RuntimeError(process.stderr.read().decode())
        time.sleep(0.1)
    conn.connect(str(sock))
    reader = conn.makefile("rb")

    def send(action, **kwargs):
        global serial
        serial += 1
        conn.sendall((json.dumps(dict(version=1, id=serial, action=action, **kwargs)) + "\n").encode())
        while True:
            response = json.loads(reader.readline())
            if response.get("event") == "error":
                raise RuntimeError(response["text"])
            if response.get("id") == serial:
                if response.get("error"):
                    raise RuntimeError(response["error"])
                return response["view"]

    def idle():
        while True:
            response = json.loads(reader.readline())
            if response.get("event") == "error":
                raise RuntimeError(response["text"])
            if response.get("event") == "state" and not response["view"]["busy"]:
                return response["view"]

    send("message", text="Give me a brief tour of this repository. Read app.txt first. Use one stop.")
    state = idle()
    assert state["stage"] == "discuss" and state["tour"]["source"]["kind"] == "repository"
    assert (repo / "app.txt").read_text() == "hello\n"
    assert (session / "worktree/app.txt").read_text() == "hello\n"
    print("PASS: live read-only repository tour", flush=True)
    tour_id = state["tour"]["id"]
    send("tour_next")
    send("message", text="Go back one step to the overview of this same tour.")
    state = idle()
    assert state["tour"]["id"] == tour_id and state["tour"]["current_stop"] == 0
    assert state["stage"] == "discuss"
    print("PASS: conversational navigation preserves the active tour and stage", flush=True)
    send("tour_close")
    send("plan", text="Change app.txt from hello to hi, keeping the trailing newline. Check the file with cat.")
    assert idle()["stage"] == "plan"
    send("begin")
    state = idle()
    assert state["stage"] == "tour"
    assert (repo / "app.txt").read_text() == "hello\n"
    assert (session / "worktree/app.txt").read_text() == "hi\n"
    send("apply")
    assert (repo / "app.txt").read_text() == "hi\n"
    send("revert", change=0)
    assert (repo / "app.txt").read_text() == "hello\n"
    print("PASS: live plan → shadow build → proposal tour → apply → revert", flush=True)
finally:
    conn.close()
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
    print(f"Private fixture retained: {root}")
