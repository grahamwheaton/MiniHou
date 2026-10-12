# MiniHou viewport toward Houdini workflows

## Reference and scope

The reference is the user-supplied Houdini PolyBevel viewport screenshot. Behavior was checked against SideFX's [Viewing the scene](https://www.sidefx.com/docs/houdini/basics/view.html) and [Houdini user interface](https://www.sidefx.com/docs/houdini/basics/ui.html). This is a staged workflow target, not a claim of complete feature parity or identical shortcuts.

## First tranche

- Component modes and applicable move/rotate/scale controls occupy a left tool strip.
- Shading, edges, points and normals occupy a right display strip; these operate the existing renderer.
- The selected node's first four scalar, boolean or enum parameters are editable above the viewport. They use the same edit queue, validation, undo and inspector synchronization as normal parameter controls. Vector, string and code parameters remain in the inspector. Inputs are updated in place so refreshes do not interrupt typing or scrubbing.
- A viewport state title and dismissible hints describe the actual MiniHou selection/navigation controls. Labels are deliberately textual for discoverability.
- Back, left and bottom views complement front, right and top. Projection remains an explicit perspective/orthographic toggle. Frame All is separate from Frame Selection.
- Shift/Ctrl selection passes through overlapping handles, so adding or subtracting components stays reliable in narrow views.
- Existing component operations, C pie menu, node handles, normals, orientation widget and timeline remain functional.

## Remaining gaps, in suggested order

1. Reference grid, world axes and configurable background; snapping to points, edges and construction planes.
2. Tool-specific states and complete parameter ordering rather than the generic first-four parameter strip; help sourced from node handle definitions.
3. Expanded selection: visible-only versus through selection, loops/rings, lasso/brush, grow/shrink, soft selection and named group visualization.
4. Smooth/flat/material/wireframe display presets, lights, textures, transparency, clipping and attribute visualizers.
5. Camera objects, camera linking/locking, viewport bookmarks, quad views and orthographic navigation UI.
6. Complete Houdini-style navigation/keymap option. Current hints describe MiniHou's existing bindings; Space still controls playback and is not a camera modifier.
7. Broader scene/object isolation, ghosting/template geometry and display flags, render region and simulation/volume visualizers.
8. Large-scene performance and native WebView2 interactive checks with production geometry.

No placeholder controls were added for these gaps. The existing bounded preview pipeline and rendering code were left unchanged.

## Validation

Real Chromium integration covers strip controls, contextual edits reaching the engine/inspector, focused typing surviving refresh, and axis navigation. The existing editor suite covers gizmos, selection, modeling, playback, undo, clipboard and live modifiers. Screenshot: `build/validation/viewport-tool-strips.png`. This does not substitute for native WebView2 testing or performance measurements on the user's exact scene.
