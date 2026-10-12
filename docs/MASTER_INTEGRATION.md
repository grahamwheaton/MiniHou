# Combined master - 12 October 2026

The integration combines Claude's main features with every local and remote Codex feature branch. Original histories are retained through merge commits; source branches and the other worktrees are preserved.

| Source branch | Integrated tip |
| --- | --- |
| ccr-17c16efa-8t2kw5 (Claude) | ca37e91 |
| codex/geonodes-integration | 9bf4bb0 |
| codex/updates | e2ad652 |
| codex/modeling-tools | 7b66f4e |
| codex/rbd | 2a6b9e7 |
| codex/home-updates-20261011 | 0f2f3c7 |

Includes resizable editor panes, the 3-D viewport and transform handles, selection tools, network boxes/copy-paste, named outputs, attribute flow, optional Python nodes, packed-app/add-on installation, repeat blocks, independent stack modifiers/source objects, safe Edit Network routing, wired Geometry Nodes, modeling tools and MuJoCo RBD.

## Node-name compatibility

Both branches implemented `skin` and `dissolve` with incompatible parameter and selection semantics. Both implementations are retained as separate registered nodes:

| Name | Behavior |
| --- | --- |
| `skin` | Claude's resampled loft; samples/loop/cap |
| `skin_profiles` | Codex's matching-corner loft; group/keep_profiles/close_loft |
| `dissolve` | Claude's planar primitive-region dissolve; group/angle |
| `dissolve_edges` | Codex's explicit edge dissolve; group/angle/preserve_seams |

All supplied Codex modeling graphs and their generator now use the explicit names. For custom graphs saved on the older Codex modeling branch, change node type `skin` to `skin_profiles` and `dissolve` to `dissolve_edges` before loading them in this combined version. Node IDs and connections need no change. For saved Blender modifier/group snapshots, update a copy of the template and reload the graph on the intended instance. Old unmarked graphs cannot be automatically disambiguated because empty parameters and `group`/`angle` are valid in both variants. Original worktrees remain available for those files.

The completed repeat-loop inspector compares against its initial geometry, avoiding standalone evaluation of the loop body. Named-output proxy streams and independent modifier instances coexist on the same server. The browser test now waits for asynchronous Blender discovery before asserting the result.

## Validation

- Combined engine/browser suite: 809 tests, OK, 157 skipped (host-only and unavailable optional accelerator tests). Actual Chromium tests ran.
- Additional cross-feature suite after the inspector fix: 5 tests, all passed, including loop inspection/named output and concurrent proxy/modifier clients.
- Actual Blender 5.2.0 LTS suite: 119 tests, all passed.
- Wall and falling-piece RBD cook/bake acceptance passed.
- Reopened offline bake rendered frames 11-13 without the engine; frame 12 mean pixel difference was 0.0.
- Wired Geometry Nodes example generation passed.

Local evidence: `build/validation/results.json`, `engine-browser.log`, `master-cross-feature.log`, `blender-full.log`, and the RBD logs. These are automated/headless acceptance results, not a fresh manual audit of production scenes.

Reproduce the combined acceptance with `python tools/validate_integration.py` using a prepared external runtime with NumPy, MuJoCo, Playwright and Chromium. See `docs/GEONODES.md`, `docs/RBD.md`, and `docs/modeling_tools.md` for supported behavior and limits. Physics remains optional and external to Blender. CI runs the physics backend in the Windows Python 3.12 engine job.

The merged add-on ZIP also installed/enabled/disabled successfully in an isolated Blender 5.2 profile (`build/validation/package-install.json`). The Windows PyInstaller build completed, but its local smoke launch was blocked with `WinError 5: Access is denied`; the standalone executable is not claimed as launch-validated.

Fresh GitHub runners exposed two validation setup assumptions: the Edit Network test now creates its temporary-file parent directory, and the Blender CI job installs MuJoCo in its external Python so wired-RBD acceptance runs rather than failing for a missing dependency. Nothing is installed into Blender's Python.

Windows CI also exercises 8.3 temporary paths such as `RUNNER~1`. Server reuse now canonicalizes both graph paths before comparison; routing tests compare canonical paths rather than short-versus-long spellings.

The Blender modifier client allows 30 seconds for an external cook (matching the shared client default). The prior five-second override timed out during a cold accelerated scatter cook on the Windows runner; failed cooks still retain their previous mesh and report an error.
