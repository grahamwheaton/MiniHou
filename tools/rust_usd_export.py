"""Serialize already-cooked Rust geometry to OpenUSD (stdin/stdout JSON).

Never starts an engine or evaluates a graph. Optional dependencies match Python USD export.
"""
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


def decode_outputs(data_by_name):
    import numpy as np
    from minihou.geometry import Geometry, Topology

    outputs = {}
    for name, data in data_by_name.items():
        topology = Topology(np.asarray(data["vertex_point"], np.int32),
                            np.asarray(data["prim_offsets"], np.int32),
                            None if data["closed"] is None else np.asarray(data["closed"], bool))
        geo = Geometry(data["npoints"], topology)
        for domain in ("point", "vertex", "prim"):
            table = getattr(geo, domain)
            for key, attr in data[domain].items():
                values = np.asarray(attr["data"], dtype=attr["dtype"])
                if attr["width"] != 1:
                    values = values.reshape(-1, attr["width"])
                table[key] = values
        geo.detail.update(data["detail"])
        geo.validate()
        outputs[name] = geo
    return outputs


def export(payload, samples=None):
    from minihou.usd_io import write_stage, write_animation, pxr
    pxr()  # Fail before consuming geometry when the optional writer is unavailable.
    options = payload.get("options") or {}
    path = Path(payload["path"])
    fmt = options.get("format", "auto")
    if fmt not in ("auto", "usda", "usdc"):
        raise ValueError("format must be auto, usda or usdc")
    # The Rust adapter resolves the same extension rules as export_target before
    # enforcing workspace containment and overwrite. Never change that path here.
    path.parent.mkdir(parents=True, exist_ok=True)
    kwargs = dict(graph=payload.get("graph"), overwrite=payload.get("overwrite", False),
                  up_axis=options.get("up_axis", "z"), units=options.get("units", "meters"),
                  root=options.get("root") or "MiniHou", materials=options.get("materials", True))
    if samples is None:
        samples = iter([{"frame": payload.get("frame"), "outputs": payload["outputs"]}])
    if payload.get("animated"):
        frames = ((record["frame"], decode_outputs(record["outputs"])) for record in samples)
        report = write_animation(path, frames, fps=payload.get("fps", 24), **kwargs).to_dict()
    else:
        record = next(samples, None)
        if record is None:
            raise ValueError("Rust cook produced no export sample")
        report = write_stage(path, decode_outputs(record["outputs"]), frame=payload.get("frame"), **kwargs).to_dict()
    report["frame"] = payload.get("frame")
    report["message"] = "Wrote " + report["path"]
    return report


def main():
    try:
        payload = json.loads(sys.stdin.readline())
        samples = (json.loads(line) for line in sys.stdin if line.strip()) if payload.get("stream") else None
        result = export(payload, samples)
    except FileExistsError as exc:
        result = {"error": str(exc) + "; set overwrite to replace it", "http_status": 409}
    except (ImportError, RuntimeError) as exc:
        result = {"error": str(exc), "http_status": 501}
    except (ValueError, OSError) as exc:
        result = {"error": str(exc), "http_status": 400}
    print(json.dumps(result))


if __name__ == "__main__":
    main()
