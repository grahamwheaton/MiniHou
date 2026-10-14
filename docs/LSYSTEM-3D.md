# Spatial L-systems

The L-System node produces spatial curve networks. The earlier turtle already understood pitch and roll, but its default rule uses only yaw and therefore makes a flat tree. Existing graphs retain exactly that behavior; adding a third dimension requires a grammar that turns out of the plane.

Enable **Separate Angles** to expose independent **Yaw**, **Pitch** and **Roll** values (degrees). These registered node parameters appear in the normal parameter editor and can be exposed like other parameters. With Separate Angles off, the existing **Angle** controls all three rotations. Initial Orientation is always active: its three components are turtle yaw, pitch and roll, applied in that order before drawing. They are not world XYZ Euler rotation.

| Symbol | Operation |
| --- | --- |
| `F`, `f` | Advance along heading, drawing / without drawing |
| `+`, `-` | Yaw left / right |
| `&`, `^` | Pitch down / up |
| `\`, `/` | Roll left / right around the heading; changes the plane of subsequent turns |
| `\|` | Turn heading and left vectors around |
| `[`, `]` | Save / restore position, full orientation, step and width |
| `!`, `"` | Multiply width / step by their scale parameters |
| `J` | Emit a leaf point with heading in `N` |

The starting frame is heading +Z, left -X and up -Y. Turns use the moving local frame, not fixed world axes. A positive 90-degree yaw sends the initial heading toward -X; positive pitch sends it toward -Y. Roll does not move the turtle. Branches restore all three frame vectors, including roll. Unbalanced brackets report an error.

Open `examples/graphs/lsystem_3d_tree.json` for a radial tree (137.5-degree roll), or `examples/graphs/lsystem_3d_coral.json` for differently oriented branching. Both examples produce deterministic genuinely non-planar curves; connect a Sweep node to obtain tubes using their width attribute. Change Generations cautiously because rewriting grows exponentially.

Compatibility: existing planar grammars and Angle values are unchanged. This is a single-symbol rewriting turtle, not full Houdini L-System parity: no parameterized symbols, context-sensitive rules, tropism, collision avoidance or interactive turtle handles. Independent angles are global per node, not per symbol occurrence. The examples are curve skeletons rather than botanical simulations.

## Why a tree can still look flat

The legacy implicit grammar uses only `+` and `-`, so it is deliberately planar even though the turtle supports three dimensions. Increasing Pitch or Roll does nothing unless the grammar contains pitch (`&`/`^`) or roll (`\`/`/`) symbols. To start spatially, open `examples/lsystem_3d.json` or the existing `examples/graphs/lsystem_3d_tree.json`. Its rule `A=F[&A]/[&A]/[&A]A` pitches branches and rolls subsequent branch directions by 137.5 degrees. Orbit the view to inspect depth; an axis-aligned view can hide it.

The spatial starter uses Premise A, four Generations, Separate Angles enabled, Yaw 33, Pitch 42, Roll 137.5, Step 0.3 and Width 0.03. These are explicit creation/example settings, not changes to old saved graphs that omit parameters. The engine tests verify rank-three positions, not merely a nonzero coordinate on one axis. Sweep converts the skeleton curves into tube geometry.
