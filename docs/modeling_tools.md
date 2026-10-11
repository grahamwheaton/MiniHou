# Modelling tools

These are useful initial implementations, not full Houdini SOP parity. All nodes
appear in the editor from their SDK registry declarations and carry parameter help.
No dependencies beyond NumPy are required. Inputs are never modified.

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

`dissolve`: selected manifold interior edges merge regions into single n-gons.
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
