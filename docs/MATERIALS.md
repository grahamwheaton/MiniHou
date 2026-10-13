# Materials

MiniHou does not shade anything. It only has to say **which faces use which material**, and carry that from Blender, through the
graph, to Blender again and to USD files. Shading (textures, shader networks) stays with the program that renders.

## How it is stored

A geometry carries two things:

* the primitive attribute `material_index`: an int per face (what Blender calls the face's material slot);
* the detail attribute `materials`: a list of **names**; entry `i` is the name for index `i`.

A name of `""` means "no material of ours": whatever the target already has in that slot stays (a Blender slot, a USD binding).
Geometry with a `material_index` and no list behaves exactly as before: Blender gets the integers and nothing else changes.

## The `set_material` node

*Attributes > Set Material.* `group` picks the faces (group names, ranges, or an expression such as `@area > 0.1`; empty is every
face) and `material` is the name. Faces outside the group keep what they had; a new name is appended to the list; an empty
name puts the faces back to "not ours". Nothing else in the graph needs to know: `merge` and `boolean` give every input one shared
list and rewrite the indices, so a Wood box merged with a Metal box has faces of each, and copying, deleting, fracturing and
subdividing keep `material_index` like any other face attribute.

Fracture and other cutting nodes make new faces that carry no material of ours (index 0, or the face they were cut from). The
usual use is *fracture, then `set_material` on the `inside` group*.

## In Blender

| where | what happens |
|---|---|
| **Reading** a `blender_object` / a modifier's input | the object's slot names become the list; each face's slot number is `material_index` |
| **Live link output** (`MiniHou_Output`, extra outputs) | the output object's slots are filled by name |
| **A MiniHou modifier** | the same, on top of the slots the object already had (the prefix's materials stay) |
| **Scene Objects, pieces mode** | every piece's mesh gets slots by name (the manifest carries the list) |

Rules that keep the user's work safe:

* A slot is **never removed or reordered**; slots are only added or changed, and only where the graph names a material.
* An empty name leaves its slot alone, so a material you assigned by hand to slot 0 survives a graph that never names slot 0.
* A name Blender does not know gets an **empty material of that name** (marked with the custom property `minihou_made`) and a note
  in the log, so a typo is visible instead of silently doing nothing. A material that exists is used as is, never modified.

## In USD

*Export* writes, under `/MiniHou/Looks`, one `Material` per name that faces use, and binds the mesh: directly when one name covers
every face, otherwise through `GeomSubset`s of the `materialBind` family (non-overlapping; faces with no material of ours stay
unbound). The material is a small `UsdPreviewSurface` whose colour is the mean `Cd` of its faces (grey without `Cd`), so any viewer
shows something sensible. A prim name must be an identifier, so `Wood Planks` is the prim `Wood_Planks`; the real name is kept in the
prim's custom data and in the file's MiniHou detail, and a MiniHou file reads back with the original names.

*Import* reads bindings from any file: a direct binding on the mesh, a binding inherited from a parent, and `materialBind`
subsets. The material's prim name becomes the name (with `_2`, `_3` for different materials that share a name). Where the mesh has no
colour of its own, faces take their material's base colour as `Cd` (`diffuseColor` of a UsdPreviewSurface, `base_color` of a MaterialX
standard surface) when it is a plain value, not a texture.

**Shading networks come through untouched.** The import remembers where each material lives (`usd_materials` in the detail: file and
prim path). When that geometry is exported again, the material is not rewritten: it is **referenced** from the original file, with a
path relative to the new file when possible. So textures, MaterialX graphs and anything else in the original material appear in the
new file exactly as they were. If the original file is gone, a preview surface is written instead.

## Checked against Blender

Blender 5.1's own USD exporter and importer were run against these files (tests in `tests/test_blender_materials.py`, which need the
`bpy` module and `usd-core` side by side and so run locally, not in CI): a Blender cube with two materials comes into MiniHou with
both names and the right faces; a MiniHou file with two materials imports into Blender with two slots and the right faces.

## Not done

* **MaterialX / shader authoring.** MiniHou writes no shading beyond the preview colour, on purpose; Hydra and Vulkan are renderer
  concerns and not involved.
* Materials on **curves and points** (USD binds them, MiniHou does not read or write that).
* Per-**instance** materials (a linked duplicate shares its source's materials; use *Scene Objects* pieces and `set_material`).
* **Textures** are not packed or copied: a referenced material points at the original files.
* Two different materials that share a name in Blender cannot exist (Blender names are unique); in USD they get suffixes on import.
