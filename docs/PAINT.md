# Painting and assigning attributes

Three ways to write values of an attribute by hand, all ending in one node, **`attrib_paint`** (Attribute group): the viewer's **Paint**
mode, the **Quick Assign** panel and cells of the **Spreadsheet**. They are also available to scripts and the MCP server as the
`paint_assign` edit op.

## What `attrib_paint` holds

One attribute on one domain:

| domain | what an element is | notes |
|---|---|---|
| `point` | a point | |
| `prim` | a primitive (a face, or a curve) | "faces" and "primitives" are the same thing in MiniHou |
| `edge` | a *derived edge* (the sorted endpoint pairs of `topo.edges`, the numbering the viewer uses) | MiniHou has no edge attributes, so painting an edge sets membership of the **edge group** of that name (`edge_group_<name>`) |

The vertex (corner) domain can be read in the spreadsheet but is not painted: the viewer has no corner numbering.

Parameters: `attribute` (name), `domain`, `kind` (`float`, `int`, `vec3`), `default` / `default_vec` (the value of elements nobody painted
when the input has none), and a hidden `data`. Elements nobody painted keep the **input's value** of that attribute, so a paint node
can refine a weight map made upstream. Painted values are stored as `(element number, value)` pairs in `data` (`mhp1:` + zlib + base64,
at most 4 million elements); a stroke only costs what it touched. Because they are stored by number, they follow the geometry only while
upstream keeps the numbering: edits to elements that no longer exist are **ignored and counted** in the detail attribute
`paint_stale_<name>` (never an error), like a `blast` pattern. Damaged `data` is a clean node error (clear the parameter). The attribute reaches
Blender as a native attribute (float, int, 3-vector; edge groups as booleans) like any other.

## Paint mode (key `5`, or the Paint button)

Turns the left mouse button into a brush; Alt-drag, middle and right buttons still navigate. The panel (drag it by its title bar) sets the
attribute, domain, type and value, and the brush: **mode** (Paint = move toward the value, Add, Subtract, Multiply, Smooth, Erase), **radius**
(pixels), **strength**, **falloff** (smooth, linear, constant) and **visible surface only**. `Shift` erases (back to the input's value),
`Ctrl` smooths, `[` / `]` change the radius, `Esc` leaves. The painted attribute is shown as colour: a heat ramp over *Display > Range* for
scalars (or grey), the colour itself for vectors, magenta edges for an edge group. "Show the attribute as colour" also works outside paint
mode.

How a stroke works: the viewer rasterises a depth and triangle-id buffer once when the stroke starts, each dab finds the surface under the
cursor and blends every element within the brush radius (in 3-D) toward the value, and the colours update at once. When you let go the touched
elements go to the engine as **one** `paint_assign` edit, so a stroke is **one undo step**. If the viewed node is not a matching
`attrib_paint` node, one is inserted after it in the same edit.

Paint needs the real geometry, like selecting: with **Detail: Auto** the full detail loads when paint mode starts; a geometry too big for the viewer,
or Detail: Fast, refuses (the message says how big). It also refuses while the **cook Proxy** is below Full, because element numbers at a
proxy level are not the real ones, and a keyboard shortcut or a selection mode (1-4) leaves paint mode.

### Mask and mirror

**Mask** (in the panel, for both tabs) limits what a brush or Assign may change:

* *Group*: only members of a group of the viewed geometry. A point group masks faces whose points are all in it (and a face group masks
  the points it uses), like every group field; for edges the mask is an edge group. The membership comes from the engine
  (`POST /api/preview` takes `mask: {name, domain}` and returns the array `mask`), so it uses the viewer's numbering and needs the real geometry.
  The group name box lists the groups the viewed geometry has.
* *Selection*: the points, edges or faces selected in the view when you press **Set from selection** (selected faces or edges stand for
  the points they use when painting points). Entering paint mode clears the viewport selection, so the mask keeps its own copy; set it first,
  then paint.
* *Invert* paints outside the mask. **Smooth** still averages over masked-out neighbours but only writes the masked ones; **Erase** respects the mask.
* *Assign to all* becomes **Assign to mask** and writes the value into the masked elements only.

**Mirror** (X / Y / Z, paint tab) reflects every dab across the enabled planes, about the world origin or the centre of the geometry's bounds.
A mirrored dab does not look at the screen (its surface is usually on the far side), so *Visible surface only* applies to the dab under the
cursor. An element in reach of two brushes takes the stronger weight rather than both. Quick Assign does not mirror.

## Quick Assign (the Assign button)

Same panel, other tab. Pick the attribute, type and value, then **Assign to selection** (the viewport's selected points, edges or faces),
**Assign to all** (clears what was painted and sets the node's default) or **Clear painted**. Each press is one undo step. The attribute name
box lists the attributes the viewed geometry has.

## Spreadsheet (the Spreadsheet button)

Rows of the viewed node's points, vertices, primitives, edges or detail attributes, paged by the engine (`POST /api/spreadsheet`, at most 500 rows
a page) and virtualised in the page, so a million points cost the same as ten. Click a row's **number** to select that element in the view;
*Selected only* filters to the viewport selection. **Double-click a cell** of an attribute (a number, or three numbers for a vector) to edit it:
the edit goes through `paint_assign`, into the viewed `attrib_paint` node or a new one. Built-in columns (`P`, `point`, `size`, ...) are read-only.

## For scripts

```json
{"op": "insert", "after": "mesh", "type": "attrib_paint", "id": "weights", "params": {"attribute": "weight", "domain": "point", "kind": "float"}}
{"op": "paint_assign", "id": "weights", "indices": [0, 5, 9], "value": 0.5}
{"op": "paint_assign", "id": "weights", "indices": [5], "erase": true}
{"op": "paint_assign", "id": "weights", "clear": true, "default": 0.25}
```

`values` gives one value per element (a number, or three for `vec3`); an `int` attribute rounds. `POST /api/preview` takes `attr: {name, domain}` and
returns that attribute's values (for a view that is not reduced) for colouring.
