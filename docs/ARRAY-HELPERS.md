# Array library helpers

The bundled `nodes` library supplies four editable graph systems. Add them from the node menu, connect geometry to **source**, and adjust their exposed controls. With no source connected, a small box provides a preview. Double-click a system to inspect its target generator and Copy to Points wiring.

| System | Controls | Placement |
| --- | --- | --- |
| Array (`array`) | Count, Offset, Origin | First copy at Origin, then one XYZ Offset per copy |
| Circle Array (`circle_array`) | Count, Radius, Center, Start Angle, Rotate Copies | XY ring, Z up; degrees from +X; no duplicate endpoint |
| Grid Array (`grid_array`) | Columns, Rows, Spacing X, Spacing Y, Origin | XY lattice from Origin, columns along X; signed spacing allowed |
| Array Between Two Positions (`array_between_two_positions`) | Count, Start, End | Includes both endpoints; Count 1 uses Start |

Source coordinates are preserved relative to each target. Put the source pivot at local zero upstream when you want a copy centred on each target. Circle copies remain upright by default; Rotate Copies turns them around Z to follow each radial angle.

Counts must be integers from 1 to 10,000. Grid rows times columns must also be at most 10,000. Defaults produce 5 linear copies, 12 circle copies, a 5 by 5 grid, or 5 copies between positions. The existing Copy to Points 256 MiB allocation guard also applies to realised copies; heavy sources can require smaller counts.

For Blender object instancing, use `array_linear_points`, `array_circle_points`, `array_grid_points`, or `array_between_points` directly, then connect their output to **Instance**. This keeps object copies as lightweight target points. The graph systems intentionally use **Copy to Points** so their source can be arbitrary MiniHou geometry.

Open `examples/array.json`, `examples/circle_array.json`, `examples/grid_array.json`, or `examples/array_between_two_positions.json` for ready-to-cook examples. The node library must be loaded (the normal MiniHou app loads the bundled `nodes` folder). No Python graph execution setting is needed: these use ordinary library nodes.
