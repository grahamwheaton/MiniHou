# Viewport tools

Build with `python tools/build_app.py --with-accel`. The default delivery is the ready-to-run `dist/MiniHou` folder; ZIPs require `--zip`. `--output` selects a distribution directory. The new app is smoke-tested before replacing the previous folder, which is preserved under build/app-previous-*.

Navigation preferences now default to Blender: middle mouse orbits, Shift-middle pans, Ctrl-middle dollies. Geometry-depth orbit is optional. Houdini / MiniHou retains the earlier Alt-left navigation. Right-mouse WASD free flight remains available in both modes.

Ctrl-R over the viewport starts Loop Cut. Hover a quad edge for a cut preview; wheel changes count. Click to start sliding, click again or Enter to commit. Right-click during slide commits centered cuts; Escape cancels without creating a node. Uses the existing PolySplit engine and inserts into the displayed stream. Multiple cuts are evenly spaced; only single cuts slide. Stops at non-quad faces; this is not arbitrary knife cutting or full Blender edge-slide parity.

Match Size exposes target translation and six target-box size handles with box guides. Enable Scale in the node to edit target dimensions. Uniform fitting still follows the node's uniform option: dragging a non-limiting dimension may not change the fitted result. Handles resize symmetrically, not Blender's opposite-face-anchored cage behavior.

Lattice Grid exposes XYZ displacement handles and cage guides. Connect an unmodified grid to Lattice rest and a second grid to deformed; selecting Lattice lets its handles edit a directly connected Lattice Grid. Other deformed-input node types should be edited at their source. Cages above 216 points omit handles for responsiveness; incompatible offsets reset when resolution changes. Curve anchors now have explicit orientation-aware axis handles in addition to tangent controls and Auto/Vector/Aligned/Mirrored/Free modes.

Selection: 1/2/3 choose points/edges/faces. Ctrl-click extends by shortest topological path (fewest edges, not geometric distance). Alt-click selects edge loops, Ctrl-Alt-click selects rings; Shift adds, Ctrl-Shift subtracts. Loops stop at non-four-valence vertices; rings traverse quads. Face selection derives the faces adjoining the chosen strip. These are not all of Blender's selection variants.

Holding Ctrl temporarily inverts snapping during transform and curve drags. PCG shelf icons now distinguish common filtering, transform, surface, attribute, merge and sampling operations.


## Segmented bevel

PolyBevel now has Divisions (1–64), Round/Flat shape, Convexity (0 flat to 1 round), Limit Overlap, and Exclude Edges. Existing one-division graphs retain their chamfer. For a rounded bevel start with Divisions 4 and Convexity 1. Output Groups includes Point Fillet Polygons/Edges, Edge Fillet Polygons, Offset Edges/Points and Merged Points; groups follow the segmented topology. Merged Points stays empty because this implementation limits offsets rather than merging collisions.

Round strips use tangent rational quadratic profiles. Corner junctions use shared-boundary triangle fans, not a rolling-ball patch solver. Houdini-specific custom profile ramps, slide-on-ring overrides, asymmetry tolerance, distance scaling modes, and global collision merging are not implemented; controls for unsupported algorithms are not shown. Terminating selected edges now create endpoint caps, including single box edges.

Loop-cut commits now retain the current camera, including right-click to center the cut.


F over the viewport frames the current component/object selection, or the displayed geometry if no components are selected. Wheel zoom uses the surface depth under the cursor when available, slowing near geometry; over empty background it falls back to distance-scaled zoom.
