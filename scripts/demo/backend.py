#!/usr/bin/env python3
"""Scripted JSONL transport for recording the real editor UI without a model."""

import json
import sys
import time
from pathlib import Path

ROOT = Path.cwd()
SHADOW = ROOT / ".preview"
OLD = (ROOT / "src/retry.rs").read_text()
NEW = OLD.replace(
    "    let millis = 100 * 2_u64.pow(attempt);",
    "    let exponent = attempt.min(9);\n"
    "    let millis = (100 * 2_u64.pow(exponent)).min(30_000);",
)
VIEW = dict(
    stage="discuss",
    busy=False,
    real=str(ROOT),
    shadow=str(SHADOW),
    proposal=None,
    proposals=[],
    tour=None,
    changes=[],
    change_index=0,
    editor={},
    navigation=None,
    generation=0,
)


def emit(**value):
    print(json.dumps(value), flush=True)


def state():
    emit(event="state", view=VIEW)


def stop(title, body, file, line, end_line):
    return dict(title=title, body=body, file=file, line=line, end_line=end_line)


def navigate(index):
    tour = VIEW["tour"]
    tour["current_stop"] = max(0, min(index, len(tour["stops"])))
    root = SHADOW if VIEW["proposal"] else ROOT
    if tour["current_stop"]:
        location = tour["stops"][tour["current_stop"] - 1]
        VIEW["navigation"] = dict(
            file=str(root / location["file"]), line=location["line"]
        )
    VIEW["generation"] += 1
    state()


emit(event="connected", project=str(ROOT))
state()
for line in sys.stdin:
    request = json.loads(line)
    action = request["action"]
    if action == "context":
        VIEW["editor"] = request["context"]
    elif action in {"tour_next", "tour_previous", "tour_jump"}:
        index = request.get(
            "index", VIEW["tour"]["current_stop"] + (1 if action == "tour_next" else -1)
        )
        navigate(index)
    elif action == "peek":
        emit(
            id=request["id"],
            comparison=dict(
                file="src/retry.rs",
                current_file=str(SHADOW / "src/retry.rs"),
                old=OLD,
                current=NEW,
                old_line=4,
                current_line=4,
            ),
        )
    elif action == "input":
        text = request["text"]
        emit(event="message", text="You: " + text)
        VIEW["busy"] = True
        state()
        emit(event="activity", text="Reading the retry flow")
        time.sleep(0.8)
        if text.startswith("/begin"):
            (SHADOW / "src/retry.rs").write_text(NEW)
            VIEW.update(
                stage="tour",
                proposal=1,
                proposals=[1],
                changes=[
                    dict(id=0, file="src/retry.rs", line=4, label="Cap retry delays")
                ],
            )
            VIEW["tour"] = dict(
                id=2,
                source=dict(kind="proposal", proposal=1),
                current_stop=1,
                title="Bounded retry delays",
                overview="Cap the delay while preserving the initial backoff.",
                stops=[
                    stop(
                        "Put a ceiling on the wait",
                        "Clamp the exponent before computing the delay, then cap the result at **30 seconds**. Early retries still double as before.",
                        "src/retry.rs",
                        4,
                        8,
                    )
                ],
            )
            reply = (
                "The preview caps retry delays at 30 seconds. Let's look at the change."
            )
        elif "tour" in text.lower() or "walk" in text.lower():
            VIEW["tour"] = dict(
                id=1,
                source=dict(kind="repository"),
                current_stop=1,
                title="Follow a retry",
                overview="From the call site to the backoff calculation.",
                stops=[
                    stop(
                        "Start with the caller",
                        "Each attempt asks `retry_delay` how long to wait. Follow that call to see how the delay grows.",
                        "src/main.rs",
                        8,
                        10,
                    ),
                    stop(
                        "The wait doubles each time",
                        "The delay starts at **100 ms** and doubles with every attempt. There is no upper limit yet, which matters if retries keep failing.",
                        "src/retry.rs",
                        3,
                        7,
                    ),
                ],
            )
            reply = "Let's follow one retry from the caller to the delay calculation."
        else:
            reply = "Clamping the exponent keeps the calculation bounded too. Capping only the final delay would still allow an oversized power to be calculated first."
        VIEW["busy"] = False
        emit(event="message", text="Assistant: " + reply)
        if (
            text.startswith("/begin")
            or "tour" in text.lower()
            or "walk" in text.lower()
        ):
            navigate(VIEW["tour"]["current_stop"])
        else:
            state()
    elif action == "stop":
        break
