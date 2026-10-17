# TODO: Image-based procedural nodes (parked)

Status: parked by the user (2026-10-17). Not started. Build order when resumed: Image Density + Scatter density option, Image Sample, Image Mask, Image Displace, Image to Curves.

---

Work only in a git worktree: git worktree add -b opus/image-nodes ..\opus-image HEAD, then work in that worktree. Rust core (rust/crates/minihou-core) and native app (rust/crates/minihou-app). Read only what you need; print short output. Set CARGO_BUILD_JOBS=2 and CARGO_TARGET_DIR=rust/target_image. Run only targeted tests; no workspace tests, no release build, no push. Commit in small commits ending with: Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>. Keep the keymap table test green; regenerate param meta when params change (python tools/dump_rust_param_meta.py; every param needs a doc string). Reuse the existing image_file node and the Blender image path; do not duplicate image decoding. Do not change unrelated engine code.

BUILD ORDER: (a) Image Density, plus a Scatter density option; (b) Image Sample; (c) Image Mask; (d) Image Displace; (e) Image to Curves. Stop after any step if a blocker needs the user; report it.

(a) IMAGE DENSITY: new node `image_density`. Input: geometry with a colour attribute Cd (as produced by image_file). Params: channel (luminance, red, green, blue, alpha), invert (default true: dark = high density), contrast (1.0), brightness/offset (0.0), threshold (optional hard cutoff, 0 = off), output attribute name (default `density`). Output: the same geometry plus a per-point float `density` in 0..1. Luminance uses Rec.709 weights. Handle greyscale images (R=G=B), missing channels and invalid values (NaN -> 0, clamp). Preserve all attributes. SCATTER DENSITY OPTION: extend the existing scatter node (find it in nodes/) with an optional `density_attribute` param (empty = off). When set, scatter weights point placement by the interpolated density across each surface; with constant density the result must match the unweighted scatter exactly (no bias). Tests: luminance of black and white, invert, contrast, threshold, channel selection, missing Cd, NaN, preservation of other attributes; scatter with constant density equals unweighted; scatter with dark-half density puts more points in the dark half (count check with a seed). Demonstration: add a test graph JSON under rust/crates/minihou-app/src/test_data (image_file -> image_density -> scatter -> copy_to_points) and a test that cooks it.

(b) IMAGE SAMPLE: node `image_sample`: samples a colour or brightness at each point's UV (uv attribute) from the image attached to the geometry (per image_file); output attribute `sample_colour` (vec3) and `sample_value` (float). Tests: UV corners sample the corresponding pixels with the row-origin convention documented in docs/RUST_TEXTURE_RENDERING.md; out-of-range UVs use the wrap mode.

(c) IMAGE MASK: node `image_mask`: creates a point group (or face group) from image brightness/colour/alpha with a threshold and an invert toggle; output group name param. Tests: group membership on a 2x2 known image.

(d) IMAGE DISPLACE: node `image_displace`: moves points along an axis by image brightness at their UV: params axis (normal, x, y, z), direction (positive/negative), strength (float), midlevel (0.5), channel. Tests: zero strength is identity; brightness 1 with strength s moves by s along the axis.

(e) IMAGE TO CURVES: node `image_to_curves`: traces black/white boundaries of a binary image (threshold) into polyline curves with simplification (tolerance) and optional smoothing. Only if (a)-(d) are finished and time remains; otherwise document it as a follow-up.

ACCEPTANCE: load a black-and-white PNG, generate points concentrated in its dark areas with image_density -> scatter, and changing density interactively cooks without errors (test the cook path with two different density settings).

Report in 6 lines: what each node does, params, tests, the demo graph path, what was not built and why, and how to try the acceptance flow.

---

# Queued node-editor fix (not the image work)

- **Tab with nothing selected places the new node at the mouse position** in the node editor (not at the default spot). Attach to the next node-editor fix; check the existing Tab-menu insertion path (ui/network.rs, network_menu.rs) so a node created with a selection still follows its current rule.

---

# Queued feature: Controller node

A "Controller" node that acts as a control panel for top-level driver parameters.

- **Add parameters of any type** (float, int, bool, vector, colour, string, menu, and so on) from the node's own panel.
- **Drag to rearrange** parameters, and **group them into sections** that can be named and reordered.
- **Purpose:** one place to hold the top-level drivers that other nodes reference, so a whole network can be tuned from one panel.
- **Link to existing work:** it should reuse the Add Input machinery (dynamic typed inputs) and the value-linking references (`ch(...)`), so other nodes can link to its parameters.
- **Sections are saved in the graph file** (decided by the user).
- **Parameters can optionally be exposed to Blender** (decided by the user): each Controller parameter has an 'Expose to Blender' flag. Exposed parameters appear in the Blender modifier panel, inside a section that matches the Controller's section, and reuse the Add Input socket machinery. Unexposed parameters stay inside MiniHou.
