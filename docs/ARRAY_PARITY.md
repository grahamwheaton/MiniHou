# Array node parity (Blender 5.2 Array Legacy modifier)

Reference: [Blender 5.2 manual](https://docs.blender.org/manual/en/5.2/modeling/modifiers/generate/array_legacy.html) and [Blender v5.2.0 `MOD_array.cc`](https://github.com/blender/blender/blob/v5.2.0/source/blender/modifiers/intern/MOD_array.cc). Blender 5.2 also has a newer Array modifier; the options requested here are the **Legacy** modifier's options.

| Blender option | Status | Node behaviour / reason |
| --- | --- | --- |
| Fixed Count, Count | Implemented | `fit_type=fixed_count`, `count`; includes the source copy. |
| Fit Length, Length | Implemented | `fit_type=fit_length`, `fit_length`; uses the step translation length and keeps at least one copy. |
| Fit Curve, Curve | Deferred | Needs a curve object/path input and its evaluated length. |
| Relative Offset X/Y/Z | Implemented | `relative_offset` multiplies the input bounds on each axis. |
| Constant Offset X/Y/Z | Implemented | `constant_offset` adds a translation per step. |
| Object Offset | Differs | `object_offset` input uses a single transform carrier: its point position and, for a packed instance, its `packed_xform` matrix. MiniHou geometry has no Blender object transform; the source object's inverse world transform is unavailable. |
| Offset enable switches | Differs | Zero vectors disable relative/constant components; connecting `object_offset` enables its component. Separate Blender switches are not exposed. |
| Merge Vertices, Distance | Differs | `merge_vertices` and positive `merge_distance` weld neighboring copies and touching caps. Distance zero disables welding to preserve existing graphs, while Blender can weld exact matches at zero. |
| First and Last Copies | Implemented | `merge_first_last` welds the last copy against the first when merging is enabled. Caps are excluded. |
| UV Offset U/V | Differs | `uv_offset_u` / `uv_offset_v` shift float2 corner attributes named `uv` or `uv_*` by copy index. Blender identifies UV maps by mesh metadata and accepts other names. |
| Cap Start, End | Implemented | Optional `start_cap` and `end_cap` inputs sit one step before/after the array; touching cap vertices can weld. |
| Other attributes / materials | Differs | Point, corner, edge and primitive data are copied through MiniHou's geometry merge. Welded points and edges retain the first surviving source attributes. Blender also remaps vertex groups and handles some mesh specific data that MiniHou does not model. |
| Geometry output | Implemented | One `geometry` output, keeping existing downstream graphs and groups. |

The node limits copies to 10,000 to bound allocations. Blender uses a mesh vertex budget instead. UV layers are MiniHou corner float2 attributes; Blender mesh UV map metadata is not represented separately.
