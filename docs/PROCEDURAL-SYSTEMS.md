# Procedural modelling systems

These nodes provide three usable, bounded workflows. The external product references describe desired modelling behavior; this implementation does not reproduce their proprietary systems or claim equivalent maturity.

Find the nodes on the **Procedural** shelf or through the node search. All controls use the normal node parameter panel. Examples are ordinary editable graphs under `examples/procedural_*.json`; open one using the graph picker.

## Footprint building rules

Connect a horizontal polygon footprint to **Building Rules**. A 2-by-2-point Grid makes a rectangular footprint. Each input primitive generates its own building through a fixed grammar:

1. Repeat storeys by height.
2. Subdivide each facade into evenly spaced bays, approximately `bay_width` wide.
3. Add a window panel and seeded balcony choice per bay.
4. Finish with a flat, pyramid or gable roof.

`rules` is a JSON list of ordered floor overrides. A matching later rule overrides only the supplied values. For example:

```json
[
  {"floor":"ground", "window_height":0.8, "balcony_probability":0},
  {"floor":"top", "window_width":0.7, "balcony_probability":1}
]
```

`floor` accepts `all`, `ground`, `top` or a zero-based storey number. The three overridable fields are `window_width`, `window_height` and `balcony_probability`. Window dimensions are fractions of a bay/storey. A zero window dimension omits the panel. Ground-floor balconies are always omitted. `seed` makes choices repeatable; footprint order is part of the seed identity.

Output primitives carry `building_id`, `floor`, `facade`, `bay`, `component`, `Cd`, and groups `walls`, `windows`, `balconies`, `roof`, `foundation`. Components are 0 wall, 1 window, 2 balcony, 3 roof, 4 foundation. These work with existing groups, materials, Blast and For Each nodes.

Open **procedural_building.json** for a four-storey building with taller ground-floor windows, balconies and a gable roof. Change Storeys, Bay Width, Roof or Seed.

Scope and limits:

- Strictly convex simple XY polygons only, with no holes, repeated or collinear corners. Clockwise input is accepted. Multiple footprints are supported. Concave/self-intersecting footprints produce an error rather than incorrect roofs.
- Gable roofs require a rectangle; their ridge connects the midpoints of edges 0 and 2. Pyramid and flat roofs accept all supported footprints.
- Windows are shallow panels over walls, not Boolean openings. Balconies are surface slabs/parapets. There are no interior rooms, doors, UV unwrap, architectural compliance rules or arbitrary recursive grammar language.
- Output is an assembly of independently indexed surface faces, not a welded watertight solid. Use semantic IDs for grouping; ordinary connectivity will see separate faces.
- Limits: 100 footprints, 256 vertices per footprint, 100 storeys, 2,000 bays per facade, 200,000 generated faces. Narrow facades always receive at least one bay.

## Geometric goal solver

This solver is **geometric form-finding**, separate from the MuJoCo rigid-body system. Chain goal nodes along one geometry stream, then connect the result to **Goal Solve**:

```
Line -> Goal Length -> Goal Gravity -> Goal Pin -> Goal Solve -> Sweep
```

Each goal captures its data into that geometry's detail attributes. **Do not Merge branches of goals:** Merge concatenates geometry and does not combine goal recipes. Do not alter topology between capturing goals and solving; the solver checks the captured topology signature. Change shape/topology upstream of the whole chain instead.

- **Goal Length:** captures every topology edge's rest length multiplied by Scale. Strength is the fraction of correction per iteration, not an elastic material stiffness.
- **Goal Collision:** equal-radius point spheres repel. All point pairs participate, including connected neighbours. This is useful for packing, not full mesh/cloth collision.
- **Goal Surface:** connect a fixed polygon mesh to Target. Selected points attach to the nearest surface point; this is two-sided attachment, not an obstacle. Target polygons use existing fan triangulation, so use convex faces or triangulate beforehand. Degenerate triangles are discarded; an entirely degenerate target is rejected.
- **Goal Gravity:** a vector acceleration applied during each relaxation step.
- **Goal Pin:** fixes the selected points at their captured positions. Standard point-group expressions work; `0 16` pins two endpoints. Empty selects all. Pins take precedence over conflicting goals.
- **Goal Solve:** deterministic replay from incoming positions. Steps are relaxation steps, not timeline frames. Iterations control projections per step; Timestep and Damping affect the resulting relaxation. Damping 1 removes previous velocity completely; smaller values retain inertia and may overshoot or slide on a surface.

