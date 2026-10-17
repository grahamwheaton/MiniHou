# Sketch tools

A practical plane-based drawing toolset, not a complete SketchUp implementation. All committed drawing uses the existing editable **curve** node. Closed rectangle, polygon and circle outlines are single polygon faces; line and pen are open polylines. They can feed existing extrusion, sweep and other modelling nodes. Drawing does not automatically split or weld an existing solid.

## Drawing

Open **Sketch & Measure** above the viewport. Choose a drawing plane (XY, XZ or YZ), then Line, Pen, Polygon, Rectangle or Circle. Left-click points. Line uses two endpoints; rectangle uses opposite corners; circle uses centre and radius with 32 segments by default. Pen and polygon accept multiple points; Enter finishes, or click the starting point to close a polygon. Escape or right-click cancels without adding a node. Middle mouse remains available for navigation. Backspace removes the last point or typed digit.

X/Y/Z locks the next segment to an axis in the selected drawing plane. An axis perpendicular to that plane is rejected. Type a positive length and press Enter to use an exact segment length/radius along the current pointer direction. For a rectangle this length is its diagonal, not separate width and height. Inputs elsewhere in the UI retain normal typing.

Native editor: inferencing now goes through the shared engine described in docs/RUST_INFERENCE.md (10 px default tolerance, 3D axes/parallel/perpendicular/extension, true 3D intersections, reference tracking, face-plane drawing for Line/Polyline). The paragraph below describes the original web sketch tool. Endpoint and edge-midpoint inferencing uses displayed geometry within 12 screen pixels and within the drawing plane. Current sketch endpoints also attract the cursor. Without a nearby endpoint/midpoint, grid and in-plane axis inference apply; the feedback line identifies the current inference. Ctrl temporarily inverts snapping. Grid spacing is in scene units; zero disables grid rounding. The drawing plane origin defaults to world zero (the API can provide another origin).

## Tape and dimensions

Tape picks two points and leaves a world-space line and distance label, updated with the viewport camera. Distances use scene units, displayed to four decimals. **Measurements are session-only overlays**, not graph nodes or saved dimensions. Clear Measurements removes them. There are no associative dimensions, unit conversion, dimension constraints or tape-based model rescaling.

## Limits and validation

(Web tool limits; the native editor's line tools draw on a picked face plane and support parallel inference, locks and hidden-geometry filtering, see docs/RUST_INFERENCE.md.) Drawing is on principal planes, not arbitrary picked faces. No automatic face intersection/healing, arc tool, offset inference, parallel inference, locked inference references, hidden-geometry filtering or model topology editing is claimed. Closed outlines reject duplicate points, zero area, nonplanarity and ordinary crossing segments; complex self-touching outlines are outside this basic tool. Circle polygons have bounded 3–256 segments through the API. No enormous realised-copy fixtures are used.

The numerical tests execute the shipped JavaScript and cook its output with the existing Python curve node. The browser test exercises actual pointer/keyboard handlers, exact numeric length, rectangle creation, tape labels, cancellation and text-input preservation using a deterministic projection fixture. Host Blender UI validation is separate.

## Integration API

Load `sketch-tool.js`; construct `MiniSketchTool(pv, canvas, async create)` where `create` receives `{type:'curve', params:{points:'x,y,z ...', closed:true|false}}` and returns only after the node creation succeeds. A failure stays visible in tool feedback. Place `tool.panel()` in modelling UI or invoke `tool.start(mode, options)` from commands. Compose `tool.draw()` into the viewport overlay redraw callback. Guard other editor tool shortcuts with `tool.active()`. `tool.cancel()` changes no graph data. `tool.clearDimensions()` and `tool.dimensions()` manage transient measurements.
