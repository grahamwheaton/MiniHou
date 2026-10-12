# Scene objects: MiniHou results as real Blender objects

Normally MiniHou's output is one mesh (shown through the `MiniHou_Proxy` object, or through a modifier). With a **Scene Objects**
node (`scene_objects`, in *Utility & I/O*) a result becomes **separate, ordinary Blender objects** you can then put modifiers,
materials, constraints and animation on, parent, or render like anything else in the scene.

```
grid -> scatter -> instance (objects "Tree Bush") -> scene_objects (prefix "Forest")
```

Each point of the input becomes one object in the collection `Forest`: a **linked duplicate** of the Blender object the point names
(so a thousand trees share one mesh), placed with the point's position, `rot` (Euler XYZ, as the instance node writes it) and
`scale` (or `pscale`). Points without an `instance` attribute use the node's `source` object. The node passes its input through
unchanged, so it can sit anywhere in a graph, next to the normal output, and the normal output (the single MiniHou mesh) keeps working.

| parameter | meaning |
|---|---|
| `mode` | `instances` (one linked duplicate per point) |
| `prefix` | names are `Prefix_0001`... |
| `collection` | Blender collection for the objects (created if missing; default: the prefix) |
| `source` | object to duplicate for points that do not say (no `instance` attribute) |
| `id_attrib` | integer point attribute that identifies each object (default `id`); without it the point number |
| `max_objects` | refuse (do not truncate) above this many points; default 2,000, at most 10,000 |

## Works with the live link and with modifiers

* **Live link** (graph open in the editor, Blender connected): the objects are made in the scene.
* **Modifier** (a MiniHou modifier on a mesh object; the graph starts with *Host Mesh* / `modifier_input`): the host mesh goes in,
  the graph runs, and the scene objects are made **parented to the host object**, with their transforms in its local space, so
  moving or scaling the host moves the scatter. The modifier's own mesh result still works as before.

## What survives, and what is never deleted

Working on the objects only makes sense if they stay put, so:

* Objects are found again by **custom properties** (`minihou_scene`, `minihou_id`), never by name. Rename one, save and reload the
  file, undo: it is still the same object. An object that is not marked is never touched, even if its name clashes.
* A cook only **creates, moves, shows and hides**. When there are fewer points, the surplus objects are **hidden** (viewport and
  render, `minihou_hidden`), not deleted; when there are more again, the same objects come back, with the modifiers, materials and
  colours you gave them. Removing the node, or the modifier, parks its objects the same way.
* Identity is the `id` point attribute when there is one (give points an `id` and a point keeps its object when others come and go),
  otherwise the point number: with point numbers, changing how many points there are can hand objects to other points.
* A changed point moves the same object; writes happen only when a transform really changed, so a recook that changes nothing
  touches nothing.
* If you change which object a point duplicates, the object's data is swapped in place (modifiers stay) when the kinds match
  (mesh to mesh). A **missing source** gets an empty as placeholder and a log line; it becomes the real thing once the source exists.
* The *MiniHou* panel shows how many scene objects there are and how many are parked, with **Delete parked** and **Delete all**.
* A quick pass of auto proxy sends no objects (only the full pass does), so the objects never flicker; a fixed proxy level (for
  example 10%) does send what it cooked, because you asked for it.

## Limits and not done

* Only `instances` mode. Splitting a result into one mesh object per piece (fracture pieces, say) is the next step.
* Linked duplicates share the source's data: per-object modifiers, yes; per-object edits to the mesh itself, no (make single user).
* Objects are created and updated from the add-on's timer, never from a handler, so a frame-change cook shows them a fraction of a
  second later. Up to 10,000 objects; Blender gets slow well before that, which is why the default limit is 2,000.
* The wire format is a new message type (`scene_nodes`, `scene_objects`, capability `scene_objects_v1`) with no protocol version
  bump; an older add-on ignores it.
