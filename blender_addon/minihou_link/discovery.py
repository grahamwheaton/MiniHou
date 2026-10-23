"""Local engine discovery. Records are hints; every candidate needs a protocol handshake."""
import json
import os
from pathlib import Path
import time
import uuid


def directory():
    return Path(os.environ.get("MINIHOU_HOME", str(Path.home() / ".minihou"))) / "engines"


def publish(host, port, editor_url):
    if host not in {"127.0.0.1", "localhost", "::1"}:
        return None
    root = directory()
    root.mkdir(parents=True, exist_ok=True)
    path = root / (uuid.uuid4().hex + ".json")
    temp = path.with_suffix(".tmp")
    temp.write_text(json.dumps({"publisher": "minihou", "host": host, "port": port, "editor_url": editor_url,
                               "started": time.time()}), encoding="utf-8")
    temp.replace(path)
    return path


def candidates():
    records = []
    for path in directory().glob("*.json"):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            if data["host"] not in {"127.0.0.1", "localhost", "::1"}:
                continue
            port = int(data["port"])
            if 0 < port < 65536:
                records.append((float(data["started"]), data["host"], port))
        except (OSError, ValueError, KeyError, TypeError):
            continue
    return [(host, port) for _, host, port in sorted(records, reverse=True)[:16]]


RETRY_MIN = 2.0
RETRY_MAX = 30.0
CONNECT_TIMEOUT = 0.15  # per candidate; a live loopback engine accepts in milliseconds
CONNECT_BUDGET = 0.35   # total main-thread time one reconnect attempt may spend


def next_retry_delay(previous):
    """Backoff for main-thread reconnects: 2 s, 4 s, 8 s ... capped at 30 s."""
    if not previous or previous < RETRY_MIN:
        return RETRY_MIN
    return min(RETRY_MAX, previous * 2.0)


def rotate(choices, start):
    """Try a different candidate first each attempt so a budget-limited attempt still reaches all of them."""
    if not choices:
        return []
    start %= len(choices)
    return choices[start:] + choices[:start]


def forget_failed(host, port):
    """Remove our own discovery records after a connection attempt fails."""
    for path in directory().glob("*.json"):
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
            own_name = len(path.stem) == 32 and uuid.UUID(hex=path.stem).version == 4
            own_record = (isinstance(data, dict) and
                          (data.get("publisher") == "minihou" or
                           set(data) == {"host", "port", "editor_url", "started"}))
            if (own_name and own_record and
                    data.get("host") == host and int(data.get("port")) == port):
                path.unlink(missing_ok=True)
        except (OSError, ValueError, TypeError, AttributeError):
            continue
