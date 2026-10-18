# Queued: Cooking, caching and packed instances (Houdini-style engine)

Source: the user's brief (2026-10-18). Not started until the audit (Phase 1) is reviewed.
Summary: incremental dependency-aware cooking, shared immutable geometry with copy-on-write, Pack/Unpack nodes, native instances for Copy to Points, an explicit Cache node, per-node cooking diagnostics, and reproducible benchmarks.
Stages: 1 audit; 2 shared immutable geometry and correct dependency tracking; 3 incremental cooking; 4 Pack/Unpack and native instances; 5 explicit caching and diagnostics; 6 benchmark and optimise.
Constraints: do not rewrite the node engine without a compelling reason; keep project files and Blender interop working; keep the node registration architecture; CPU parallelism only without unsafe concurrent mutation.

## Status 2026-10-18
- Stages 1-3 (audit, shared immutable geometry, incremental cooking): merged to native-ui.
- Stage 4 (Pack/Unpack, native instances): in progress on opus/bake-s4.
- Stage 5 (Cache node, diagnostics): next, local Codex after stage 4 lands.
- Stage 6 (benchmarks): after stage 5.
- Known gaps from stage 3: cancellation only between nodes; Graph::cook still copies on bypass.
