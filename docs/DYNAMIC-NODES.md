# Dynamic Nodes

Dynamic Nodes is gentle, temporary feedback around a node being dragged. Nearby
unselected nodes move away by at most 12 graph pixels and spring back when the
drag ends. It is not an automatic layout algorithm. Selected nodes and the node
being dragged remain fixed to their normal positions. Saved node positions,
selection, graph connections and undo records are not modified by this effect.

The node-editor preference defaults on and can be switched off. The effect also
turns off for the operating system's reduced-motion preference, during editor
operations which pause it, and for graphs with more than 500 nodes. At most the
nearest 48 nodes within 220 graph pixels move. Animation stops when settled;
there is no continuously running idle physics simulation.

## Integration contract

`MiniDynamicNodes({onChange})` owns only transient offsets. `onChange` receives a
Map from node ID to `[dx,dy]`. Apply the same offset to nodes and wire endpoints.
Call `update({nodes, selected, dragging, pointer, enabled, paused})` from actual
interaction changes, not the rendering callback. `nodes` is an array of
`{id,x,y}` centers in graph coordinates; selected and dragging are ID arrays;
pointer is a graph-coordinate pair. Call `update({dragging:[]})` on release.
`offset(id)` returns the current pair; `reset()` clears feedback immediately.
`dispose()` releases animation and the reduced-motion listener.

Manual drag coordinates, graph serialization, undo and auto-layout must use
the underlying graph position. Do not write the feedback offsets into the graph.
Pause feedback when editing text or running competing layout/selection tools.

The numerical tests exercise the shipped JavaScript, including default-on,
maximum displacement, pinning, exact settling, reduced motion, node limits and
bounded animation scheduling. Editor integration requires separate browser tests.
