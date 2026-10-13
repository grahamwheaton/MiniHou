# Passing geometry and images between Blender, MiniHou and Houdini

| | Blender -> MiniHou | MiniHou -> Blender | Files (to or from Houdini and others) |
|---|---|---|---|
| **Mesh** | `blender_object` (live; modifiers applied) | the output object (live), `scene_objects` *pieces* | USD (`usd_import`, File > Export USD) |
| **Curves** | `blender_object` on a curve, hair-curves or Geometry Nodes object | `scene_objects` mode **curves** (a real curves object); loose edges of the output mesh | USD `BasisCurves` |
| **Points / point clouds** | `blender_object` on a point-cloud object (or loose vertices of a mesh) | `scene_objects` mode **points** (a real point cloud); vertices of the output mesh | USD `Points` |
| **Instances** | (instances are points that name objects) | `scene_objects` mode **instances** (linked duplicates) | USD `PointInstancer` |
| **Images** | `blender_image` (live; follows painting) | `image_out` (makes or updates a Blender image) | `image_file` / `image_out` PNG (8-bit sRGB or 16-bit) |
| **Materials** | slot names | slots filled by name | USD bindings ([MATERIALS.md](MATERIALS.md)) |

The live link works in both directions at once; the file formats are how a program MiniHou is not connected to (Houdini) joins in.
"Houdini" below means any program that reads USD or PNG.

## Curves and point clouds into MiniHou

`blender_object` reads whatever an object evaluates to. A legacy curve object, a hair-curves object, a point-cloud object, text, and the
output of a Geometry Nodes tree are all fine, and an object whose evaluated geometry holds **several kinds at once** (a mesh, curves and
points from one tree) arrives as one geometry with the parts side by side.

* **Curves** become open primitives, one per spline. They are sampled **exactly as Blender draws them**: Bezier handles at the curve's
  resolution, NURBS with their order, knot mode (normal, endpoint, Bezier) and weights, Catmull-Rom, poly. A cyclic curve is a closed
  primitive. The tests compare every type with Blender's own evaluation. `radius` and `tilt` ride along as point attributes (blended the way
  Blender blends them), a curve-domain attribute becomes a primitive attribute, a point-domain one a point attribute, and the first colour
  attribute is `Cd`.
* **Point clouds** are points with `radius` and their attributes.
* **Beveled curves and text** evaluate to meshes, and are read as meshes (the curves component is empty then).
* The old behaviour for a *mesh* with loose edges is unchanged: they are joined into long curves (`join_edges`). Real curves are never
  re-joined.

## Curves and point clouds out of MiniHou

*Scene Objects* has two more modes:

* **curves**: one Blender curves object (poly curves) with every open primitive; closed primitives (and polygons) become cyclic curves.
  Point attributes, vertex attributes and primitive attributes are carried (`attributes`, or `*` for all; `radius` and `tilt` always), and
  `Cd` becomes a colour attribute called `Color`.
* **points**: one Blender point-cloud object with every point and its attributes (`radius` always).

The object is found again by its marker, so the user's modifiers, materials and constraints stay when the data changes; if the node's mode
changes, the object is replaced by one of the right kind (its modifiers cannot move). A graph's main output is still the one mesh object
(curves as loose edges, points as vertices).

## Images

An image in a graph is a **pixel grid** (see `minihou/images.py`): a point per pixel, row by row from the bottom left as Blender stores
images, with `Cd` (and `Alpha`), `uv`, and the detail `grid_columns` / `grid_rows` the heightfield nodes already use. So every geometry node
is an image node: a wrangle reads and writes pixels (`@Cd`, `@P`), `attribute_blur` blurs, `heightfield_erode` erodes a height map.

* `blender_image` reads a Blender image (it is watched: painting in the Image Editor recooks the graph). `max_size` averages big images
  down. 8-bit sRGB images become linear colour (`colorspace: auto`); `stored` keeps the numbers as they are (masks, heights, normal maps).
  `height` lifts the points by a channel: `z = value * height_scale`.
* `image_file` reads a PNG (16-bit keeps every bit; other formats when Pillow is installed).
* `image_out` writes the point attribute `source` to a Blender image: from a pixel grid (`lattice`) or by baking any points by X / Y
  position (`bake`: mean per pixel, empty pixels transparent). `encoding` is `linear` (a float image), `srgb` (an 8-bit sRGB image) or `data`
  (a float Non-Color image). `file` also writes a PNG (8-bit sRGB for `srgb`, otherwise 16-bit), which is how a height map reaches Houdini.
  The image keeps its place in the file and its users; it follows the data's size. Save or pack it to keep it.

`examples/graphs/height_image.json` makes an eroded height map and sends it to Blender.

## Houdini

MiniHou cannot talk to a running Houdini. USD carries geometry (meshes, curves, points, instances, materials by binding) and PNG carries
images; both are written and read by Houdini natively. Not done: `.bgeo`, `.abc` and OpenEXR files, Houdini's volumes and VDB.

## Limits

* Curves are sampled when read; the control points and handles are not kept, so a curve cannot be sent back as Bezier (it comes back
  as a poly curve with the sampled points).
* A Geometry Nodes object that outputs *instances* is read realised only if the tree realises them; unrealised instances are not read.
* Images are limited to 16 million pixels in a graph and 8192 on a side; `blender_image` averages down to `max_size`.
* An image the graph makes with `image_out` cannot be read back by `blender_image` in the same file (it would feed itself).
* The modifier path (`modifier_input`) still takes a mesh only; curves and points come in with `blender_object`.
