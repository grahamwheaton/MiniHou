# Higgsas build queue (from docs/HIGGSAS_NODES.md)

Source: user request 2026-10-18, picked from the audit's "build natively" and library-node verdicts. Not started. Plan first with an advisor (Argon) before any build.

## Requested items

1. Planarize (audit: build natively, priority 2 - `planarize`)
2. Cluster (audit: build natively, priority 2 - `cluster`; check existing registry entry first)
3. Differential Growth (library node, priority 1)
4. Blend Curves (library node, priority 2)
5. Circle Packing
6. Dynamic Catenary Splines (library node, priority 2)
7. Hexagon Grid (library node, priority 2 - build natively as `Hexagon Grid` tile generator)
8. Tessellate Mesh (library node, priority 2; Smooth variant alongside)
9. Tubes to Splines
10. Bezier Extend
11. Align Curve to Surface
12. Curve Offset
13. Mix Splines
14. Splines Patch
15. Twist Curve (library node, priority 3)
16. Set Center
17. Connect Points
18. Edge Bisect (audit: have it - clip plane cut; confirm the Higgsas inputs are covered)
19. Edge Offset
20. Shortest Points Path (library node, priority 3)

## Requirements for every item

- Full input and output control: every optional Higgsas input and group output must be a MiniHou input or output, or a recorded decision not to expose it.
- Blender versions may lack optional group outputs; add them to the Blender add-on where needed (cheap).
- Check the audit row for each item before building; some may already exist under another name (for example Edge Bisect, Cluster, Twist).

## Open

- Coverage check against the node registry (`rust/crates/minihou-core/src/nodes/mod.rs`) has not been completed; grep hits so far are inconclusive.
- Argon plan not yet written.
