"""Explicit per-user engine preferences, never loaded from graph files."""
import json
import os
from pathlib import Path


def path():
    return Path(os.environ.get("MINIHOU_PREFERENCES_FILE") or
                Path(os.environ.get("LOCALAPPDATA", Path.home() / ".config")) / "MiniHou" / "preferences.json")


def read():
    try:
        data = json.loads(path().read_text(encoding="utf-8"))
        return {"python_nodes": data.get("python_nodes") is True}
    except (OSError, ValueError, AttributeError):
        return {"python_nodes": False}


def set_python(value):
    if not isinstance(value, bool):
        raise ValueError("python_nodes must be a boolean")
    target = path()
    target.parent.mkdir(parents=True, exist_ok=True)
    temporary = target.with_suffix(".tmp")
    temporary.write_text(json.dumps({"python_nodes": value}), encoding="utf-8")
    temporary.replace(target)
