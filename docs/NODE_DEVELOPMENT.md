# Rust node packages

A standard geometry node lives in one folder under `rust/crates/minihou-core/src/nodes/packages/`. Adding that folder requires no edits to the core registry, Tab menu, or shelf catalogue. Cargo's core build script discovers folders with `node.rs`, sorts them by `id`, and generates the package registry and display catalogue. The legacy nodes remain in their existing modules and catalogue entries.

## Folder layout

```text
nodes/packages/my_offset/
  node.rs       # NODE specification and cook function
  node.toml     # identity and menu metadata
  tests.rs      # focused unit tests, included from node.rs
```

Use a unique lowercase `snake_case` folder and node ID. The folder and ID may differ, but matching them makes review easier. Every package folder must have both `node.rs` and `node.toml`.

## Metadata

All five fields are required, nonempty quoted strings. `category` must match an existing Tab and shelf category exactly. `keywords` is a space separated search phrase. Package tools are appended after legacy tools in their category. The shelf uses a generic icon for package tools in Stage 1.

```toml
id = "my_offset"
name = "My Offset"
category = "Modify"
description = "Move points along Y."
keywords = "move offset translate y"
```

The build fails for missing files, missing or invalid metadata, unknown categories, or duplicate IDs among packages. At registry initialization, an ID that collides with a legacy node or disagrees with `NODE.name` also fails clearly. Keep `NODE.name` identical to `id`.

## Rust example

```rust
use crate::{error::{Error, Result}, geometry::Geometry,
    nodes::{InputSpec, Inputs, NodeSpec}, params::Params};

pub(super) const NODE: NodeSpec = NodeSpec {
    name: "my_offset",
    inputs: &[InputSpec { name: "geo", multi: false, optional: false }],
    params: crate::params!("amount": 1.0; "enabled": true),
    cook,
};

fn cook(inputs: &Inputs, p: &Params) -> Result<Geometry> {
    let source = inputs[0].first().ok_or_else(|| Error::new("geo is not connected"))?;
    let mut out = (**source).clone();
    if p.b("enabled")? {
        let amount = p.f("amount")? as f32;
        let mut points = out.p().to_vec();
        for xyz in points.chunks_exact_mut(3) { xyz[1] += amount; }
        out.set_p(points);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
```

Each input is one `InputSpec`; `multi` accepts several connections and `optional` permits none. Cook receives one vector per input and returns one `Geometry` output. Clone input geometry before changing attributes. Use `Params::resolve` in unit tests to exercise defaults and supplied values.

## Parameters and tests

`crate::params!` declares defaults. JSON defaults may be numbers, booleans, strings, or arrays such as `[0.0, 0.0, 0.0]`. Read them with `p.f`, `p.i`, `p.b`, `p.s`, or `p.v3`; enum strings use `p.e(name, choices)`. The macro supports `unsupported` after a default for a parameter that must reject nondefault values. Test enabled and disabled paths, missing input, and representative geometry with a focused test in `tests.rs`.

From the worktree root, for example:

```powershell
$env:CARGO_BUILD_JOBS='2'
$env:CARGO_TARGET_DIR='rust/target_arch'
cargo test --manifest-path rust/Cargo.toml -p minihou-core my_offset --lib
```

## Registration and limits

Only nodes in `nodes/packages/*/node.rs` join the generated list. The generated list is sorted by ID and merged after the legacy registry. The metadata is used by the native Tab search and shelf; existing entries keep their order and labels. This stage supplies one geometry output per node, existing parameter value types, and a generic shelf icon. A new category, custom icon, extra output socket, or nonstandard UI behavior needs a separate shared UI change. The web editor and Python registry do not consume this Rust package catalogue.

## Parallel work

Give each agent a distinct package folder and ID. Agents should edit only their own `node.rs`, `node.toml`, and `tests.rs`, plus a separate documentation file when needed. Avoid simultaneous edits to `nodes/mod.rs`, the core build script, and native catalogue files. Run focused tests with separate target directories if agents test concurrently. Integrate package commits one at a time and check ID collisions after merging.

## Copyable agent instruction

```text
Add a standard Rust geometry node as one self-registering package in
rust/crates/minihou-core/src/nodes/packages/<unique_snake_case_id>/.
Create node.rs, node.toml, and tests.rs. Define pub(super) const NODE with
NODE.name equal to node.toml id. Supply name, an existing category,
description, and keywords. Implement the cook function and focused tests.
Do not edit shared registry or native menu/shelf files. Use
CARGO_BUILD_JOBS=2 and CARGO_TARGET_DIR=rust/target_arch; run only the
targeted core tests for this node. Report any UI, multi-output, or custom
icon needs separately.
```
