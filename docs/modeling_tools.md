# Modelling tools

These are useful initial implementations, not full Houdini SOP parity. All nodes
appear in the editor from their SDK registry declarations and carry parameter help.
No dependencies beyond NumPy are required. Inputs are never modified.

PolyExtrude's Divisions control creates evenly spaced extrusion steps, retaining every wall segment and only
the original back cap. Front, Side and Back output groups describe the complete result. Divisions default to
one, with a limit of 64 and a guard against more than one million source corner samples.

Subdivision's optional Crease Source accepts a mesh with matching point correspondence and topology. It supplies
corner weights, edge crease weights and named edge groups; geometry positions still come from the primary input.
Box and Sphere centre handles edit their source parameters directly, including modal G movement. Solidify's
thickness arrow follows a selected face normal and accounts for the shell offset. Ray's lift arrow follows a hit normal.

Scatter accepts a primitive Group and a scalar point or primitive Density attribute in surface mode. Point density
is interpolated linearly inside the sampled triangles; zero density excludes an area. These controls retain the
requested point count when positive area exists. UV Flatten's Seams field accepts named edge groups, edge IDs,
or endpoint pairs, supplementing angle and attribute seams.

Attribute Delete can use reference geometry to identify attributes separately on each domain, select an attribute
class and invert the match to keep chosen attributes. P is protected. Attribute Blur has an optional matching
Rest mesh and inverse-distance weighting; Uniform remains the default. Solver passes Input 2–4 to its body's
corresponding auxiliary ports and resets cached simulations when those sources change. Auxiliary inputs are
static for a cook, rather than independently sampled animation at each simulated frame.

## Edges, Bridge and Dissolve

`topo.edges(geo)` derives unique undirected edges, sorted by endpoint point ids.
An edge parameter accepts ids/ranges (`0-8:2`), endpoint pairs (`p0-3`), named
edge groups, wildcards, unions and `^` exclusions. Edge ids change with topology.
Empty patterns select all edges, except Dissolve where empty means no operation;
Bridge requires both selections explicitly.

`edge_group` creates named selections by edge pattern, boundary, both endpoints
in a point group, or any edge in a primitive group. Its combine modes match the
point/primitive group tools. `edge_group_promote` copies selected edges to
incident points or primitives for existing node group fields.

Selections are stored as `edge_group_NAME` on outgoing vertex/corner rows, with
both half-edges of a shared edge carrying membership. Open-curve endpoints carry
zero. This preserves the existing geometry and wire protocol. Blender receives
these as CORNER attributes, not native editable EDGE selection. Group and crease
data require edge-aware remapping when connectivity changes: the new modelling
nodes do this; arbitrary existing topology nodes may treat them as ordinary
corner attributes. Create edge groups after such nodes when exact edge semantics
matter. Position-only nodes retain them unchanged.

`polybridge`: select two disjoint, equal-count boundary chains or loops. The
node retains input geometry and makes a linear quad strip; `segments` inserts
interpolated rings. Polygon boundary winding is respected. `twist` shifts closed
loop correspondence; `reverse` is for standalone open-profile correspondence.
No automatic unequal-count resampling, caps removal, curvature control or
self-intersection repair. Existing UV corners remain intact; new corner/point
floats interpolate, discrete values follow the source, primitive values follow
the source profile face. New connecting edges start with no edge membership.

`dissolve_edges`: selected manifold interior edges merge regions into single n-gons.
The angle limit prevents unintended folds; default seam protection rejects
differing primitive attributes/materials and corner UV/data seams. Primitive
groups union; other primitive values come from the lowest-numbered source.
Disabling seam protection explicitly permits that loss. Boundary edges,
inconsistent winding, region holes and branching boundaries raise errors.
Unused points remain, preserving their attributes and numbering.

