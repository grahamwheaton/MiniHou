# Inset, Outset and Push/Pull

Choose faces in the viewport, then use the face action menu, Polygon shelf or node menu. Inset and Outset provide an amount square you can drag; PolyExtrude also has an inset amount handle alongside its distance arrow. Push/Pull has a normal distance arrow. Positive distance adds solid; negative distance cuts into it and can cut through the opposite wall.

Load `examples/graphs/pushpull_hole.json` for a small through-hole example. Change hole distance from -3 to -0.4 for a pocket. Use a box face directly with positive distance for outward extension. The named `extrudeFront` group from Inset identifies the inner face.

Push/Pull currently accepts exactly one convex planar face on a closed outward-wound solid. Open sheets, multiple faces, concave or nonplanar selected faces produce actionable errors. The implementation uses the existing boolean kernel, with up to four deterministic solve frames and closed-edge, winding, volume and polygon-crossing checks. This is not a general proof against all possible mesh self-intersections. Numerically difficult solids may still be rejected. Input is limited to 20,000 faces / 50,000 points and polygons to 256 corners; use small modelling solids. It is not full Blender Extrude Manifold or SketchUp parity.

Inset/Outset use the existing radial PolyExtrude inset convention, not constant perpendicular edge offsets. They do not resolve overlap or collisions with neighbouring faces. Existing graphs and ordinary PolyExtrude remain compatible.

Tests check volume, edge twins, polygon crossings and Euler characteristic for a real through tunnel, all six rotated box faces, input preservation and rejection of invalid boolean results. Chromium tests exercise sketch creation in the full editor and the inset amount handle. No enormous realised-copy workload or driver-fix claim is involved.
