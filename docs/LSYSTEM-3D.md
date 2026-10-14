# Spatial L-systems

The L-System node produces spatial curve networks. The earlier turtle already understood pitch and roll, but its default rule uses only yaw and therefore makes a flat tree. Existing graphs retain exactly that behavior; adding a third dimension requires a grammar that turns out of the plane.

Enable **Separate Angles** to expose independent **Yaw**, **Pitch** and **Roll** values (degrees). These registered node parameters appear in the normal parameter editor and can be exposed like other parameters. With Separate Angles off, the existing **Angle** controls all three rotations. Initial Orientation is always active: its three components are turtle yaw, pitch and roll, applied in that order before drawing. They are not world XYZ Euler rotation.

| Symbol | Operation |
| --- | --- |
| `F(l,w,s,d)`, `H` | Advance drawing along heading (`H` moves half length); optional override length, width, tube cols, tube rows |
| `f(l)`, `h` | Move along heading without drawing (`h` half length) |
| `G(l)` | Move along heading without recording a vertex |
| `+`, `-` | Yaw left / right in local frame (by angle, or `+(deg)`) |
| `&`, `^` | Pitch down / up in local frame (by angle, or `&(deg)`) |
| `\`, `/` | Roll left / right around heading (by angle, or `/(deg)`); changes the plane of subsequent turns |
| `\|` | Turn 180 degrees around heading |
| `*` | Roll 180 degrees |
| `~(a)` | Random pitch, yaw, roll up to `a` degrees (default 180) |
| `[`, `]` | Push / pop turtle branch (position, full frame, step, width, groups, attributes) |
| `%` | Truncate / cut off remainder of current branch |
| `!`, `?` | Multiply / divide thickness by scale factor (or `!(factor)`) |
| `"`, `_` | Multiply / divide step size by scale factor (or `"(factor)`) |
| `;`, `@` | Multiply / divide turning angle by scale factor (or `;(factor)`) |
| `T(g)` | Tropism towards -Z (gravity) |
| `$(x,y,z)` | Re-orient turtle frame towards up-ref, or point heading directly at (x,y,z) |
| `'`, `#` | Increment color lookup U / V coordinate |
| `g(i)` | Start primitive group `<prefix><i>` |
| `a("name",v1,v2,v3)` | Set custom point attribute |
| `{`, `.`, `}` | Polygon creation: mark vertices with `.` and construct closed polygon |
| `J`, `K`, `M(s,x,a,b,c)` | Stamp input geometry (Leaf J, K, M) scaled by `s` and stamped with parameters `a`, `b`, `c`. Without input, `J` emits a leaf point |

The starting frame is heading +Z, left -X and up -Y. Turns use the moving local frame, not fixed world axes. A positive 90-degree yaw sends the initial heading toward -X; positive pitch sends it toward -Y. Roll does not move the turtle. Branches restore all frame vectors, step size, width, and active attributes. Unbalanced brackets report an error.

Open `examples/graphs/lsystem_3d_tree.json` for a radial tree (137.5-degree roll), or `examples/graphs/lsystem_3d_coral.json` for differently oriented branching. Connect a Sweep node to sweep skeleton curves or set Type to Tube (`type="tube"`) for direct quad polygon tube meshes.

## Houdini Parity

MiniHou brings high-fidelity parity with Houdini's L-System SOP:
- **Rule Grammar**: `[left<]symbol[(vars)][>right][:condition]=replacement[:probability]`. Context matching traverses branches and respects `context_ignore` and `context_includes_siblings`. Probabilities support both 0-1 and 0-100 scales.
- **Parametric Expressions**: Variables `a`, `b`, `c`, `d`, `g`, `i`, `t`, `x`, `y`, `z`, `A`, `L`, `T`, `U`, `V`, `W`, plus custom user variables and `chan(i)`. Functions include `sin`, `cos`, `tan`, `asin`, `acos`, `min`, `max`, `abs`, `sqrt`, `floor`, `ceil`, `pow`, `exp`, `log`, `rand`, `pic`, and `inside`/`in(x,y,z)`.
- **4 Inputs**: `leaf_j`, `leaf_k`, `leaf_m` for instancing modeled leaf/flower geometry, and `meta_test` for bounding envelope containment pruning via `in(x,y,z)`.
- **Geometry Types**: `skeleton` (curves with optional point attributes `segs`, `div`, `lage`, `arc`, `up`, `gen`) and `tube` (quad polygon tubes with customizable `rows`, `cols`, and vertex `uv` texturing coordinates).
- **Fractional Generations**: Fractional `generations` interpolate growth of angles, lengths, and widths on the newest rewrite iteration when continuous options are active.
- **Parameter Aliases**: Supports Houdini standard names (`type`, `random_seed`, `thickness`, `thickness_scale`, `step_size`, `step_size_scale`) alongside backwards-compatible MiniHou legacy names (`geometry_type`, `seed`, `width`, `width_scale`, `step`, `step_scale`).
- **Rule File I/O**: Read rules directly from text files or use the `write_rules` action to save rules to disk.

## Why a tree can still look flat

The legacy implicit grammar uses only `+` and `-`, so it is deliberately planar even though the turtle supports three dimensions. Increasing Pitch or Roll does nothing unless the grammar contains pitch (`&`/`^`) or roll (`\`/`/`) symbols. To start spatially, open `examples/lsystem_3d.json` or the existing `examples/graphs/lsystem_3d_tree.json`. Its rule `A=F[&A]/[&A]/[&A]A` pitches branches and rolls subsequent branch directions by 137.5 degrees. Orbit the view to inspect depth; an axis-aligned view can hide it.

The spatial starter uses Premise A, four Generations, Separate Angles enabled, Yaw 33, Pitch 42, Roll 137.5, Step 0.3 and Width 0.03. These are explicit creation/example settings, not changes to old saved graphs that omit parameters. The engine tests verify rank-three positions, not merely a nonzero coordinate on one axis. Sweep converts the skeleton curves into tube geometry.