Design references: [SideFX Poly Bridge](https://www.sidefx.com/docs/houdini/nodes/sop/polybridge.html),
[Dissolve](https://www.sidefx.com/docs/houdini/nodes/sop/dissolve-.html).
Implementations are original and intentionally narrower.

Run isolated-checkout tests with `python tools/test_worktree.py tests.test_edge_tools`.
The runner prepends its own checkout even when an embedded Python `_pth` points
at another MiniHou checkout.

## Skin / Loft and Poly Expand 2D

`skin_profiles` connects selected profiles in primitive-number order with quads. Use Merge
in the intended profile order first. Profiles must have equal corner counts,
matching closure and deliberately aligned corner correspondence; use Resample
upstream when necessary. `keep_profiles` retains the sources; `close_loft` joins
last to first. There is no spline fitting, automatic alignment or cap generation.
Points and their attributes are retained; quad corners inherit their source
profile UVs, and primitive data comes from the earlier profile. Overlapping or
crossing profiles are not automatically repaired.

`polyexpand2d` performs a miter offset of independent strictly convex closed loops
in their fitted planes. Positive distance expands regardless of winding. It
rejects open, concave, nonplanar, repeated/collinear, self-crossing or collapsed
profiles and excessive miters. No straight skeleton, hole handling, Boolean
union, or intersecting-profile cleanup. Selected profiles get independent point
copies to avoid moving shared unselected geometry; unused original points remain.
All attributes copy from corresponding source elements (UVs are not reprojected).

References: [SideFX Skin](https://www.sidefx.com/docs/houdini/nodes/sop/skin.html),
[Poly Expand 2D](https://www.sidefx.com/docs/houdini/nodes/sop/polyexpand2d.html).

## Deformation workflows

`soft_transform` blends a pivoted XYZ rotation/scale/translation with the original
position using radial linear or smoothstep falloff. The center is both the pivot
and falloff origin; the group restricts which points may move. No geodesic falloff.

`lattice_grid` makes a complete 3D point grid. Branch it to a Transform, Soft
Transform or Wrangle and connect the original to `lattice.rest`, the edited copy
to `lattice.deformed`, and the model to `lattice.geo`. Rest controls must form a
complete axis-aligned Cartesian grid (nonuniform axis spacing is accepted).
Deformed point order/topology must match. Interpolation is trilinear displacement,
not Bezier/NURBS. Outside points either use clamped boundary displacement or stay
unchanged. An arbitrarily rotated rest grid is not supported.

`point_deform` uses the same three-input arrangement with an arbitrary control
cloud. A weighted local affine displacement fit reproduces affine transforms
with at least four noncoplanar neighbours. Rank-deficient control sets constrain
only their spanned directions; unconstrained displacement gradients are zero.
Radius zero means nearest `max_points` controls without cutoff; otherwise
uncaptured points remain unchanged. Point correspondence is by index, not ids.
There is no persistent capture, topology matching, piece matching or volume
preservation. NumPy search is bounded to 50 million model/control pairs; use a
small control cloud. Neighbour/radius transitions can be discontinuous.

All three keep topology, UVs, materials, groups and other attributes. Existing
point/corner/primitive `N` is adjusted using a finite-difference inverse-transpose
Jacobian (primitive Jacobians average their corners); collapsed transforms use a
pseudoinverse. Normals at hard selection/capture boundaries are approximate;
use Normal downstream when geometric face normals are preferable.

References: [Soft Transform](https://www.sidefx.com/docs/houdini/nodes/sop/softxform.html),
[Lattice](https://www.sidefx.com/docs/houdini/nodes/sop/lattice.html),
[Point Deform](https://www.sidefx.com/docs/houdini/nodes/sop/pointdeform.html).

## Subdivision creases

Use `crease` with domain `edges` and an edge group/pattern, or domain `points`
and a point group. `sharpness` is the number of subdivision levels of influence;
fractional values blend the smooth and sharp rules. Edge data is stored in vertex
`creaseweight`, point corners in `cornerweight`. Set zero to clear a selection.
The existing `subdivide` node now has `use_creases`. Two sharp incident edges use
a boundary-curve rule; three pin a corner; point corner weights pin explicitly.
Sharpness decreases by one per level. Only the two child edges inherit each old
edge's group/crease; new face spokes start at zero. Linear subdivision propagates
and decays metadata without smoothing positions. No OpenSubdiv parity is claimed
for fractional junctions/nonmanifold topology. Blender receives corner crease
metadata; native Blender subdivision modifier crease editing is not implemented.

Reference: [SideFX Crease](https://www.sidefx.com/docs/houdini/nodes/sop/crease.html).

## Validation and integration (2026-10-11)

Baseline: tested `7d55596`; branch: `codex/modeling-tools`.

* Focused geometry suites passed at each stage: edge patterns/groups, open and
  closed bridge winding, segment interpolation, rigid-transform equivariance,
  dissolve seam/hole/nonmanifold rejection, profile offsets in rotated planes,
  capture transforms/rank deficiency, normal transforms, crease decay and UVs.
* Full engine regression: **575 tests, 464 passed, 111 skipped**, 101.841 seconds.
  Skips reflect unavailable optional dependencies/host tests in the engine runtime.
* Blender **5.2.0 LTS**: **3 actual host tests passed**. All eight new workflows
  traversed protocol serialization and the native mesh writer to evaluated meshes.
  Bridge UVs/material indices/groups and position-only updates were checked.
  A separate engine subprocess also served the Bridge graph over TCP; changing
  Segments through the Geometry Nodes modifier rebuilt the evaluated proxy.
  These are background host tests, not interactive editor/viewport acceptance.
* A subsequent focused run of registry/workflow and portability checks passed;
  the three Blender tests skip under ordinary Python and pass in the host run.
* No new dependencies. The engine tests and CLI subprocess helpers prepend this
  worktree to `sys.path`; they do not rely on the bundled runtime's original
  checkout `_pth`. Source paths are printed by the test/smoke runners.

To repeat the live Blender tests, set `MINIHOU_ENGINE_PYTHON` to the prepared
external Python executable before running `tools/blender_modeling_smoke.py` in
Blender. The standalone mesh tests need only Blender's bundled NumPy.

The delivery bundle excludes `7d55596` and requires that prerequisite commit.
Import/integrate the branch changes without replacing the original checkout's
concurrent repeat-loop work. Keep the original checkout's extra atomic-library
reload fix: this branch intentionally does not modify `minihou/library.py` or
the repeat-loop implementation. `demo.json` and `rock_scatter.json` are untouched.
No push or merge was performed.
