// MiniHou 3-D preview: a small WebGL2 viewer for the geometry of a node. Plain JS, no libraries.
// Data comes from POST /api/preview (see minihou/preview.py for the wire format). Z is up, like Blender.
"use strict";
window.MiniHouPreview = (() => {
  const VS = `#version 300 es
  in vec3 aPos; in vec3 aCol;
  uniform mat4 uMV, uP; uniform float uPointSize;
  out vec3 vView; out vec3 vCol;
  void main() { vec4 v = uMV * vec4(aPos, 1.0); vView = v.xyz; vCol = aCol; gl_Position = uP * v; gl_PointSize = uPointSize; }`;
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
    const prog = (fs) => {
      const p = gl.createProgram();
      for (const [type, src] of [[gl.VERTEX_SHADER, VS], [gl.FRAGMENT_SHADER, fs]]) {
        const sh = gl.createShader(type); gl.shaderSource(sh, src); gl.compileShader(sh);
        if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(sh));
        gl.attachShader(p, sh);
      }
      gl.bindAttribLocation(p, 0, "aPos"); gl.bindAttribLocation(p, 1, "aCol");
      gl.linkProgram(p);
      if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p));
      const u = {};
      for (const n of ["uMV", "uP", "uPointSize", "uBase", "uUseCol", "uAlpha"]) u[n] = gl.getUniformLocation(p, n);
      return { p, u };
    };
    let mesh, flat;
    try { mesh = prog(FS_MESH); flat = prog(FS_FLAT); } catch (e) { if (onMessage) onMessage("3-D preview unavailable: " + e.message); api.ok = false; return api; }

    const vao = gl.createVertexArray(), posBuf = gl.createBuffer(), colBuf = gl.createBuffer();
    const triBuf = gl.createBuffer(), edgeBuf = gl.createBuffer();
    gl.bindVertexArray(vao);
    gl.bindBuffer(gl.ARRAY_BUFFER, posBuf); gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, colBuf); gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);

    const view = { yaw: -0.6, pitch: 0.45, dist: 5, target: [0, 0, 0] };
    const opt = { shaded: true, edges: true, points: false };
    let data = { npoints: 0, ntris: 0, nedges: 0, hasCd: false, bounds: [[-1, -1, -1], [1, 1, 1]] };
    let raf = 0;

    function upload(head, arrays) {
      gl.bindBuffer(gl.ARRAY_BUFFER, posBuf); gl.bufferData(gl.ARRAY_BUFFER, arrays.P, gl.STATIC_DRAW);
      data.hasCd = !!arrays.Cd;
      if (arrays.Cd) { gl.bindBuffer(gl.ARRAY_BUFFER, colBuf); gl.bufferData(gl.ARRAY_BUFFER, arrays.Cd, gl.STATIC_DRAW); }
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, triBuf); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, arrays.tri, gl.STATIC_DRAW);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, edgeBuf); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, arrays.edge, gl.STATIC_DRAW);
      data.npoints = arrays.P.length / 3; data.ntris = arrays.tri.length / 3; data.nedges = arrays.edge.length / 2;
      data.bounds = head.bounds;
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
    function draw() {
      raf = 0;
      resize();
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0.13, 0.14, 0.16, 1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      api.draws++;
      if (!data.npoints) return;
      const [lo, hi] = data.bounds, r = Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2, 1e-3);
      const cp = Math.cos(view.pitch), eye = [view.target[0] + view.dist * cp * Math.cos(view.yaw),
        view.target[1] + view.dist * cp * Math.sin(view.yaw), view.target[2] + view.dist * Math.sin(view.pitch)];
      const MV = lookAt(eye, view.target, [0, 0, 1]);
      const P = perspective(Math.PI / 4, canvas.width / canvas.height, Math.max(view.dist * 0.01, r * 1e-3), view.dist + r * 4);
      gl.enable(gl.DEPTH_TEST);
      gl.bindVertexArray(vao);
      const setup = (pr, base, useCol, alpha) => {
        gl.useProgram(pr.p);
        gl.uniformMatrix4fv(pr.u.uMV, false, MV); gl.uniformMatrix4fv(pr.u.uP, false, P);
        gl.uniform3fv(pr.u.uBase, base); gl.uniform1f(pr.u.uUseCol, useCol && data.hasCd ? 1 : 0);
        gl.uniform1f(pr.u.uPointSize, 4);
        if (pr.u.uAlpha) gl.uniform1f(pr.u.uAlpha, alpha === undefined ? 1 : alpha);
        if (data.hasCd) gl.enableVertexAttribArray(1); else { gl.disableVertexAttribArray(1); gl.vertexAttrib3f(1, 1, 1, 1); }
      };
      if (opt.shaded && data.ntris) {
        setup(mesh, [0.62, 0.68, 0.76], true);
        gl.enable(gl.POLYGON_OFFSET_FILL); gl.polygonOffset(1, 1);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, triBuf); gl.drawElements(gl.TRIANGLES, data.ntris * 3, gl.UNSIGNED_INT, 0);
        gl.disable(gl.POLYGON_OFFSET_FILL);
      }
      if (opt.edges && data.nedges) {
        const over = opt.shaded && data.ntris;                  // over a shaded surface the edges are soft dark lines
        setup(flat, over ? [0.04, 0.05, 0.06] : [0.75, 0.82, 0.9], false, over ? 0.35 : 1);
        gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, edgeBuf); gl.drawElements(gl.LINES, data.nedges * 2, gl.UNSIGNED_INT, 0);
        gl.disable(gl.BLEND);
      }
      if (opt.points || (!data.ntris && !data.nedges)) {
        setup(flat, [0.95, 0.75, 0.3], true);
        gl.drawArrays(gl.POINTS, 0, data.npoints);
      }
      gl.bindVertexArray(null);
    }
    function schedule() { if (!raf) raf = requestAnimationFrame(draw); }

    // ---- camera controls
    let dragging = null;
    canvas.addEventListener("pointerdown", (e) => {
      canvas.setPointerCapture(e.pointerId);
      dragging = { x: e.clientX, y: e.clientY, pan: e.button === 1 || e.button === 2 || e.shiftKey };
      e.preventDefault();
    });
    canvas.addEventListener("pointermove", (e) => {
      if (!dragging) return;
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
    const endDrag = () => { dragging = null; };
    canvas.addEventListener("pointerup", endDrag); canvas.addEventListener("pointercancel", endDrag);
    canvas.addEventListener("contextmenu", (e) => e.preventDefault());
    canvas.addEventListener("wheel", (e) => { e.preventDefault(); view.dist *= Math.exp(e.deltaY * 0.0012); schedule(); }, { passive: false });
    canvas.addEventListener("dblclick", frameAll);
    if (window.ResizeObserver) new ResizeObserver(schedule).observe(canvas);

    Object.assign(api, {
      view, opt,
      set(head, arrays, keepCamera) { upload(head, arrays); if (!keepCamera) frameAll(); else schedule(); },
      clear() { data.npoints = 0; Object.assign(api, { tris: 0, edges: 0, points: 0 }); schedule(); },
      frameAll, redraw: schedule,
    });
    return api;
  }
  return { create, parse };
})();
