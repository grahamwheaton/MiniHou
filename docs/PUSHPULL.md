# Inset, Outset and Push/Pull

Choose faces in the viewport, then use the face action menu, Polygon shelf or node menu. Inset and Outset provide an amount square you can drag; PolyExtrude also has an inset amount handle alongside its distance arrow. Push/Pull has a normal distance arrow. Positive distance adds solid; negative distance cuts into it and can cut through the opposite wall.

Load `examples/graphs/pushpull_hole.json` for a small through-hole example. Change hole distance from -3 to -0.4 for a pocket. Use a box face directly with positive distance for outward extension. The named `extrudeFront` group from Inset identifies the inner face.

Push/Pull currently accepts exactly one convex planar face on a closed outward-wound solid. Open sheets, multiple faces, concave or nonplanar selected faces produce actionable errors. The implementation uses the existing boolean kernel, with up to four deterministic solve frames and closed-edge, winding, volume and polygon-crossing checks. This is not a general proof against all possible mesh self-intersections. Numerically difficult solids may still be rejected. Input is limited to 20,000 faces / 50,000 points and polygons to 256 corners; use small modelling solids. It is not full Blender Extrude Manifold or SketchUp parity.

Inset/Outset use the existing radial PolyExtrude inset convention, not constant perpendicular edge offsets. They do not resolve overlap or collisions with neighbouring faces. Existing graphs and ordinary PolyExtrude remain compatible.

Tests check volume, edge twins, polygon crossings and Euler characteristic for a real through tunnel, all six rotated box faces, input preservation and rejection of invalid boolean results. Chromium tests exercise sketch creation in the full editor and the inset amount handle. No enormous realised-copy workload or driver-fix claim is involved.

## Punch through (Rust `through` parameter)

`through` is a Rust-only Push/Pull parameter: `auto` (default), `force` or `off`. `off` is the original Python behaviour above (one convex face, cutter ends at `distance`); the Python oracle tests run in `off`.

- `auto`: when a push reaches the opposite surface (every ray cast back from the face exits the solid, deepest exit within 1e-5 of the bounding size) or goes past it, the cutter stops just beyond the far surface, so the result is a clean through-hole: the pushed face and the coincident far face are removed and the walls join the two openings. Excess distance is ignored, like SketchUp. Faces between the shallow and deep exits of a slanted back face give an ordinary cut.
- A pull that lands on a same-facing parallel face plane snaps to it and dissolves the edges between coplanar faces touched by the new geometry (SketchUp's merge).
- `force` punches through whatever the distance, when an opposite surface exists.
- Several faces, ngons, concave faces and coplanar edge-connected faces (for example a face split around an existing hole) are pushed as regions: one closed prism per region. Open sheets have no opposite surface, so the node falls back to a plain per-face extrusion.
- New walls keep the pushed face's primitive attributes (material slot). Their corner UVs are invalid (NaN) as faces without a source, per docs/RUST_MATERIALS_SCHEMA.md.
- Collinear points the boolean leaves along cut edges are removed in `auto`/`force`. A solve that stays closed but has an inconsistent volume is rejected and retried in another frame.

In the viewport, Alt+E Push/Pull casts rays from the picked face at the start of the gesture. Dragging within 8 px of the far face snaps to it and shows **Through**. Dragging deeper keeps the value free and still shows Through. A parallel face plane snaps with **On Face**. Typed numbers are never moved. The preview cooks through the same node, Enter or a click confirms it as one undo step, and Esc or RMB restores the graph exactly.

Limits: concave regions where the boolean kernel hits coplanar overlaps can still be rejected with an error. Snap targets are computed against the whole displayed scene, while the node only sees its input object.
