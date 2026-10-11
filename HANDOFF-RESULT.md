# MiniHou combined integration result - 11 October 2026

Status: COMPLETE for the supported staged Geometry Nodes bridge and local integration.
Checkout: `C:/Users/Graham/Documents/MiniHou-geonodes`
Branch: `codex/geonodes-integration`

The Geometry input is really evaluated and sent to the external engine. This is not the old cache-wrapper geometry bypass. A normal native GN network now supports native operations before, between and after multiple independently configured MiniHou groups. Blender has no supported Python geometry execution callback, so evaluation is staged through disposable native input probes and private persistent result meshes. Interactive updates are eventual; explicit cooking provides the barrier before rendering.

## Start here on Bob

1. Install `dist/MiniHou-Link-integrated.zip` through Blender's Install from Disk and enable MiniHou Link. It was install/enabled in an isolated Blender 5.2 profile successfully; no existing user installation/preferences were modified.
2. Run `Start-MiniHou-Wired.cmd` in this checkout. It uses the prepared external runtime at `build/runtime/python312/python.exe`, port 8765, editor port 8766. Nothing is installed inside Blender.
3. Open `dist/MiniHou-Wired-Groups.blend`. It opens in the Geometry Nodes workspace with the actual native -> MiniHou A -> native -> MiniHou B -> native chain. The stored endpoint is 8765, not the temporary acceptance-test port.
4. Change the first native Scale, MiniHou A Scale, or MiniHou B Translate. Select a MiniHou group and use Geometry Nodes editor sidebar > MiniHou > Cook Wired Groups / Stack Now to force a deterministic refresh and inspect status.
5. For Edit Network, configure the addon's External Python and MiniHou Project Folder to this checkout, then Start Server to start/reuse the ready server. Select a MiniHou placement, Edit Network, save in the external editor, then Reload Group Graph. Editing changes the template, not a stored snapshot live. The editor session is shared.

Full usage and supported limits: `docs/GEONODES.md`. Recreate the example with `tools/blender_geonodes_demo.py`; it is additive to the current scene. Native RBD usage: `docs/RBD.md`; modeling usage: `docs/modeling_tools.md`.

## Delivered artifacts

All paths below are relative to this checkout.

- `dist/MiniHou-Link-integrated.zip`: latest combined installable addon (GN, stack modifiers, source objects, Edit Network and RBD bake).
- `dist/MiniHou-integrated-source.zip`: committed source, handoff, tools, tests, examples and the verified GN blend. The external runtime is not embedded in this source ZIP.
- `dist/MiniHou-integrated.bundle`: complete recoverable local Git history, including the integration branch and HEAD. No GitHub push/merge was performed.
- `dist/integration-artifacts.json`: exact packaged commit, branch, byte counts and SHA256 hashes. This is the authoritative final package revision rather than a self-referential hash inside this handoff.
- `dist/MiniHou-Wired-Groups.blend`: verified normal GN network; saved output also displays without addon/engine.
- `dist/rbd_acceptance/RBD_Brick_Wall_Baked.blend`: native offline bake, frames 1-96.
- `dist/rbd_acceptance/RBD_Falling_Pieces_Baked.blend`: 25-piece native offline bake, frames 1-96.
- `dist/rbd_acceptance/wall_live_0012.png`: actual projectile impact render, visually inspected.
- `dist/rbd_acceptance/offline_verified_0012.png`: matching offline render from reopened bake.

Recovery example:

```powershell
git clone -b codex/geonodes-integration .\dist\MiniHou-integrated.bundle C:\path\MiniHou-recovered
```

The prepared CPython 3.12.10/NumPy 2.2.6/MuJoCo 3.15.0 runtime was copied from the completed RBD checkout into this checkout. Its `_pth` now resolves this integration source, verified via `minihou.__file__`. Existing trusted Playwright/Chromium files were copied from the completed editor checkout into `build/runtime`; no untrusted binary or engine dependency was installed into Blender. To move the ready runtime to another machine, include `build/runtime/python312` separately or prepare an external runtime as documented.

## Verified results

