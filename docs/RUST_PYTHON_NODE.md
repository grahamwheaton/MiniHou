# The `python` node in the Rust app (opt-in subprocess bridge)

**This is not a sandbox.** An allowed python node runs the code stored in the graph as a normal Python process with your
user account's privileges: it can read and write your files and use the network. Enable it only for graphs you trust.

## Permission

* `Preferences > Security > Allow Python nodes` (persisted as `security.allow_python_nodes`, default **off**; only a literal
  `true` in the preferences file enables it). While off, every python node shows
  `Python execution is disabled; enable it in Preferences > Security` and nothing is started.
* While on, each distinct snippet (exact code text; the dialog shows a short FNV-1a id) asks **Run / Skip** once per app
  session. Skip keeps that snippet off until the code changes or the app restarts. Editing the code asks again.
* The engine takes the permission as an explicit input: `Session::set_python_gate(Some(gate))`. A `Session` or `Graph::cook`
  without a gate denies python nodes, so headless cooks, the Blender link's instance sessions and anything else that does
  not install a gate never run Python. Only the desktop UI (Preferences window and the Run / Skip dialog) holds the gate.
  The MCP HTTP API and the Blender link have no code path to it (`http_tests::remote_*` check that a request cannot
  enable it). Caveat: once *you* confirmed a snippet, a remote edit may change that node's numeric parameters and recook it;
  a remote edit that changes the code needs a new confirmation.

## Runtime

Discovery (first that runs `import numpy` wins, cached for the session): `MINIHOU_PYTHON`, the repo venv
(`.venv`/`venv`), `py -3` (Windows), `python3`, `python`. The repo root (found from `MINIHOU_REPO`, the executable, the
working directory or the build tree) is put on `PYTHONPATH` so the runner can `import minihou`; then it uses the Python
engine's own `python` node function, `Geometry`, `groups` and `timectx`, so results match the Python engine exactly. Without
`minihou` the runner falls back to a numpy-only stand-in with the same names in scope (`groups` is `None` there).

The runner is `tools/python_node_runner.py`, embedded in the core with `include_str!`. Each cook:

1. creates a fresh temp dir (also the working directory) and writes `python_node_runner.py` and `job.mhl` there;
2. starts `<python> [-3] -B python_node_runner.py <dir>` by argv (no shell), stdin closed, no console window, in its own
   process group, with the environment cleared to an allowlist (`PATH`, `SYSTEMROOT`, `TEMP`, `HOME`/`USERPROFILE`,
   `APPDATA`, locale, `VIRTUAL_ENV`/`CONDA_PREFIX`, `PYTHONHOME`) plus `PYTHONPATH`;
3. waits up to the timeout (`security.python_timeout_s`, default 30 s, 1..3600); on expiry the whole process tree is killed
   (`taskkill /T /F` on Windows, `kill -KILL -pgid` on Unix);
4. a non-zero exit shows the last 4000 characters of stderr (traceback, then the Python node's own one-line message with the
   snippet's line number) as the node error; stderr/stdout are capped at 64 KiB;
5. reads `out.mhl` if it is at most 2 GiB, validates it, and removes the temp dir.

## Wire format (version 1)

Both files use the live link's MHL1 framing (`docs`: `minihou/link/protocol.py`): `b"MHL1"`, u32 header length, u64 blob
length, a JSON header, then 8-byte aligned little-endian arrays described by `header.arrays`.

* `job.mhl`: `{"type":"python_job","version":1,"params":{code,a,b,c,d,i,j,v,s,flag,variables,animated},
  "time":{frame,fps,time_inc},"inputs":[null | {"npoints","detail"} x 4]}` with arrays `in<k>/point/<name>`,
  `in<k>/vertex/<name>`, `in<k>/prim/<name>` (`<f4`/`<i4`, shape `[n]` or `[n, width]`), `in<k>/topo/vertex_point`,
  `in<k>/topo/prim_offsets` and, when any primitive is open, `in<k>/topo/prim_closed` (`|u1`).
* `out.mhl`: `{"type":"python_result","version":1,"npoints","detail"}` with the same arrays without the prefix. A bool
  attribute (`|b1`) becomes int32 0/1 (the Rust core has no bool attributes; groups are int32 in both engines).

Every attribute value round-trips bit for bit (NaN UV markers included), as do corner UVs, groups, open curves and the whole
detail map (`materials`, `material_bindings` per `RUST_MATERIALS_SCHEMA.md`, anything JSON). Detail values that are numpy
arrays or scalars are converted to JSON lists and numbers; NaN in detail is refused.

## Caching

The node's cache key adds, to the usual type + parameters (which include the code) + input keys + time (when `animated`):
the gate's decision for this snippet, the timeout, the output limit and the runtime id (interpreter path, size and mtime,
runner source). Errors are never cached and nothing is retried automatically: a failed snippet runs again only on the next
cook that needs it (an edit, a frame change for animated nodes, or a permission change).

## Tests

`minihou-core` `python_bridge::tests`: pure permission rule and confirm-once, wire round trip, fake runners (success and
caching, non-zero exit, timeout with a grandchild that must not survive, malformed, oversized and missing output, scrubbed
environment and temp cwd), and the real runner on `DEFAULT_CODE` and a user snippet compared exactly with the Python
engine's own cook (skipped when no Python with numpy is found). `minihou-app`: preferences default/round trip/apply and the
remote-surface tests.
