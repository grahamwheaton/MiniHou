"""Write the parameter metadata (kind, range, options, doc, default) of the node types the Rust core has, from the Python node
registry, to ``rust/crates/minihou-app/src/param_meta.json``. The app reads it to draw sliders and drop-downs. The registry is
the source of truth; ``tests/test_rust_param_meta.py`` fails when the file is out of date, so run this script after changing a
parameter of one of these nodes::

    py tools/dump_rust_param_meta.py
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

TYPES = ["grid", "box", "point", "curve", "line", "circle", "sphere", "tube", "torus", "transform", "normal", "noise", "merge", "null", "output"]
TARGET = os.path.join(ROOT, "rust", "crates", "minihou-app", "src", "param_meta.json")


def meta() -> dict:
    import minihou  # noqa: F401 - registers the node types
    from minihou.sdk import REGISTRY

    out = {}
    for t in TYPES:
        params = {}
        for p in REGISTRY.get(t).params:
            d = {"kind": p.kind}
            for a in ("min", "max", "soft_min", "soft_max", "step", "doc"):
                v = getattr(p, a, None)
                if v is not None:
                    d[a] = v
            d["default"] = p.to_json_value(p.default)
            opts = getattr(p, "options", None) or getattr(p, "choices", None)
            if opts:
                d["options"] = [o if isinstance(o, str) else getattr(o, "name", str(o)) for o in opts]
            params[p.name] = d
        out[t] = params
    return out


def main() -> int:
    with open(TARGET, "w", encoding="utf-8", newline="\n") as f:
        json.dump(meta(), f, indent=1, sort_keys=True)
        f.write("\n")
    print("wrote", TARGET)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
