le:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/blender_link/connection.rs#L306-L309)), comparing full byte buffers (`!=`) instead of `Arc` pointers.
- **Packing blob & assembling frame (`encode_message`, lines 26–48):** [`blob`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/link_protocol.rs#L30-L37) starts empty (`Vec::new()` without `with_capacity`), copying each array's bytes and reallocating as it grows. Then [`frame`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/link_protocol.rs#L42-L46) allocates `16 + hb.len() + blob.len()` and copies both `hb` and `blob` into it before [`send_frame`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/blender_link/connection.rs#L30-L32) writes it to the socket.
- **Inbound decoding (`read_message` / `decode_payload`, lines 51–87):** Reads the entire frame into `payload`, clones each slice into `Arc<Vec<u8>>` ([line 83](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/link_protocol.rs#L83)), and then [`decode_attr`](file:///C:/Users/Grayw/Documents/Minihou_Rust/.claude/worktrees/native-ui-cont/rust/crates/minihou-app/src/link_protocol.rs#L136-L146) allocates a fresh `Vec<f32>`/`Vec<i32>`.

**2. Real Risk and Magnitude**

**Medium-High** for dense meshes. On every cook, even if only one parameter or small attribute changes, `geometry_arrays` serializes *all* geometry attributes into byte vectors just to diff them. For a 1M-polygon mesh (~50–100 MB of attributes), outbound transfer peaks at **3x–4x payload size** in transient allocations (`Arrays` + unreserved `blob` reallocations + `frame`), causing allocator churn and latency spikes during interactive Blender live-link sessions.

**3. Smallest Safe Fix**

1. Add a `write_message(writer: &mut impl Write, header: &Value, arrays: &Arrays)` function (which `encode_message` can call with a pre-sized `Vec<u8>`). Compute byte offsets from `a.bytes.len()` first, serialize the header, and stream the 16-byte prefix, header, and each `a.bytes` slice (+ up to 7 padding bytes) directly to the writer—eliminating both the `blob` and `frame` buffers.
2. Pre-allocate byte vectors (`Vec::with_capacity(v.len() * 4)`) in `geometry_arrays` / `i32_array`.

**4. Proving Test**

A unit test in `link_protocol::tests` asserting byte-for-byte identity between `encode_message(&header, &arrays)` and `write_message(&mut buf, &header, &arrays)` across multi-array unaligned payloads, paired with a benchmark/allocation test on a 100k-point `Geometry` verifying reduced peak heap allocation.

**5. What Could Not Be Verified**

Actual runtime transfer latency, TCP socket buffering overhead, and peak RSS memory impact during live Blender sessions, as no benchmarks or live Blender processes were executed.

