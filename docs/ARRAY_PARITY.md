# Array node parity (Blender 5.2 Array)

The MiniHou Array node follows Blender 5.2's node-based **Array** modifier (Line, Grid and Radial shapes), not the
Array (Legacy) modifier. Parameters are shown by mode through `visible_when` in `param_meta.json`.

| Blender option | Status | Node behaviour / reason |
| --- | --- | --- |
| Shape: Line | Implemented | `mode=line`, `count` copies including the first. |
| Line spacing: Relative | Implemented | `spacing=relative`; `relative_offset` multiplies the input size per axis. `constant_offset` is also added so migrated legacy graphs keep their shape. |
| Line spacing: Offset | Implemented | `spacing=offset`; `constant_offset` is the step. |
| Line spacing: Endpoint | Implemented | `spacing=endpoint`; the last copy sits at `endpoint`, the rest are spread evenly. |
| Shape: Grid | Implemented | `mode=grid`, `count_x`, `count_y`, `count_z` (at most 10,000 copies in all). The same spacing modes apply per axis: Relative uses the input size on that axis, Offset a distance, Endpoint the far corner. Neighbours along each axis can weld. |
| Shape: Radial | Implemented | `mode=radial`; `radial_mode` Count (`count`) or Distance (`distance`, arc length between copies); `radius`, `axis` (X, Y, Z), `angle` in degrees, `align_rotation` (on by default). A full turn spaces copies evenly; a partial angle puts the last copy at the end of the arc. Copy 0 sits at `radius` along the next axis (X about Z, Y about X, Z about Y). |
| Randomize | Implemented | `random_seed`, `random_position` (per axis, either way), `random_rotation` (degrees per axis, about each copy's origin), `random_scale` with `uniform_scale`, `random_uv`. Each copy draws from a stable hash of (seed, copy number, channel): adding copies never changes existing ones. Copy 0 is randomised too. |
| Pack / instances | Implemented | `pack` outputs one shared source and one instance point per copy (`packed_xform` holds rotation and scale). UV offsets, random UV and welding are off while packed. Unpack gives the same geometry as the unpacked array without UV offsets or welding. |
| Merge, Distance | Differs | `merge_vertices` and a positive `merge_distance` weld neighbouring copies. Distance zero disables welding to keep older graphs unchanged. |
| Merge first and last | Implemented | `merge_first_last` also welds the last copy to the first. |
| UV Offset U/V | Differs | `uv_offset_u` / `uv_offset_v` shift float2 corner attributes named `uv` or `uv_*` by the copy number. |
| Copy index output | Implemented | `output_copy_index` writes the point attribute `copy_index` (copy number; also on instance points when packed). |
| Copy fraction output | Implemented | `output_copy_fraction` writes the point attribute `copy_fraction`, 0 for the first copy and 1 for the last. |
| Per-copy group | Differs | `output_copies_group` writes the face attribute `array_copies` holding each face's copy number (one attribute instead of one group per copy). Not written when packed. |
| Object Offset, Start/End Caps | Removed | Dropped in favour of the Blender 5.2 Array options. The node has one `geo` input. |
| Copy to Curves | Not yet | Planned separately. |
| Viewport gizmos | Not yet | Planned separately. |

## Older graphs

Graphs saved with the legacy `fit_type` still load. Fixed Count keeps its offsets and count (Relative spacing).
Fit Length becomes Endpoint spacing: the count comes from the old length and step, and the last copy lands where it did
before, so the geometry is unchanged. Other `fit_type` values report an error.

The node limits copies to 10,000 to bound allocations. UV layers are MiniHou corner float2 attributes.
