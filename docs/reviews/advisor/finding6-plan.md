# Finding 6: Blender transport measurement plan

Status: measurement plan only. The MHL1 protocol and implementation are unchanged in this pass. The advisor's 3–4× transient-memory estimate is a hypothesis, not a measured result.

## Receiver reference measured in this pass

On this Windows host with Blender 5.2.0 LTS, `python tools/launch_blender.py --bench -- --mode raw --shape mesh --points 100000 --iterations 5` completed with 99,856 points and 99,225 faces. The first 3.2 MB frame took 2.5 ms for transport and 20.2 ms for Blender-side apply, including 17.3 ms for topology. Five later position-only frames were 1.2 MB each; median transport was 0.77 ms and median Blender-side apply was 0.09 ms (0.9 ms median full path). This is a short Python-sender/Blender-receiver reference. It does **not** time Rust `geometry_arrays`, diffing, `encode_message`, or Rust peak memory, and five samples are too few for a reliable p95.

## Path to measure

For a native Rust-to-Blender update, time and count allocations separately at `link_protocol::geometry_arrays`, the diff in `blender_link/connection.rs` (around the `arrays`/`changed` construction), `link_protocol::encode_message`, `send_frame`/`TcpStream::write_all`, and Blender receive/decode/`mesh_writer.apply_update`. Record the array payload bytes, changed bytes, blob bytes, frame bytes, and the number of changed attributes. Include the first full topology frame and subsequent position-only and no-change updates. The diff currently serializes all attributes before it decides which arrays changed; `encode_message` builds a blob and then copies it into a frame.

## Reproducible runs

1. Use the same Windows machine, Blender version, Rust build profile, and loopback connection for every run. Record CPU, RAM, Rust commit, Blender version, graph/mesh fixture, and whether another Blender process is open. Warm up five updates, then collect at least 30 samples for each case.
2. Test point clouds and quad meshes at 100k and 1M points. For each size, run a first full update, repeated position-only updates, a one-small-attribute change, and no-change updates. Include a dense mesh with edge/UV/color attributes to expose copy amplification. Keep the source geometry resident while measuring transient allocations.
3. Add temporary local profiling outside the committed protocol: wall-clock spans around the stages above, an allocation profiler or peak heap counter for Rust, and per-process peak working set/RSS for Rust and Blender. Sample memory fast enough to catch short-lived blob/frame peaks; process RSS alone may miss them. Measure Blender decode and mesh-write spans separately from socket receive.
4. Report median, p95 and maximum latency per stage and end to end, bytes sent, peak additional Rust heap/RSS, Blender peak RSS, and allocations relative to payload size. Repeat each case three times. Record both successful and failed/disconnected sends.
5. As a receiver reference, run `python tools/launch_blender.py --bench -- --mode raw --shape mesh --points 1000000 --iterations 30` and the matching `--shape points` run. These exercise the Python sender and Blender receiver, so label them separately from the native Rust transport results. The existing `--mode engine` bench also uses the Python engine and must not be reported as a Rust sender measurement.

## Decision gate for later work

First compare array construction and diff cost against frame assembly and socket write cost. If frame assembly dominates, prototype pre-sized/streamed or vectored writes while preserving byte-for-byte MHL1 frames, and compare the same fixtures. If serialization of unchanged arrays dominates, measure a change-detection approach before altering the protocol. Consider shared memory only if the measured end-to-end improvement would justify its cross-process lifetime and fallback complexity. Keep compatibility tests and Blender live-link verification separate from microbenchmarks.

No live native Rust-to-Blender latency or peak-memory numbers were collected in this review pass; those runs require a dedicated instrumented session.
