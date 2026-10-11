// MiniHou 3-D preview: a small WebGL2 viewer for the geometry of a node. Plain JS, no libraries.
// Data comes from POST /api/preview (see minihou/preview.py for the wire format). Z is up, like Blender.
"use strict";
window.MiniHouPreview = (() => {
  const VS = `#version 300 es
  in vec3 aPos; in vec3 aCol;
  uniform mat4 uMV, uP; uniform float uPointSize, uBias;
  out vec3 vView; out vec3 vCol;
  void main() { vec4 v = uMV * vec4(aPos, 1.0); vView = v.xyz; vCol = aCol; gl_Position = uP * v;
                gl_Position.z -= uBias * gl_Position.w; gl_PointSize = uPointSize; }`;
  // selected edges: each edge is a quad that is widened in screen space, so selections read as thick lines
  const VS_EDGE = `#version 300 es
  in vec3 aA; in vec3 aB; in float aT; in float aS;
  uniform mat4 uMV, uP; uniform vec2 uViewport; uniform float uWidth, uBias;
  void main() {
    vec4 ca = uP * uMV * vec4(aA, 1.0), cb = uP * uMV * vec4(aB, 1.0);
    vec2 d = (cb.xy / max(cb.w, 1e-4) - ca.xy / max(ca.w, 1e-4)) * uViewport;
    float l = length(d); d = l > 1e-6 ? d / l : vec2(1.0, 0.0);
    vec4 c = mix(ca, cb, aT);
    c.xy += vec2(-d.y, d.x) * aS * uWidth * 2.0 / uViewport * c.w;
    c.z -= uBias * c.w;
    gl_Position = c;
  }`;
  const FS_COLOR = `#version 300 es
  precision highp float;
  uniform vec4 uColor; out vec4 o;
  void main() { o = uColor; }`;
  const FS_MESH = `#version 300 es
  precision highp float;
  in vec3 vView; in vec3 vCol; uniform vec3 uBase; uniform float uUseCol; out vec4 o;
  void main() {
    vec3 n = normalize(cross(dFdx(vView), dFdy(vView)));
    float d = max(n.z, 0.0);                                    // headlight: view space looks down -Z, n faces the viewer
    vec3 base = mix(uBase, vCol, uUseCol);
    vec3 col = base * (0.28 + 0.72 * d) + vec3(0.10) * pow(1.0 - d, 3.0);
    if (!gl_FrontFacing) col = mix(col, vec3(0.95, 0.45, 0.18), 0.55);   // back faces: flipped normals show up orange
    o = vec4(col, 1.0);
  }`;
  const FS_FLAT = `#version 300 es
  precision highp float;
  in vec3 vView; in vec3 vCol; uniform vec3 uBase; uniform float uUseCol; uniform float uAlpha; out vec4 o;
  void main() { o = vec4(mix(uBase, vCol, uUseCol), uAlpha); }`;

  // ---- tiny column-major mat4 helpers
  const ident = () => new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
  function perspective(fovy, aspect, near, far) {
    const f = 1 / Math.tan(fovy / 2), m = new Float32Array(16);
    m[0] = f / aspect; m[5] = f; m[10] = (far + near) / (near - far); m[11] = -1; m[14] = (2 * far * near) / (near - far);
    return m;
  }
  function lookAt(eye, c, up) {
    const z = norm(sub(eye, c)), x = norm(cross(up, z)), y = cross(z, x), m = ident();
    m[0] = x[0]; m[4] = x[1]; m[8] = x[2]; m[1] = y[0]; m[5] = y[1]; m[9] = y[2]; m[2] = z[0]; m[6] = z[1]; m[10] = z[2];
    m[12] = -dot(x, eye); m[13] = -dot(y, eye); m[14] = -dot(z, eye);
    return m;
  }
  const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };

  function parse(buf) {
    const dv = new DataView(buf);
    if (buf.byteLength >= 8 && dv.getUint32(0, false) === 0x4d485056) {              // "MHPV"
      const hl = dv.getUint32(4, true);
      const head = JSON.parse(new TextDecoder().decode(new Uint8Array(buf, 8, hl)));
      const base = 8 + hl, arrays = {};
      for (const t of head.arrays) {
        const C = t.dtype === "float32" ? Float32Array : Uint32Array;
        arrays[t.name] = new C(buf, base + t.offset, t.count);
      }
      return { head, arrays };
    }
    return { head: JSON.parse(new TextDecoder().decode(new Uint8Array(buf))), arrays: null };    // an error, as JSON
  }

  function create(canvas, onMessage) {
    const gl = canvas.getContext("webgl2", { antialias: true, alpha: false });
    const api = { ok: !!gl, tris: 0, edges: 0, points: 0, draws: 0, node: null };
    if (!gl) { if (onMessage) onMessage("WebGL2 is not available in this browser."); return api; }
    const prog = (vs, fs, attribs, uniforms) => {
      const p = gl.createProgram();
      for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fs]]) {
        const sh = gl.createShader(type); gl.shaderSource(sh, src); gl.compileShader(sh);
        if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(sh));
        gl.attachShader(p, sh);
      }
      attribs.forEach((a, i) => gl.bindAttribLocation(p, i, a));
      gl.linkProgram(p);
      if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
      const u = {};
      for (const n of uniforms) u[n] = gl.getUniformLocation(p, n);
      return { p, u };
    };
    const MESH_U = ["uMV", "uP", "uPointSize", "uBase", "uUseCol", "uAlpha", "uBias"];
    let mesh, flat, edgeProg, colorProg;
    try {
      mesh = prog(VS, FS_MESH, ["aPos", "aCol"], MESH_U);
      flat = prog(VS, FS_FLAT, ["aPos", "aCol"], MESH_U);
      edgeProg = prog(VS_EDGE, FS_COLOR, ["aA", "aB", "aT", "aS"], ["uMV", "uP", "uViewport", "uWidth", "uBias", "uColor"]);
      colorProg = prog(VS, FS_COLOR, ["aPos", "aCol"], ["uMV", "uP", "uPointSize", "uBias", "uColor"]);
    } catch (e) { if (onMessage) onMessage("3-D preview unavailable: " + e.message); api.ok = false; return api; }

    const vao = gl.createVertexArray(), posBuf = gl.createBuffer(), colBuf = gl.createBuffer();
    const triBuf = gl.createBuffer(), edgeBuf = gl.createBuffer();
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, posBuf); gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, colBuf); gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    // selection overlay buffers: selected points (a position buffer), edge quads, and the triangles of selected faces
    const ptVao = gl.createVertexArray(), ptBuf = gl.createBuffer();
    gl.bindVertexArray(ptVao); gl.bindBuffer(gl.ARRAY_BUFFER, ptBuf); gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0); gl.bindVertexArray(null);
    const sedVao = gl.createVertexArray(), sedBuf = gl.createBuffer(), sedIdx = gl.createBuffer();
    gl.bindVertexArray(sedVao); gl.bindBuffer(gl.ARRAY_BUFFER, sedBuf);
    for (const [loc, n, off] of [[0, 3, 0], [1, 3, 12], [2, 1, 24], [3, 1, 28]]) {
      gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, n, gl.FLOAT, false, 32, off);
    }
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, sedIdx); gl.bindVertexArray(null);
    const faceIdx = gl.createBuffer();
    const ov = { points: 0, edgeIdx: 0, faceIdx: 0 };

    const view = { yaw: -0.6, pitch: 0.45, dist: 5, target: [0, 0, 0] };
    const opt = { shaded: true, edges: true, points: false };
    let data = { npoints: 0, ntris: 0, nedges: 0, hasCd: false, bounds: [[-1, -1, -1], [1, 1, 1]] };
    let raw = null;                                                  // the geometry's typed arrays, for picking
    let raf = 0;

    function upload(head, arrays) {
      gl.bindBuffer(gl.ARRAY_BUFFER, posBuf); gl.bufferData(gl.ARRAY_BUFFER, arrays.P, gl.STATIC_DRAW);
      data.hasCd = !!arrays.Cd;
      if (arrays.Cd) { gl.bindBuffer(gl.ARRAY_BUFFER, colBuf); gl.bufferData(gl.ARRAY_BUFFER, arrays.Cd, gl.STATIC_DRAW); }
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, triBuf); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, arrays.tri, gl.STATIC_DRAW);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, edgeBuf); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, arrays.edge, gl.STATIC_DRAW);
      data.npoints = arrays.P.length / 3; data.ntris = arrays.tri.length / 3; data.nedges = arrays.edge.length / 2;
      data.bounds = head.bounds;
      raw = { P: arrays.P, tri: arrays.tri, edge: arrays.edge, triPrim: arrays.tri_prim || new Uint32Array(arrays.tri.length / 3) };
      Object.assign(api, { tris: data.ntris, edges: data.nedges, points: data.npoints });
    }
    function frameAll() {
      const [lo, hi] = data.bounds;
      view.target = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
      const r = Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2, 1e-3);
      view.dist = r / Math.tan(Math.PI / 8) * 1.15;
      schedule();
    }
    function resize() {
      const dpr = window.devicePixelRatio || 1, w = Math.max(1, Math.round(canvas.clientWidth * dpr)), h = Math.max(1, Math.round(canvas.clientHeight * dpr));
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    }
    function camera() {                                              // matrices for the current view; sizes in CSS pixels
      const [lo, hi] = data.bounds, r = Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2, 1e-3);
      const cp = Math.cos(view.pitch), eye = [view.target[0] + view.dist * cp * Math.cos(view.yaw),
        view.target[1] + view.dist * cp * Math.sin(view.yaw), view.target[2] + view.dist * Math.sin(view.pitch)];
      const w = Math.max(canvas.clientWidth, 1), h = Math.max(canvas.clientHeight, 1), near = Math.max(view.dist * 0.01, r * 1e-3);
      return { MV: lookAt(eye, view.target, [0, 0, 1]), P: perspective(Math.PI / 4, w / h, near, view.dist + r * 4), eye, w, h, near };
    }

    // ---------------------------------------------------------------- selection (points, edges, faces)
    const sel = { mode: "view", point: new Set(), edge: new Map(), face: new Set() };
    const edgeKey = (a, b) => (a < b ? a + "-" + b : b + "-" + a);
    const summary = () => ({ mode: sel.mode, points: [...sel.point].sort((a, b) => a - b),
      edges: [...sel.edge.values()].sort((x, y) => x[0] - y[0] || x[1] - y[1]), faces: [...sel.face].sort((a, b) => a - b) });
    function toScreen(cam, x, y, z) {                                // [sx, sy, depth] in CSS pixels, null behind the camera
      const m = cam.MV, vx = m[0] * x + m[4] * y + m[8] * z + m[12], vy = m[1] * x + m[5] * y + m[9] * z + m[13];
      const vz = m[2] * x + m[6] * y + m[10] * z + m[14];
      if (vz > -cam.near) return null;
      return [(cam.P[0] * vx / -vz * 0.5 + 0.5) * cam.w, (0.5 - cam.P[5] * vy / -vz * 0.5) * cam.h, -vz];
    }
    function rayAt(cam, sx, sy) {                                    // unit direction (world) through a pixel
      const dx = (2 * sx / cam.w - 1) / cam.P[0], dy = (1 - 2 * sy / cam.h) / cam.P[5], m = cam.MV;
      return norm([dx * m[0] + dy * m[1] - m[2], dx * m[4] + dy * m[5] - m[6], dx * m[8] + dy * m[9] - m[10]]);
    }
    function raycast(o, d, tMax, anyHit) {                           // nearest triangle hit (Moller-Trumbore): {t, tri} or null
      const P = raw.P, T = raw.tri;
      let best = null, bt = tMax === undefined ? Infinity : tMax;
      for (let i = 0, n = T.length / 3; i < n; i++) {
        const a = T[3 * i] * 3, b = T[3 * i + 1] * 3, c = T[3 * i + 2] * 3;
        const e1x = P[b] - P[a], e1y = P[b + 1] - P[a + 1], e1z = P[b + 2] - P[a + 2];
        const e2x = P[c] - P[a], e2y = P[c + 1] - P[a + 1], e2z = P[c + 2] - P[a + 2];
        const px = d[1] * e2z - d[2] * e2y, py = d[2] * e2x - d[0] * e2z, pz = d[0] * e2y - d[1] * e2x;
        const det = e1x * px + e1y * py + e1z * pz;
        if (det > -1e-14 && det < 1e-14) continue;
        const inv = 1 / det, tx = o[0] - P[a], ty = o[1] - P[a + 1], tz = o[2] - P[a + 2];
        const u = (tx * px + ty * py + tz * pz) * inv;
        if (u < 0 || u > 1) continue;
        const qx = ty * e1z - tz * e1y, qy = tz * e1x - tx * e1z, qz = tx * e1y - ty * e1x;
        const v = (d[0] * qx + d[1] * qy + d[2] * qz) * inv;
        if (v < 0 || u + v > 1) continue;
        const t = (e2x * qx + e2y * qy + e2z * qz) * inv;
        if (t > 1e-9 && t < bt) { bt = t; best = { t, tri: i }; if (anyHit) return best; }
      }
      return best;
    }
    function visible(cam, x, y, z) {                                 // nothing solid between the eye and this point
      if (!raw.tri.length) return true;
      const d = [x - cam.eye[0], y - cam.eye[1], z - cam.eye[2]], L = Math.hypot(d[0], d[1], d[2]);
      return !raycast(cam.eye, [d[0] / L, d[1] / L, d[2] / L], L * (1 - 2e-3) - 1e-6, true);
    }
    function projectedPoints(cam) {
      const n = raw.P.length / 3, out = new Float32Array(n * 3);
      for (let i = 0; i < n; i++) {
        const s = toScreen(cam, raw.P[3 * i], raw.P[3 * i + 1], raw.P[3 * i + 2]);
        if (s) { out[3 * i] = s[0]; out[3 * i + 1] = s[1]; out[3 * i + 2] = s[2]; } else out[3 * i + 2] = -1;
      }
      return out;
    }
    function pickPoint(cam, sx, sy, rad) {
      const sp = projectedPoints(cam), cand = [];
      for (let i = 0, n = sp.length / 3; i < n; i++) {
        if (sp[3 * i + 2] < 0) continue;
        const d = Math.hypot(sp[3 * i] - sx, sp[3 * i + 1] - sy);
        if (d <= rad) cand.push([d, i]);
      }
      cand.sort((a, b) => a[0] - b[0]);
      for (const [, i] of cand.slice(0, 8)) if (visible(cam, raw.P[3 * i], raw.P[3 * i + 1], raw.P[3 * i + 2])) return i;
      return -1;
    }
    function pickEdge(cam, sx, sy, rad) {
      const sp = projectedPoints(cam), E = raw.edge, cand = [];
      for (let k = 0, n = E.length / 2; k < n; k++) {
        const a = E[2 * k], b = E[2 * k + 1];
        if (sp[3 * a + 2] < 0 || sp[3 * b + 2] < 0) continue;
        const ax = sp[3 * a], ay = sp[3 * a + 1], bx = sp[3 * b], by = sp[3 * b + 1];
        const dx = bx - ax, dy = by - ay, l2 = dx * dx + dy * dy;
        const t = l2 > 0 ? Math.max(0, Math.min(1, ((sx - ax) * dx + (sy - ay) * dy) / l2)) : 0;
        const d = Math.hypot(ax + t * dx - sx, ay + t * dy - sy);
        if (d <= rad) cand.push([d, k, t]);
      }
      cand.sort((x, y) => x[0] - y[0]);
      for (const [, k, t] of cand.slice(0, 8)) {
        const a = E[2 * k] * 3, b = E[2 * k + 1] * 3, P = raw.P;
        const x = P[a] + (P[b] - P[a]) * t, y = P[a + 1] + (P[b + 1] - P[a + 1]) * t, z = P[a + 2] + (P[b + 2] - P[a + 2]) * t;
        if (visible(cam, x, y, z)) return [E[2 * k], E[2 * k + 1]];
      }
      return null;
    }
    function pickFace(cam, sx, sy) {
      const hit = raycast(cam.eye, rayAt(cam, sx, sy));
      return hit ? raw.triPrim[hit.tri] : -1;
    }
    function boxPick(cam, x0, y0, x1, y1) {                          // everything inside the rectangle (X-ray)
      const lo = [Math.min(x0, x1), Math.min(y0, y1)], hi = [Math.max(x0, x1), Math.max(y0, y1)];
      const inside = (x, y) => x >= lo[0] && x <= hi[0] && y >= lo[1] && y <= hi[1];
      const sp = projectedPoints(cam), out = { points: [], edges: [], faces: [] };
      if (sel.mode === "point") {
        for (let i = 0, n = sp.length / 3; i < n; i++) if (sp[3 * i + 2] >= 0 && inside(sp[3 * i], sp[3 * i + 1])) out.points.push(i);
      } else if (sel.mode === "edge") {
        const E = raw.edge;
        for (let k = 0, n = E.length / 2; k < n; k++) {
          const a = E[2 * k], b = E[2 * k + 1];
          if (sp[3 * a + 2] >= 0 && sp[3 * b + 2] >= 0 && inside(sp[3 * a], sp[3 * a + 1]) && inside(sp[3 * b], sp[3 * b + 1])) out.edges.push([a, b]);
        }
      } else {
        const T = raw.tri, faces = new Set();
        for (let i = 0, n = T.length / 3; i < n; i++) {
          const a = T[3 * i], b = T[3 * i + 1], c = T[3 * i + 2];
          if (sp[3 * a + 2] < 0 || sp[3 * b + 2] < 0 || sp[3 * c + 2] < 0) continue;
          if (inside((sp[3 * a] + sp[3 * b] + sp[3 * c]) / 3, (sp[3 * a + 1] + sp[3 * b + 1] + sp[3 * c + 1]) / 3)) faces.add(raw.triPrim[i]);
        }
        out.faces = [...faces];
      }
      return out;
    }
    function change(found, how) {                                    // found: {points, edges, faces}; how: replace | add | subtract
      if (how === "replace") { sel.point.clear(); sel.edge.clear(); sel.face.clear(); }
      for (const i of found.points || []) how === "subtract" ? sel.point.delete(i) : sel.point.add(i);
      for (const [a, b] of found.edges || []) how === "subtract" ? sel.edge.delete(edgeKey(a, b)) : sel.edge.set(edgeKey(a, b), a < b ? [a, b] : [b, a]);
      for (const f of found.faces || []) how === "subtract" ? sel.face.delete(f) : sel.face.add(f);
      rebuildOverlay();
      if (api.onSelect) api.onSelect(summary());
    }
    function rebuildOverlay() {                                      // GPU copies of the current selection
      const P = raw ? raw.P : new Float32Array(0);
      const pts = new Float32Array(sel.point.size * 3);
      let k = 0;
      for (const i of sel.point) { pts[k++] = P[3 * i]; pts[k++] = P[3 * i + 1]; pts[k++] = P[3 * i + 2]; }
      gl.bindBuffer(gl.ARRAY_BUFFER, ptBuf); gl.bufferData(gl.ARRAY_BUFFER, pts, gl.DYNAMIC_DRAW);
      ov.points = sel.point.size;
      const verts = new Float32Array(sel.edge.size * 4 * 8), idx = new Uint32Array(sel.edge.size * 6);
      let e = 0;
      for (const [a, b] of sel.edge.values()) {
        [[0, -1], [0, 1], [1, 1], [1, -1]].forEach(([t, side], j) => {
          const o = (e * 4 + j) * 8;
          verts.set([P[3 * a], P[3 * a + 1], P[3 * a + 2], P[3 * b], P[3 * b + 1], P[3 * b + 2], t, side], o);
        });
        idx.set([e * 4, e * 4 + 1, e * 4 + 2, e * 4, e * 4 + 2, e * 4 + 3], e * 6);
        e++;
      }
      gl.bindBuffer(gl.ARRAY_BUFFER, sedBuf); gl.bufferData(gl.ARRAY_BUFFER, verts, gl.DYNAMIC_DRAW);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, sedIdx); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, idx, gl.DYNAMIC_DRAW);
      ov.edgeIdx = idx.length;
      const tris = [];
      if (sel.face.size && raw) for (let i = 0; i < raw.triPrim.length; i++) if (sel.face.has(raw.triPrim[i])) tris.push(raw.tri[3 * i], raw.tri[3 * i + 1], raw.tri[3 * i + 2]);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, faceIdx); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, new Uint32Array(tris), gl.DYNAMIC_DRAW);
      ov.faceIdx = tris.length;
      schedule();
    }
    function drawSelection(cam) {
      if (!ov.points && !ov.edgeIdx && !ov.faceIdx) return;
      const ORANGE = [1.0, 0.62, 0.12];
      gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      if (ov.faceIdx) {
        gl.useProgram(colorProg.p); gl.bindVertexArray(vao);
        gl.uniformMatrix4fv(colorProg.u.uMV, false, cam.MV); gl.uniformMatrix4fv(colorProg.u.uP, false, cam.P);
        gl.uniform1f(colorProg.u.uBias, 0.0006); gl.uniform4f(colorProg.u.uColor, ORANGE[0], ORANGE[1], ORANGE[2], 0.5);
        gl.disableVertexAttribArray(1); gl.vertexAttrib3f(1, 1, 1, 1);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, faceIdx); gl.drawElements(gl.TRIANGLES, ov.faceIdx, gl.UNSIGNED_INT, 0);
      }
      if (ov.edgeIdx) {
        gl.useProgram(edgeProg.p); gl.bindVertexArray(sedVao);
        gl.uniformMatrix4fv(edgeProg.u.uMV, false, cam.MV); gl.uniformMatrix4fv(edgeProg.u.uP, false, cam.P);
        gl.uniform2f(edgeProg.u.uViewport, canvas.clientWidth, canvas.clientHeight); gl.uniform1f(edgeProg.u.uWidth, 2.2);
        gl.uniform1f(edgeProg.u.uBias, 0.002); gl.uniform4f(edgeProg.u.uColor, ORANGE[0], ORANGE[1], ORANGE[2], 1);
        gl.drawElements(gl.TRIANGLES, ov.edgeIdx, gl.UNSIGNED_INT, 0);
      }
      if (ov.points) {
        gl.useProgram(colorProg.p); gl.bindVertexArray(ptVao);
        gl.uniformMatrix4fv(colorProg.u.uMV, false, cam.MV); gl.uniformMatrix4fv(colorProg.u.uP, false, cam.P);
        gl.uniform1f(colorProg.u.uPointSize, 9); gl.uniform1f(colorProg.u.uBias, 0.003);
        gl.uniform4f(colorProg.u.uColor, ORANGE[0], ORANGE[1], ORANGE[2], 1);
        gl.drawArrays(gl.POINTS, 0, ov.points);
        gl.uniform1f(colorProg.u.uPointSize, 4);
      }
      gl.disable(gl.BLEND);
    }

    function draw() {
      raf = 0;
      resize();
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0.13, 0.14, 0.16, 1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      api.draws++;
      if (!data.npoints) return;
      const cam = camera(), MV = cam.MV, P = cam.P;
      gl.enable(gl.DEPTH_TEST);
      gl.bindVertexArray(vao);
      const setup = (pr, base, useCol, alpha) => {
        gl.useProgram(pr.p);
        gl.uniformMatrix4fv(pr.u.uMV, false, MV); gl.uniformMatrix4fv(pr.u.uP, false, P);
        gl.uniform3fv(pr.u.uBase, base); gl.uniform1f(pr.u.uUseCol, useCol && data.hasCd ? 1 : 0);
        gl.uniform1f(pr.u.uPointSize, 4); gl.uniform1f(pr.u.uBias, 0);
        if (pr.u.uAlpha) gl.uniform1f(pr.u.uAlpha, alpha === undefined ? 1 : alpha);
        if (data.hasCd) gl.enableVertexAttribArray(1); else { gl.disableVertexAttribArray(1); gl.vertexAttrib3f(1, 1, 1, 1); }
      };
      if (opt.shaded && data.ntris) {
        setup(mesh, [0.62, 0.68, 0.76], true);
        gl.enable(gl.POLYGON_OFFSET_FILL); gl.polygonOffset(1, 1);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, triBuf); gl.drawElements(gl.TRIANGLES, data.ntris * 3, gl.UNSIGNED_INT, 0);
        gl.disable(gl.POLYGON_OFFSET_FILL);
      }
      if ((opt.edges || sel.mode === "edge") && data.nedges) {
        const over = opt.shaded && data.ntris;                  // over a shaded surface the edges are soft dark lines
        setup(flat, over ? [0.04, 0.05, 0.06] : [0.75, 0.82, 0.9], false, over ? (sel.mode === "edge" ? 0.6 : 0.35) : 1);
        gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, edgeBuf); gl.drawElements(gl.LINES, data.nedges * 2, gl.UNSIGNED_INT, 0);
        gl.disable(gl.BLEND);
      }
      if (opt.points || sel.mode === "point" || (!data.ntris && !data.nedges)) {
        setup(flat, sel.mode === "point" ? [0.9, 0.93, 0.98] : [0.95, 0.75, 0.3], true);
        gl.uniform1f(flat.u.uBias, 0.0004);
        gl.drawArrays(gl.POINTS, 0, data.npoints);
      }
      drawSelection(cam);
      gl.bindVertexArray(null);
    }
    function schedule() { if (!raf) raf = requestAnimationFrame(draw); }

    // ---------------------------------------------------------------- mouse: orbit / pan / select
    const marquee = document.createElement("div");
    marquee.className = "pv-marquee hidden";
    canvas.parentElement.append(marquee);
    let dragging = null;
    canvas.addEventListener("pointerdown", (e) => {
      canvas.setPointerCapture(e.pointerId);
      const r = canvas.getBoundingClientRect();
      const selecting = sel.mode !== "view" && e.button === 0 && !e.altKey && raw;
      const pan = sel.mode === "view" ? (e.button === 1 || e.button === 2 || e.shiftKey) : (e.button === 2 || (e.button === 1 && e.shiftKey));
      dragging = { x: e.clientX, y: e.clientY, pan, select: !!selecting, x0: e.clientX - r.left, y0: e.clientY - r.top, moved: false,
        how: e.shiftKey ? "add" : (e.ctrlKey || e.metaKey) ? "subtract" : "replace" };
      e.preventDefault();
    });
    canvas.addEventListener("pointermove", (e) => {
      if (!dragging) return;
      if (dragging.select) {
        const r = canvas.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top;
        if (Math.hypot(x - dragging.x0, y - dragging.y0) > 4) dragging.moved = true;
        if (dragging.moved) {
          marquee.classList.remove("hidden");
          Object.assign(marquee.style, { left: Math.min(x, dragging.x0) + "px", top: Math.min(y, dragging.y0) + "px",
            width: Math.abs(x - dragging.x0) + "px", height: Math.abs(y - dragging.y0) + "px" });
          dragging.x1 = x; dragging.y1 = y;
        }
        return;
      }
      const dx = e.clientX - dragging.x, dy = e.clientY - dragging.y;
      dragging.x = e.clientX; dragging.y = e.clientY;
      if (dragging.pan) {
        const k = (view.dist * Math.tan(Math.PI / 8) * 2) / Math.max(canvas.clientHeight, 1);
        const right = [-Math.sin(view.yaw), Math.cos(view.yaw), 0];
        const up = [-Math.sin(view.pitch) * Math.cos(view.yaw), -Math.sin(view.pitch) * Math.sin(view.yaw), Math.cos(view.pitch)];
        for (let i = 0; i < 3; i++) view.target[i] += (-dx * right[i] + dy * up[i]) * k;
      } else {
        view.yaw -= dx * 0.008;
        view.pitch = Math.max(-1.5, Math.min(1.5, view.pitch + dy * 0.008));
      }
      schedule();
    });
    const endDrag = (e) => {
      const d = dragging; dragging = null;
      marquee.classList.add("hidden");
      if (!d || !d.select || (e && e.type === "pointercancel")) return;
      const cam = camera();
      if (d.moved && d.x1 !== undefined) api.boxSelect(d.x0, d.y0, d.x1, d.y1, d.how);
      else api.pickAt(d.x0, d.y0, d.how);
    };
    canvas.addEventListener("pointerup", endDrag); canvas.addEventListener("pointercancel", endDrag);
    canvas.addEventListener("contextmenu", (e) => e.preventDefault());
    canvas.addEventListener("wheel", (e) => { e.preventDefault(); view.dist *= Math.exp(e.deltaY * 0.0012); schedule(); }, { passive: false });
    canvas.addEventListener("dblclick", (e) => { if (sel.mode === "view") frameAll(); });
    if (window.ResizeObserver) new ResizeObserver(schedule).observe(canvas);

    Object.assign(api, {
      view, opt, onSelect: null,
      set(head, arrays, keepCamera) {
        upload(head, arrays);
        if (!keepCamera) frameAll(); else schedule();
        if (sel.point.size || sel.edge.size || sel.face.size) change({}, "replace");        // the geometry changed: drop the selection
        else rebuildOverlay();
      },
      clear() { data.npoints = 0; raw = null; Object.assign(api, { tris: 0, edges: 0, points: 0 }); change({}, "replace"); schedule(); },
      frameAll, redraw: schedule,
      setMode(m) {
        if (!["view", "point", "edge", "face"].includes(m)) return;
        sel.mode = m;
        canvas.style.cursor = m === "view" ? "grab" : "crosshair";
        change({}, "replace");                                                          // a new mode starts with nothing selected
        schedule();
      },
      selection: summary,
      clearSelection() { change({}, "replace"); },
      pickAt(sx, sy, how) {                                                             // CSS pixels inside the canvas
        if (!raw || sel.mode === "view") return false;
        const cam = camera();
        if (sel.mode === "point") { const i = pickPoint(cam, sx, sy, 12); change({ points: i >= 0 ? [i] : [] }, i < 0 && how === "replace" ? "replace" : how); }
        else if (sel.mode === "edge") { const e = pickEdge(cam, sx, sy, 8); change({ edges: e ? [e] : [] }, !e && how === "replace" ? "replace" : how); }
        else { const f = pickFace(cam, sx, sy); change({ faces: f >= 0 ? [f] : [] }, f < 0 && how === "replace" ? "replace" : how); }
        return true;
      },
      boxSelect(x0, y0, x1, y1, how) { if (raw && sel.mode !== "view") change(boxPick(camera(), x0, y0, x1, y1), how || "replace"); },
      selectAll() {
        if (!raw || sel.mode === "view") return;
        const all = { points: [], edges: [], faces: [] };
        if (sel.mode === "point") for (let i = 0; i < raw.P.length / 3; i++) all.points.push(i);
        else if (sel.mode === "edge") for (let k = 0; k < raw.edge.length / 2; k++) all.edges.push([raw.edge[2 * k], raw.edge[2 * k + 1]]);
        else all.faces = [...new Set(raw.triPrim)];
        change(all, "replace");
      },
      // where an element is on screen (CSS pixels in the canvas): for tests and for anchoring tools
      screenOf(kind, id) {
        if (!raw) return null;
        const cam = camera(), P = raw.P;
        if (kind === "point") return toScreen(cam, P[3 * id], P[3 * id + 1], P[3 * id + 2]);
        if (kind === "edge") {
          const [a, b] = id;
          return toScreen(cam, (P[3 * a] + P[3 * b]) / 2, (P[3 * a + 1] + P[3 * b + 1]) / 2, (P[3 * a + 2] + P[3 * b + 2]) / 2);
        }
        let n = 0, x = 0, y = 0, z = 0;                                                  // a face: the centre of its triangles
        for (let i = 0; i < raw.triPrim.length; i++) if (raw.triPrim[i] === id) for (let c = 0; c < 3; c++) {
          const v = raw.tri[3 * i + c] * 3; x += P[v]; y += P[v + 1]; z += P[v + 2]; n++;
        }
        return n ? toScreen(cam, x / n, y / n, z / n) : null;
      },
    });
    Object.defineProperty(api, "mode", { get: () => sel.mode, enumerable: true });         // live, unlike Object.assign's copy
    return api;
  }
  return { create, parse };
})();
