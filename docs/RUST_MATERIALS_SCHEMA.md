# Rust geometry material contract (version 1)

`Geometry.detail.materials` is an array of display names, indexed by the
int32 scalar `Geometry.prim.material_index`. The new
`Geometry.detail.material_bindings` is `{ "version": 1, "slots": [id_or_null, ...] }`.
The `slots` array parallels `materials`; each non-null string is an immutable
material ID from a document/session registry. `null` selects the neutral
material. IDs include their source namespace and descriptor revision; names
alone are never an identity when an ID is available. Older geometry may omit
bindings. Empty legacy slots remain distinct when material tables merge.

UV sets are float32 width-2 or width-3 attributes on `Geometry.vertex`, where
vertex means polygon corner. `uv` is the render UV set; other names are
preserved verbatim. Sampling uses XY. A point UV with the same name is a
fallback only when the corner attribute is absent. A shared point may have
different corner UVs on adjacent faces.

Retained faces keep their slot and corner values. Splits interpolate UVs from
corners of the source face; subdivision children inherit the parent slot.
Faces created without a source (such as a cut cap) use a neutral slot and an
invalid UV marker: their corner UV values are `NaN` in both sampled channels
(and Z for width-3 sets). Producers append a null binding if no neutral slot
exists. UV generation or explicit material assignment may replace these values.
Consumers must not sample an invalid UV or silently treat zero UV as valid.

The binding table, names, UV attributes and face indices are part of cooked
geometry and its cache identity. Image payloads and mutable material descriptions
live outside geometry. Missing, unsupported or loading images use the visible
checker fallback specified in `DESIGN_VIEWPORT_TEXTURES.md` Decisions.