- Full combined engine/protocol/browser suite: **667 tests, OK, 143 skipped**, 99.034 seconds. Actual Chromium editor tests ran, including delayed-edit recovery after graph switches. Evidence: `build/validation/engine-browser-final.log`. The first run's missing-browser skip was corrected by setting the trusted browser path; do not use `engine-browser.log` as browser acceptance evidence.
- Combined actual Blender 5.2.0 LTS suite: **115 tests, all passed**, 23.485 seconds. Includes existing addon, modifier, source-object, server/editor routing and modeling host tests plus **30 wired-GN tests**. Evidence: `build/validation/blender-full.log`. These are real evaluated Blender meshes and real external-engine TCP requests, not mocked geometry evaluation.
- GN acceptance covers independent A/B parameter changes, upstream native mesh changes, native mesh generation, topology changes, nesting, Group Input bindings, branches/Join, stacked mixed GN and legacy modifiers, duplicated placements/objects, fake-user groups, save/reopen, zero unchanged engine cooks, named attributes/materials, missing-engine recovery, resource cleanup, unsupported-input rejection, honest dependent Waiting states, and actual RBD inside independent wired placements.
- Engine-only GN tests: 4 real instance/TCP tests. Combined repeat/RBD tests: 3, including independent worlds/replay and rejecting RBD inside a repeat even at zero iterations. Existing atomic library reload concurrency test also passed.
- Registered Add Group, Cook, Reload Group and Edit Network operators were exercised in an actual Node Editor context. GN panel visibility was tested, not a full visual GUI layout audit. OS browser launch is mocked in the editor-routing tests; the separate browser suite uses actual Chromium.
- Wall RBD demo: **12 bricks displaced**, contacts, exact scrub replay, independent modifiers, materials, still renders and 96-frame native bake passed. Evidence: `build/validation/rbd-wall.log`, `dist/rbd_acceptance/blender_smoke.json`.
- Falling RBD demo: **25 pieces**, floor collisions, group preservation, exact scrub and 96-frame bake passed. Evidence: `build/validation/rbd-falling.log`, `dist/rbd_acceptance/falling_smoke.json`.
- Actual `animation_play`, in an owned hidden Blender process: reached **frame 30 in 2.294 seconds**, with changed evaluated geometry. Evidence: `dist/rbd_acceptance/timeline_playback.json`, `build/validation/rbd-playback.log`.
- Reopened RBD bake rendered frames 11-13 with no engine/addon. Frame 12 mean pixel error versus live render: **0.0**. Evidence: `dist/rbd_acceptance/offline_render_validation.json`, `build/validation/rbd-offline.log`.
- Saved GN example reopens engine-free: 8 vertices, 6 faces, z bounds approximately 0.6 to 3.4, named attribute/material retained, saved port 8765. Evidence: `build/validation/geonodes-offline.json`.
- Actual packaged addon install/enable/disable passed, using an isolated `build/package-smoke` profile. Engine package was not imported into Blender. Evidence: `build/validation/package-install.json`.
- Windows text portability and `git diff --check` passed. Addon ZIP/source ZIP CRCs and Git bundle were verified by `tools/package_integration.py`.

The ordinary-Python skips are host/optional dependency tests; Blender coverage is reported separately above. This integration was verified on Blender 5.2.0 LTS. Other agents' earlier 5.1 results are not claimed as testing this final combined build.

Reproduce combined validation with:

```powershell
.\build\runtime\python312\python.exe tools\validate_integration.py
```

It uses the local browser runtime when available and rejects skipped Chromium acceptance. Logs are under `build/validation`. Actual interactive playback is an additional run of `tools/blender_rbd_smoke.py -- --playback` without `--background`.

## Supported boundaries

