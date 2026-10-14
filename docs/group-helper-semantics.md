# Group helper semantics

`group_by_color`, `group_by_normal`, and `group_by_proximity` write named groups on points, primitives, edges, or vertices. Point, primitive, and vertex groups use integer `group_<name>` attributes. Edge groups use the derived edge representation and `edge_group_<name>` on outgoing vertex rows. The input geometry and all unrelated attributes are preserved.

Each helper evaluates a match for every element of the chosen domain, applies **Invert** if enabled, then intersects the result with **Base Group** if one is given. **Combine** then merges that result with an existing group of the same name: replace, union, intersect, or subtract. An empty Base Group means all elements. Missing literal group names fail through the normal group-pattern parser.

## Group by Cd Colour

`attribute` defaults to `Cd`. The node compares the first three numeric components with `color` using Euclidean RGB distance. An element matches when the distance is at most `tolerance`, which must be nonnegative. An empty domain creates an empty group. On a nonempty domain, a missing, malformed, or nonfinite colour attribute produces a clear error.

Colour lookup prefers the attribute on the chosen domain. Vertex colours can fall back to point colours; point colours can fall back to the mean of incident vertex colours. Primitive colours fall back to the mean of their corner points. Edge colours prefer the mean of their outgoing vertex colours and otherwise use the mean of their two endpoint colours. The node does not infer primitive colour for points or edges.

## Group by Normal

`direction` is normalized; a zero direction is an error. `angle` is the maximum facing angle in degrees, from 0 to 180. **Both Sides** also accepts the opposite direction. Zero normals never match, including at 180 degrees.

Existing `N` on the selected domain takes precedence and is normalized. Vertices can fall back to point `N`; edges average unit vertex `N`, then point `N`, when available. Without an applicable `N`, closed primitives use geometric face normals, vertices use their face normal, points use the area-weighted normals of incident faces, and edges average the unit normals of incident faces. Open curves and degenerate faces have no computed facing normal. Supplied nonfinite or malformed `N` is an error.

## Group by Proximity

Each point and vertex uses its point position. Each primitive uses its centroid, and each edge uses its midpoint. These representative positions determine membership; a large primitive can intersect a target while its centroid is outside the distance limit.

`target_mode=position` measures to `position`. `points` measures to the nearest point of the connected `target` geometry. `surface` measures to actual polygon triangles and open-curve segments. Concave polygons are ear-clipped, so a point in a notch does not match merely because it lies inside a triangle of a simple fan. Zero-area triangles are ignored; a connected target with no usable surface produces an error. `auto` uses the target surface when it has nondegenerate faces or segments, its points when it has no usable surface, and `position` when no target is connected. An empty connected target matches nothing. The threshold is inclusive, with a small tolerance for float32 stored positions. Point searches use the project spatial index; curve searches use an exact segment bounding-box hierarchy without requiring SciPy.