Topology, vertex/primitive attributes and non-normal point attributes survive solving. Existing `N` is removed because deformation invalidates it; append a Normal node if needed. Details `goal_converged`, `goal_steps`, `goal_max_displacement`, `goal_max_residual` and `goal_residual_*` are readable in the geometry detail inspector. `goal_report` also contains the structured result. Residuals are absolute geometry-unit violations, and pins can make other goals impossible. A low displacement alone does not count as convergence.

Examples:

- **procedural_cable.json:** endpoint-pinned slack cable under gravity, followed by Sweep. Select `relax` to inspect the centerline/report.
- **procedural_fabric.json:** corner-pinned 9-by-9 point grid sagging under gravity. This is an edge-length membrane with no bending/shear material model.
- **procedural_packing.json:** 30 equal spheres distributed on a bounded plane, using separation plus surface attachment. Select `relax` to inspect particle positions and convergence; the final Copy to Points is display geometry.

Measured regression results for the supplied settings: packing converged in 7 steps with maximum separation residual approximately `1.67e-5`. Cable and fabric used their bounded step counts with maximum length residuals approximately `0.00333` and `0.00812`; they correctly report **not converged** at tolerance `0.0001`. These are useful relaxed forms, not an assertion of exact equilibrium. More iterations, a smaller timestep, weaker loading or a looser meaningful tolerance can change convergence.

Unsupported: material-calibrated elasticity, bending, torsion, volume preservation, triangle/edge collision, continuous collision detection, unequal particle radii/masses, collision obstacles, moving targets, rigid bodies and structural engineering verification. The solver is not a replacement for the independent RBD glue/pin/hinge/spring nodes. Surface attachment snapshots the upstream target for each cook.

Limits: 10,000 points overall; sphere separation 1,000 points; 32 goal nodes; 50,000 target triangles; 1,000 steps and 100 iterations. A conservative 50-million estimated correction-work budget rejects oversized cooks. Every pair is checked for sphere separation, so packing cost is quadratic. Graphs replay deterministically; there is no persistent timeline state or warm-start cache.

## Connected regions

Connect geometry to **Region Identify**, then **Region Transform**:

- Connectivity joins primitives that share any point. This reuses existing connectivity labels.
- Primitive mode makes each face its own region; adjacent faces will be split by Region Transform. This is convenient for paving and tiles.
- Attribute mode accepts an existing scalar integer primitive label, such as `class`. Equal labels are one logical region, even if disconnected. IDs use the engine's signed 32-bit integer attribute convention.

Region Identify writes `region_id`, deterministic `region_random`, `region_center` and random `Cd`. Random choices in both nodes are keyed by region ID and Seed. Reordering primitives with retained attribute IDs keeps their random choice; recomputing connectivity/primitive IDs after topology edits need not.

Region Transform independently extracts each region using the existing attribute-preserving subset operation. It optionally extrudes with connected PolyExtrude, then scales/rotates about the region's original mean point position and translates. Random parameters are uniform plus/minus ranges; random scale is a multiplicative uniform fraction. Extrusion happens before transformation and keeps the original back faces. A connected two-face patch gets boundary walls only, with no wall on the shared internal edge. Shared boundaries between separate regions are split. Polygon point/vertex/primitive attributes are carried through, `region_center` is updated, and unused points are omitted. Extrusion drops stale normals on all domains; append Normal to regenerate them.

Examples:

- **procedural_regions.json:** paving/weathered surface, using separated faces, gaps, tilt and varied extrusion.
- **procedural_stone_wall.json:** independently varied stone blocks, stood upright with a normal Transform node.
- **procedural_roof_tiles.json:** individual tiles with small thickness/angle variations, then a 30-degree roof pitch.
- **procedural_damaged_surface.json:** seeded missing panels using Blast, followed by individual tilt/displacement/thickness variation. It models panel damage, not physical fracture or crack propagation.

This is mesh-region processing, not image flood-fill. It does not segment by texture colour, fracture faces, resolve overlaps or guarantee self-intersection-free extrusion. Connected PolyExtrude's normal-based corner behavior is inherited; extreme distances on folded/nonmanifold patches can intersect. Use reasonable planar patches for paving/tiles and inspect more complex geometry. Region Transform requires polygon faces, supports up to 10,000 regions/1,000,000 input vertices, and applies a 50-million region-by-vertex work bound.

## Validation

`tests/test_procedural_systems.py` checks exact architectural component counts, roofs and input restrictions, deterministic rules, connected boundary extrusion, attribute preservation and normals, frozen-input safety, goal length convergence, pinned cable/fabric sag, packing separation, surface boundary projection, incompatible-goal residuals, topology mismatch, finite inputs and work limits, and deterministic example cooks. Parent integration additionally runs real Chromium editor tests for catalog discovery, example previews and parameter changes. No Blender host test or packaged EXE rebuild is implied by these checks.
