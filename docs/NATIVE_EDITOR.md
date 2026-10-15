# Native Editor

The native MiniHou editor (`minihou-app`) is a high-performance desktop application built in Rust using `egui` (0.33.3) and `wgpu` (27). It provides full parity with the web editor workflow while eliminating browser overhead and garbage collection pauses.

## Units

MiniHou follows Blender's model for units and dimensions: internal model coordinates remain invariant, while display coordinates are converted for user presentation and parameter input.

### Coordinate Invariance

- **Internal Coordinates**: Geometry coordinates, vertex positions, transformations, and saved graph JSON always operate in internal coordinate units.
- **Display Coordinates**: Display values are presentation-only. Changing the display unit or unit scale never alters or resizes existing geometry or scene files.

### Units Conversion Formula

Display and internal values are related by:
- `display_value = internal_value * unit_scale / metres[display_unit]`
- `internal_value = display_value * metres[display_unit] / unit_scale`

The supported length units and their metre equivalents:
| Unit | Symbol | Factor (metres) |
| :--- | :--- | :--- |
| Millimetre | `mm` | 0.001 |
| Centimetre | `cm` | 0.01 |
| Metre | `m` | 1.0 |
| Kilometre | `km` | 1000.0 |
| Inch | `in` | 0.0254 |
| Foot | `ft` | 0.3048 |

### Default Settings

- **Display Unit**: `mm` (millimetres)
- **Unit Scale**: `1.0` (1 internal unit = 1 metre = 1000 mm)

Under defaults, a model coordinate distance of `1.0` displays as `1000 mm`. Setting `unit_scale` to `0.001` adjusts physical interpretation so that 1 model unit displays as `1 mm`.

### Preferences

The **Preferences** window (`Ctrl+,` or from the top menu) includes a **Units** tab:
- **Length display**: Dropdown choosing from `mm`, `cm`, `m`, `km`, `in`, `ft`.
- **Unit scale**: Drag value specifying the scale multiplier (metres per internal unit). Must be positive and finite; defaults to `1.0`.
- **Persistence**: Stored in `Preferences` under `units`, serialised to `~/.minihou/preferences.json` (also backwards-compatible with `length_unit` and `unit_scale` fields).

### Length Parameter Identification

MiniHou maintains explicit semantic length metadata ported from `minihou/web/static/length-params.js`. Generic floats and vector attributes (e.g., counts, segments, weights, normals, UVs, scales) are never converted.

Length parameters include:
- `point`: `center`
- `box`: `center`, `size`
- `sphere`: `center`, `radius`
- `grid`: `center`, `size_x`, `size_y`
- `tube`: `center`, `radius_bottom`, `radius_top`, `height`
- `torus`: `center`, `radius`, `thickness`
- `circle`: `center`, `radius`
- `line`: `origin`, `length`
- `helix`: `radius`, `radius_end`, `height`
- `transform`: `translate`, `pivot`
- `soft_transform`: `translate`, `center`, `radius`
- `polybevel`: `distance`
- `polyextrude`: `distance`, `inset`
- `inset`: `amount`
- `outset`: `amount`
- `pushpull`: `distance`
- `match_size`: `size`, `center`
- `lattice_grid`: `size`, `center`
- `mirror`: `origin`, `distance`
- `clip`: `origin`
- `fuse`: `distance`
- `resample`: `length`
- `fillet`: `radius`
- `sweep`: `radius`
- `curve_simplify`: `tolerance`

### UI Integration

1. **Parameters Panel**:
   - Parameter rows identified as lengths display the current unit symbol in their label, e.g. `Size (mm)`.
   - Number inputs and sliders format displayed numbers in the active display unit with trailing zeros trimmed (e.g. `1000 mm`).
   - Text inputs parse both bare numbers (interpreted in current display unit) and explicit suffixes (e.g. typing `50cm`, `2m`, or `0.05in` converts correctly to internal coordinates).
   - Vec3 length parameters format each component with its unit and parse per-component inputs.

2. **Tape Measure & Dimensions** (`ui/sketch.rs`):
   - Measurement labels between picked endpoints render in formatted display units (e.g. `1000 mm`).
   - Active preview segments during tape measurement draw their real-time length with unit suffix.
   - Numeric typed input in sketch mode parses length with units (e.g., bare numbers or explicit unit suffixes).

3. **Transform Gizmos** (`ui/transform_gizmo.rs`):
   - Move tool axis drag displays real-time delta with unit readout (e.g. `250 mm`).
   - Direct keyboard typing during Move drag accepts bare numbers in display units or explicit suffixes (`mm`, `cm`, `m`, `km`, `in`, `ft`).
   - Rotation and Scale remain dimensionless (degrees and ratio multipliers).

4. **Viewport Overlays & Snapping** (`ui/viewport_ui.rs`):
   - Header snap increment button displays formatted length when snapping is enabled (e.g. `1000 mm` or `250 mm`).
   - Snapping popover `Increment` field displays and parses values in display units.
   - Proportional Editing popover `Radius` field displays and parses falloff radius in display units.

5. **Geometry Information** (`ui/geo_info.rs`):
   - Bounding box statistics (`Min`, `Max`, `Size`) display XYZ coordinates formatted in current display units.
