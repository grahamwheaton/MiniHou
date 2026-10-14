# Points nodes: MiniHou against Houdini's Points family

| Houdini node | MiniHou | Notes |
|---|---|---|
| Cluster Points | `cluster_points` | k-means / grid / distance clusters, `rest` input, centroid output. Not done: control attributes with weights, cluster filter / thresholds |
| Particle Trail | `particle_trail` | Stateless: a curve behind each point along its velocity (and gravity). Not done: frame-history trails, splits, ramps |
| Point Cloud Iso | `points_to_surface` | Existing node (voxel remesh of a point cloud) |
| Point Cloud Measure | `point_cloud_measure` | `neighbors`, `density`, `nn_dist`, `mean_dist`, optional `sample` input, PCA shape measures (`curvature`, `linear`, `planar`, `spherical`). Not done: eigenvectors output |
| Point Cloud Normal | `point_cloud_normal` | PCA plane fit; orient keep / outward / inward / viewpoint / up; `radius`, `reverse` |
| Point Cloud Reduce | `point_cloud_reduce` | distance (blue noise), count, percent, voxel (average / nearest centre / first). Not done: reference VDB input |
| Point Cloud Surface | `point_cloud_surface` | Moving-least-squares projection onto a cloud's surface. Not done: boundary types, max subdivisions |
| Point Generate | `point_generate` | Points from nothing (box, sphere, ball, line, circle, origin). Per-input-point generation is `point_replicate` |
| Point Jitter | `point_jitter` | `id_attrib` (stable per id), `use_pscale`, `mask` |
| Point Replicate | `point_replicate` | Count or per-point attribute, random offset, `copy` / `source`. Not done: shape orientation, noise, velocity stretch |
| Point Velocity | `point_velocity` | difference of two positions (matched by `id`), curl noise, direction. Not done: acceleration, angular velocity, object motion, conical noise |
| Point VOP | `wrangle` | The expression compiler covers what a point VOP does for most uses |
| Point Weld | `fuse` | Existing node |
| Points from Volume | `points_from_volume` | Closed polygon meshes (lattice, random, poisson, shell only). Not done: VDB sources, isovalue, dither |
| Scatter | `scatter` | `density_attrib`, `relax_iterations`, `output_id` added. Not done: force total count, per-prim counts, density texture |
| Scatter in Texture Mask | `scatter_in_texture_mask` | Mask from a pixel grid (`blender_image` / `image_file`), channel, gamma, threshold, invert, density scale. Not done: relax with constrain to mask |
| Spray Paint | - | An interactive viewport brush; use `attrib_paint` to paint a density attribute and `scatter` with `density_attrib` |

`tools/houdini_parity.py` regenerates the full per-parameter checklist in `HOUDINI_PARITY.md`.
