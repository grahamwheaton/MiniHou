# Edge domain for attributes (design)

Status: planned 2026-10-18, not started. Owner: to be assigned after an Argon plan review.

## Problem
Blender stores `bevel_weight_edge`, `crease_edge`, `sharp_edge` and similar values on edges. MiniHou has point, vertex (corner) and primitive domains only, so `mesh_reader.py` skips edge attributes and they are lost on the way in. A cube from Blender lost its bevel weights for this reason.

## Goal
Edge attributes survive the round trip Blender -> MiniHou -> Blender unchanged, and nodes can read and write them.

## Options
- **A (chosen): a real edge domain.** `Geometry` gets an `edge` attribute table keyed by the unique edges of the topology. Every node that builds or changes topology must carry edge attributes through, and the Python reference and Blender add-on must match.
- **B (interim, not chosen):** carry each edge value on its two corners with an `edge:` prefix. Round-trips unmodified meshes only.

## Work breakdown
1. Core: add the edge table to `Geometry`; define how edges are identified (sorted vertex pair, plus an edge index per prim-loop); validate it in `Geometry::validate`.
2. Core nodes: every node that changes topology (subdivide, extrude, boolean, knife, bevel, copy, merge, blast, delete, polyextrude, polybevel, fracture and others) must keep, drop or interpolate edge attributes explicitly. Start with the list in `docs/TODO_IMAGE_NODES.md`.
3. Blender add-on: export edge attributes from `mesh.edges` and import them back; keep the existing point/corner/face paths unchanged.
4. Python reference (`minihou/`): match the Rust edge domain, or document an exemption for each node.
5. Tests: round-trip a cube with bevel weights and crease through the link (real Blender test), and core tests per node that touches topology.

## Risks
- Topology-changing nodes that silently drop edge attributes (must be listed, not hidden).
- Python parity tests will need the edge domain in both engines or an explicit exemption list.
- Performance: an extra table per geometry; check it stays empty when unused.