- Staged main-thread probes, not a native synchronous Python GN callback. Cook before a still render/save when freshness matters. Animation/background rendering must explicitly cook each frame, or use the supported native RBD bake. No depsgraph callback writes geometry.
- Mesh input/output. Instances are realized at the boundary; remaining curves, point clouds, volumes, grease pencil and residual instances are rejected. External object instancing output is unsupported for wired groups.
- Named numeric POINT/CORNER/FACE attributes and material assignments survive supported engine operations. Anonymous attributes, native EDGE data, string/matrix data and internal dot-prefixed attributes are not promised. Exact storage types/UI metadata are not preserved: booleans become integer flags, UV/render colors use existing engine conventions. Skipped supported-reader attributes appear in status. See docs for detail.
- Placement parameters support literal values, Value constants, reroutes and nested Group Input value bindings. Arbitrary linked math/fields and modifier attribute mode are rejected rather than reading their defaults as evaluated values.
- Acyclic nesting and branches are supported. Shared ancestors containing placements become private to avoid cross-placement cache ownership. Muted containing groups, native simulation/repeat/foreach/bake zones in probed stacks, Self Object, linked library groups and cross-object MiniHou feedback dependencies are rejected.
- RBD requires static inputs. The RBD Solver can be before or after an external repeat block, but not inside its body or a reusable library system. Existing RBD convexity, integer-frame and no-deforming-collider limits remain; see `docs/RBD.md`.
- Missing engine/cook errors retain each last valid result and expose status; dependent computations wait. Before any success, wired groups have an empty output, never an input passthrough advertised as computation.
- Every refresh still evaluates native probes; hashes avoid unchanged external cooks/mesh writes, not Blender probe cost. Large networks may be expensive. Generated result objects are managed internal caches.

## Local integration record

Starting point: `053bbe6` (Bob repeat-loop commit, equivalent to home `0f2f3c7`; no duplicate cherry-pick).
Fetched `build/handoff-20261011/home-handoff.bundle` from original MiniHou using its explicit branch ref.

Preserved/imported:

| Source | Integrated local commit(s) |
| --- | --- |
| Home atomic library reload `660c9b6` | `5949adc` |
| Wired-input GN implementation | `bfed9e3` |
| Modeling `fed79fd`, `85c4885`, `30d29e4`, `a4f14d1`, `7b66f4e` | `b1a8e62`, `7e874bb`, `8e3c55b`, `a6f83c9`, `e57c38e` |
| Edit Network `e2ad652` | `9421e23` |
| GN editor routing/portability | `13ac43b` |
| RBD `10cb9b9`, `346648c`, `b6d1674`, `2a6b9e7` | `ece4825`, `e1b2fc7`, `2faa6bd`, `53c7524` |
| Combined validation, RBD/loop guard and packaging tools | `00eb59e` |

Later delivery-only commits finalize the saved endpoint, recovery bundle refs and this handoff; see `git log` and the artifact manifest. Registry conflicts retain modeling + loops + RBD. Evaluator conflicts retain the repeat cook budget/loop validation and independent RBD ownership/static-input checks. Atomic registry batches remain around library reload, alongside the RBD system guard. Addon registration retains all GN/editor/RBD lifecycle hooks.

Other agents' completion records were read and their worktrees verified clean before cherry-picking. Their checkouts were read only. No earlier equivalent source/group/server commits were duplicated. Protected `examples/graphs/demo.json` and `nodes/rock_scatter.json` have no diff from the starting commit. No ProjectNotes edits, GitHub pushes/merges, or home-user uncommitted edits were included.

## Chats and completion records

- This integration: `01a12cbc-a2b2-7103-bbf8-71f5bcbefa51`.
- Modeling: `01a12caa-b4aa-7513-8f87-28ffa7bbdf89`.
- RBD: `01a12ca6-0ab7-7b60-8ee7-49763f6e411e`.
- Edit Network/loops: `01a12c3c-87a8-7632-8bf3-fa0f3efee5fe`.
- Original handoff and finished agent records: `C:/Users/Graham/Documents/MiniHou/build/handoff-20261011/{HANDOFF.md,modeling-RESULT.md,rbd-RESULT.md,editor-RESULT.md}`.

No implementation task remains open in the authorized supported bridge scope. Graham's next step is to try the delivered normal GN example on production meshes and assess probe performance and the documented field/data limitations before adopting it for large scenes.
