// MiniHou 3-D preview: a small WebGL2 viewer for the geometry of a node. Plain JS, no libraries.
// Data comes from POST /api/preview (see minihou/preview.py for the wire format). Z is up, like Blender.
"use strict";
window.MiniHouPreview = (() => {
  const VS = `#version 300 es
  in vec3 aPos; in vec3 aCol; in float aOwner;
  uniform mat4 uMV, uP; uniform float uPointSize, uBias, uIsolate, uActiveStart, uActiveEnd, uOwnerMode;
  out float vDim;
  out vec3 vView; out vec3 vCol; out vec3 vWorld;
  void main() { float owner = uOwnerMode > 0.5 ? aOwner : float(gl_VertexID); vDim = (owner < uActiveStart || owner >= uActiveEnd) ? uIsolate : 0.0; vec4 v = uMV * vec4(aPos, 1.0); vView = v.xyz; vCol = aCol; vWorld = aPos; gl_Position = uP * v;
                gl_Position.z -= uBias * gl_Position.w; gl_PointSize = uPointSize; }`;
  // selected edges: each edge is a quad that is widened in screen space, so selections read as thick lines
  const VS_EDGE = `#version 300 es
  in vec3 aA; in vec3 aB; in float aT; in float aS;
  uniform mat4 uMV, uP; uniform vec2 uViewport; uniform float uWidth, uBias;
  out vec3 vWorld;
  void main() {
    vWorld = mix(aA, aB, aT);
    vec4 ca = uP * uMV * vec4(aA, 1.0), cb = uP * uMV * vec4(aB, 1.0);
    vec2 d = (cb.xy / max(cb.w, 1e-4) - ca.xy / max(ca.w, 1e-4)) * uViewport;
    float l = length(d); d = l > 1e-6 ? d / l : vec2(1.0, 0.0);
    vec4 c = mix(ca, cb, aT);
    c.xy += vec2(-d.y, d.x) * aS * uWidth * 2.0 / uViewport * c.w;
    c.z -= uBias * c.w;
    gl_Position = c;
  }`;
  // The section plane (a cut through the scene): fragments on the far side of it are not drawn. Every program that draws geometry shares this.
  const CLIP_DECL = "in vec3 vWorld; uniform vec4 uClip; uniform float uClipOn;";
  const CLIP_TEST = "if (uClipOn > 0.5 && dot(vWorld, uClip.xyz) > uClip.w) discard;";
  const FS_COLOR = `#version 300 es
  precision highp float;
  ${CLIP_DECL}
  uniform vec4 uColor; out vec4 o;
  void main() { ${CLIP_TEST} o = uColor; }`;
  const FS_MESH = `#version 300 es
  precision highp float;
  in float vDim; in vec3 vView; in vec3 vCol; ${CLIP_DECL} uniform vec3 uBase; uniform float uUseCol, uLighting; out vec4 o;
  void main() {
    ${CLIP_TEST}
    vec3 n = normalize(cross(dFdx(vView), dFdy(vView)));
    float d = max(n.z, 0.0);                                    // headlight: view space looks down -Z, n faces the viewer
    vec3 base = mix(uBase, vCol, uUseCol);
    vec3 col = base * (0.28 + 0.72 * d * uLighting) + vec3(0.10) * pow(1.0 - d, 3.0);
    if (!gl_FrontFacing) col = mix(col, vec3(0.95, 0.45, 0.18), 0.55);   // back faces: flipped normals show up orange
    o = vec4(mix(col, vec3(0.22), vDim), 1.0);
  }`;
  const FS_FLAT = `#version 300 es
  precision highp float;
  in float vDim; in vec3 vView; in vec3 vCol; ${CLIP_DECL} uniform vec3 uBase; uniform float uUseCol; uniform float uAlpha; out vec4 o;
  void main() { ${CLIP_TEST} o = vec4(mix(mix(uBase, vCol, uUseCol),vec3(0.22),vDim), uAlpha); }`;

  // Instances drawn as impostors: one transform per instance (three rows of a 3x4 matrix) on shared shape meshes. Two-sided: a mirrored instance flips its winding.
  const VS_INST = `#version 300 es
  in vec3 aPos; in vec4 aR0; in vec4 aR1; in vec4 aR2; in vec3 aCol;
  uniform mat4 uMV, uP;
  out vec3 vView; out vec3 vCol; out vec3 vWorld;
  void main() { vec4 p = vec4(aPos, 1.0); vec3 w = vec3(dot(aR0, p), dot(aR1, p), dot(aR2, p));
                vec4 v = uMV * vec4(w, 1.0); vView = v.xyz; vCol = aCol; vWorld = w; gl_Position = uP * v; }`;
  const FS_INST = `#version 300 es
  precision highp float;
  in vec3 vView; in vec3 vCol; ${CLIP_DECL} uniform vec3 uBase; uniform float uUseCol, uLighting, uDim; out vec4 o;
  void main() {
    ${CLIP_TEST}
    vec3 n = normalize(cross(dFdx(vView), dFdy(vView)));
    float d = abs(n.z);
    vec3 base = mix(uBase, vCol, uUseCol);
    o = vec4(mix(base * (0.28 + 0.72 * d * uLighting) + vec3(0.10) * pow(1.0 - d, 3.0),vec3(0.22),uDim), 1.0);
  }`;

  // Infinite Z-up construction plane, drawn as a background so it never hides geometry.
  const VS_GRID = `#version 300 es
  out vec2 vNdc;
  void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    vNdc = p * 2.0 - 1.0; gl_Position = vec4(vNdc, 0.0, 1.0);
  }`;
  const FS_GRID = `#version 300 es
  precision highp float;
  in vec2 vNdc;
  uniform mat4 uMV, uP;
  uniform vec3 uEye, uTarget;
  uniform float uSpacing, uRange, uOpacity, uAxes;
  out vec4 o;
  float lines(vec2 p) {
    vec2 width = max(fwidth(p), vec2(0.0001));
    vec2 d = abs(fract(p - 0.5) - 0.5) / width;
    return 1.0 - min(min(d.x, d.y), 1.0);
  }
  void main() {
    vec3 ray = normalize(transpose(mat3(uMV)) * vec3(vNdc / vec2(uP[0][0], uP[1][1]), -1.0));
    if (abs(ray.z) < 0.00001) discard;
    float t = -uEye.z / ray.z;
    if (t <= 0.0) discard;
    vec2 p = (uEye + ray * t).xy;
    float minor = lines(p / uSpacing), major = lines(p / (uSpacing * 10.0));
    float fade = 1.0 - smoothstep(uRange * 0.25, uRange, length(p - uTarget.xy));
    fade *= smoothstep(0.0, 0.15, abs(ray.z));
    vec2 axis = 1.0 - min(abs(p) / max(fwidth(p), vec2(0.0001)), vec2(1.0));
    vec3 color = vec3(0.48, 0.50, 0.53);
    color = mix(color, vec3(0.65, 0.28, 0.26), axis.y * uAxes);
    color = mix(color, vec3(0.28, 0.53, 0.34), axis.x * uAxes);
    o = vec4(color, max(max(minor * 0.18, major * 0.32), max(axis.x, axis.y) * 0.5 * uAxes) * fade * uOpacity);
  }`;

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
    const CLIP_U = ["uClip", "uClipOn"];
    const MESH_U = ["uOwnerMode", "uIsolate", "uActiveStart", "uActiveEnd","uMV", "uP", "uPointSize", "uBase", "uUseCol", "uAlpha", "uBias", "uLighting", ...CLIP_U];
    let mesh, flat, edgeProg, colorProg, gridProg, instProg;
    try {
      gridProg = prog(VS_GRID, FS_GRID, [], ["uMV", "uP", "uEye", "uTarget", "uSpacing", "uRange", "uOpacity", "uAxes"]);
      mesh = prog(VS, FS_MESH, ["aPos", "aCol", "aOwner"], MESH_U);
      flat = prog(VS, FS_FLAT, ["aPos", "aCol", "aOwner"], MESH_U);
      edgeProg = prog(VS_EDGE, FS_COLOR, ["aA", "aB", "aT", "aS"], ["uMV", "uP", "uViewport", "uWidth", "uBias", "uColor", ...CLIP_U]);
      colorProg = prog(VS, FS_COLOR, ["aPos", "aCol"], ["uMV", "uP", "uPointSize", "uBias", "uColor", ...CLIP_U]);
      instProg = prog(VS_INST, FS_INST, ["aPos", "aR0", "aR1", "aR2", "aCol"], ["uMV", "uP", "uBase", "uUseCol", "uLighting", "uDim", ...CLIP_U]);
    } catch (e) { if (onMessage) onMessage("3-D preview unavailable: " + e.message); api.ok = false; return api; }

    const vao = gl.createVertexArray(), posBuf = gl.createBuffer(), colBuf = gl.createBuffer();
    const triBuf = gl.createBuffer(), edgeBuf = gl.createBuffer();
    const templateVao = gl.createVertexArray(), templatePos = gl.createBuffer(), templateEdge = gl.createBuffer();
    let templateRaw = null;
    gl.bindVertexArray(templateVao);gl.bindBuffer(gl.ARRAY_BUFFER,templatePos);
    gl.enableVertexAttribArray(0);gl.vertexAttribPointer(0,3,gl.FLOAT,false,0,0);
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
    // normals overlay: one line per primitive from its centre along its normal (from the winding), and a dot at the tip
    const nrmVao = gl.createVertexArray(), nrmBuf = gl.createBuffer(), tipVao = gl.createVertexArray(), tipBuf = gl.createBuffer();
    for (const [vo, bu] of [[nrmVao, nrmBuf], [tipVao, tipBuf]]) {
      gl.bindVertexArray(vo); gl.bindBuffer(gl.ARRAY_BUFFER, bu); gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    }
    gl.bindVertexArray(null);
    let nrmCount = 0, nrmDirty = true, nrmLines = new Float32Array(0);
    const MAX_NORMALS = 100000;
    const ov = { points: 0, edgeIdx: 0, faceIdx: 0 };

    const view = { yaw: -0.6, pitch: 0.45, dist: 5, target: [0, 0, 0], ortho: false };
    const PITCH_MAX = 1.5707, FOV = Math.PI / 4, ORTHO_FOV = 0.03;     // "orthographic" is a very narrow lens far away: the same maths, so picking and handles keep working
    const opt = { bounds: false, navigation: "blender", orbitDepth: true, shaded: true, edges: true, points: false, normals: false, grid: true, background: "#212429", gridOpacity: 1, axes: true, lighting: 1, pointSize: 4, gridSpacing: 0,
      instBias: 1, instTris: 1500000,
      clipNear: 0, clipFar: 0,                                       // camera clip distances (0 = automatic from the scene's size)
      section: { on: false, axis: 2, offset: 0, flip: false },       // a cutting plane: what lies beyond it is hidden (and cannot be picked or painted)
      cull: true, cullPx: 0,                                         // skip the runs of triangles outside the view (and, if cullPx > 0, ones smaller than that on screen)
      navQuality: "full" };                                          // full | balanced | fast: draw cheaper while the camera moves                              // impostors: > 1 keeps detail to smaller instances; the most triangles all instances may draw together
    let data = { npoints: 0, ntris: 0, nedges: 0, hasCd: false, bounds: [[-1, -1, -1], [1, 1, 1]] };
    let measurementCache=null;
    let raw = null;                                                  // the geometry's typed arrays, for picking
    let raf = 0;

    // ---------------------------------------------------------------- an attribute shown as colour (the painted one)
    // attrView = {domain: point|prim|edge, comps, values (Float32Array), min, max, ramp: heat|gray}. Points reuse the colour buffer, primitives
    // draw from an unwelded copy of the triangles (a point is shared by faces, a primitive's colour is not), edges are an overlay.
    let attrView = null, cd0 = null, primCount = 0, aedCount = 0, ptCol = null, primColArr = null, primTriIdx = null;
    const primVao = gl.createVertexArray(), primPos = gl.createBuffer(), primCol = gl.createBuffer(), primOwner = gl.createBuffer();
    gl.bindVertexArray(primVao); gl.bindBuffer(gl.ARRAY_BUFFER, primPos); gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER, primCol); gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 3, gl.FLOAT, false, 0, 0);
    gl.bindBuffer(gl.ARRAY_BUFFER,primOwner);gl.enableVertexAttribArray(2);gl.vertexAttribPointer(2,1,gl.FLOAT,false,0,0);gl.bindVertexArray(null);
    const aedVao = gl.createVertexArray(), aedBuf = gl.createBuffer(), aedIdx = gl.createBuffer();
    gl.bindVertexArray(aedVao); gl.bindBuffer(gl.ARRAY_BUFFER, aedBuf);
    for (const [loc, n, off] of [[0, 3, 0], [1, 3, 12], [2, 1, 24], [3, 1, 28]]) { gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, n, gl.FLOAT, false, 32, off); }
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, aedIdx); gl.bindVertexArray(null);
    const MAX_PRIM_COLOUR_TRIS = Math.floor(4000000 * 72 / 84); // retain the previous 288 MB buffer budget
    const HEAT = [[0.12, 0.2, 0.75], [0.1, 0.75, 0.85], [0.2, 0.8, 0.25], [0.95, 0.85, 0.15], [0.9, 0.2, 0.15]];
    function colourOf(a, i, out, o) {                                    // the colour of element i of the attribute, into out[o..o+2]
      const v = a.values, nc = a.comps;
      if (nc >= 3) { out[o] = Math.min(1, Math.max(0, v[3 * i])); out[o + 1] = Math.min(1, Math.max(0, v[3 * i + 1])); out[o + 2] = Math.min(1, Math.max(0, v[3 * i + 2])); return; }
      const span = a.max - a.min, t = span > 0 ? Math.min(1, Math.max(0, (v[nc * i] - a.min) / span)) : 0;
      if (a.ramp === "gray") { out[o] = out[o + 1] = out[o + 2] = t; return; }
      const f = t * (HEAT.length - 1), k = Math.min(HEAT.length - 2, Math.floor(f)), u = f - k;
      for (let c = 0; c < 3; c++) out[o + c] = HEAT[k][c] * (1 - u) + HEAT[k + 1][c] * u;
    }
    function recolour() {
      const a = attrView;
      if (!a || !raw) return;
      if (a.domain === "point") {
        const n = data.npoints, col = new Float32Array(n * 3);
        for (let i = 0; i < n; i++) colourOf(a, i, col, 3 * i);
        gl.bindBuffer(gl.ARRAY_BUFFER, colBuf); gl.bufferData(gl.ARRAY_BUFFER, col, gl.DYNAMIC_DRAW); data.hasCd = true; ptCol = col;
      } else if (a.domain === "prim") {
        const T = raw.tri, TP = raw.triPrim, nt = T.length / 3;
        if (nt > MAX_PRIM_COLOUR_TRIS) {                                  // unwelding needs 84 bytes per triangle including explicit corner owners: too much for the tab
          if (onMessage) onMessage(`Too many faces to colour by primitive (${nt.toLocaleString()} triangles); points and edges still work.`);
          return;
        }
        if (primCount !== nt * 3) {                                       // the unwelded positions, once per geometry
          const pos = new Float32Array(nt * 9), owner = new Float32Array(nt * 3);
          for(let t=0;t<nt;t++)owner.fill(TP[t],t*3,t*3+3);
          gl.bindBuffer(gl.ARRAY_BUFFER,primOwner);gl.bufferData(gl.ARRAY_BUFFER,owner,gl.STATIC_DRAW);
          for (let t = 0; t < nt; t++) for (let c = 0; c < 3; c++) { const v = T[3 * t + c] * 3; pos.set([raw.P[v], raw.P[v + 1], raw.P[v + 2]], 9 * t + 3 * c); }
          gl.bindBuffer(gl.ARRAY_BUFFER, primPos); gl.bufferData(gl.ARRAY_BUFFER, pos, gl.STATIC_DRAW); primCount = nt * 3;
        }
        const col = new Float32Array(nt * 9), tmp = [0, 0, 0];
        for (let t = 0; t < nt; t++) { colourOf(a, TP[t], tmp, 0); for (let c = 0; c < 3; c++) col.set(tmp, 9 * t + 3 * c); }
        gl.bindBuffer(gl.ARRAY_BUFFER, primCol); gl.bufferData(gl.ARRAY_BUFFER, col, gl.DYNAMIC_DRAW); primColArr = col;
        if (!primTriIdx || primTriIdx.raw !== raw) {                         // which triangles each primitive owns, for touch-ups
          let np = 0; for (let t = 0; t < nt; t++) if (TP[t] >= np) np = TP[t] + 1;
          const start = new Int32Array(np + 1), items = new Int32Array(nt);
          for (let t = 0; t < nt; t++) start[TP[t] + 1]++;
          for (let f = 0; f < np; f++) start[f + 1] += start[f];
          const fill = start.slice(0, np);
          for (let t = 0; t < nt; t++) items[fill[TP[t]]++] = t;
          primTriIdx = { raw, start, items, np };
        }
      } else if (a.domain === "edge") {
        const E = raw.edge, P = raw.P, m = [];                            // the edges that are in the group (one colour: the overlay has no per-edge colour)
        for (let k = 0; k < E.length / 2; k++) if (k < a.values.length / a.comps && a.values[a.comps * k] !== 0) m.push(k);
        const verts = new Float32Array(m.length * 32), idx = new Uint32Array(m.length * 6);
        m.forEach((k, e) => {
          const A = E[2 * k] * 3, B = E[2 * k + 1] * 3;
          [[0, -1], [0, 1], [1, 1], [1, -1]].forEach(([t, side], j) => verts.set([P[A], P[A + 1], P[A + 2], P[B], P[B + 1], P[B + 2], t, side], (e * 4 + j) * 8));
          idx.set([e * 4, e * 4 + 1, e * 4 + 2, e * 4, e * 4 + 2, e * 4 + 3], e * 6);
        });
        gl.bindBuffer(gl.ARRAY_BUFFER, aedBuf); gl.bufferData(gl.ARRAY_BUFFER, verts, gl.DYNAMIC_DRAW);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, aedIdx); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, idx, gl.DYNAMIC_DRAW);
        aedCount = idx.length;
      }
      schedule();
    }
    function recolourTouched(ids) {                                       // after a dab: only the elements it changed, not the whole buffer
      const a = attrView;
      if (!a || !raw) return;
      if (a.domain === "point" && ptCol && ptCol.length === data.npoints * 3) {
        gl.bindBuffer(gl.ARRAY_BUFFER, colBuf);
        for (const i of ids) { colourOf(a, i, ptCol, 3 * i); gl.bufferSubData(gl.ARRAY_BUFFER, i * 12, ptCol, 3 * i, 3); }
      } else if (a.domain === "prim" && primColArr && primTriIdx && primTriIdx.raw === raw && primCount === data.ntris * 3) {
        gl.bindBuffer(gl.ARRAY_BUFFER, primCol);
        const tmp = [0, 0, 0];
        for (const f of ids) {
          if (f >= primTriIdx.np) continue;
          colourOf(a, f, tmp, 0);
          for (let j = primTriIdx.start[f]; j < primTriIdx.start[f + 1]; j++) {
            const t = primTriIdx.items[j];
            for (let c = 0; c < 3; c++) primColArr.set(tmp, 9 * t + 3 * c);
            gl.bufferSubData(gl.ARRAY_BUFFER, t * 36, primColArr, 9 * t, 9);
          }
        }
      } else { recolour(); return; }
      schedule();
    }
    function setAttrView(v) {
      const was = attrView;
      attrView = v || null;
      if (was && was.domain === "point" && !attrView) {                    // back to the geometry's own colours
        if (cd0) { gl.bindBuffer(gl.ARRAY_BUFFER, colBuf); gl.bufferData(gl.ARRAY_BUFFER, cd0, gl.STATIC_DRAW); }
        data.hasCd = !!cd0;
      }
      if (was && was.domain === "edge" && !(attrView && attrView.domain === "edge")) aedCount = 0;
      if (attrView) recolour(); else schedule();
    }
    function upload(head, arrays, keepIndices) {
      gl.bindBuffer(gl.ARRAY_BUFFER, posBuf);
      if (keepIndices && raw && raw.P.length === arrays.P.length) gl.bufferSubData(gl.ARRAY_BUFFER, 0, arrays.P);   // only the points moved
      else gl.bufferData(gl.ARRAY_BUFFER, arrays.P, gl.STATIC_DRAW);
      data.hasCd = !!arrays.Cd; cd0 = arrays.Cd || null; attrView = null; primCount = 0; aedCount = 0;
      if (arrays.Cd) { gl.bindBuffer(gl.ARRAY_BUFFER, colBuf); gl.bufferData(gl.ARRAY_BUFFER, arrays.Cd, gl.STATIC_DRAW); }
      if (!keepIndices) {
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, triBuf); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, arrays.tri, gl.STATIC_DRAW);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, edgeBuf); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, arrays.edge, gl.STATIC_DRAW);
      }
      data.npoints = arrays.P.length / 3; data.ntris = arrays.tri.length / 3; data.nedges = arrays.edge.length / 2;
      data.bounds = head.bounds; nrmDirty = true;
      uploadInstances(head, arrays);
      raw = { P: arrays.P, tri: arrays.tri, edge: arrays.edge, triPrim: arrays.tri_prim || new Uint32Array(arrays.tri.length / 3), chunks: arrays.chunks || null,
              topology: head.topology || null };
      Object.assign(api, { tris: data.ntris, edges: data.nedges, points: data.npoints });
    }
    function frameAll() {
      const [lo, hi] = data.bounds;
      view.target = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
      const r = Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2, 1e-3);
      view.dist = fitDist(r, 1.15);
      schedule();
    }
    function fitDist(r, slack) {                                     // far enough that a sphere of radius r fits the narrower side of the canvas
      const aspect = Math.min(1, Math.max(canvas.clientWidth, 1) / Math.max(canvas.clientHeight, 1));
      return r / (Math.tan(Math.PI / 8) * aspect) * slack;
    }
    function resize(scale) {
      const dpr = (window.devicePixelRatio || 1) * (scale || 1), w = Math.max(1, Math.round(canvas.clientWidth * dpr)), h = Math.max(1, Math.round(canvas.clientHeight * dpr));
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h; }
    }
    function camera() {                                              // matrices for the current view; sizes in CSS pixels
      const [lo, hi] = data.bounds, r = Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2, 1e-3);
      const fov = view.ortho ? ORTHO_FOV : FOV, D = view.dist * Math.tan(FOV / 2) / Math.tan(fov / 2);   // same apparent size either way
      const cp = Math.cos(view.pitch), eye = [view.target[0] + D * cp * Math.cos(view.yaw),
        view.target[1] + D * cp * Math.sin(view.yaw), view.target[2] + D * Math.sin(view.pitch)];
      const w = Math.max(canvas.clientWidth, 1), h = Math.max(canvas.clientHeight, 1);
      let near = view.ortho ? Math.max(D - view.dist - r * 4, D * 0.01) : Math.max(view.dist * 0.01, r * 1e-3);
      let far = Math.max(D + r * 4 + (view.ortho ? view.dist : 0), Math.hypot(...eye.map((v,i)=>v-(lo[i]+hi[i])/2))+r*4);
      if (!view.ortho) { if (opt.clipNear > 0) near = opt.clipNear; if (opt.clipFar > 0) far = opt.clipFar; far = Math.max(far, near * 1.01); }     // manual clip distances are measured from the camera
      return { MV: lookAt(eye, view.target, [0, 0, 1]), P: perspective(fov, w / h, near, far), eye, w, h, near, far };
    }

    // ---------------------------------------------------------------- selection (points, edges, faces)
    const sel = { mode: "view", point: new Set(), edge: new Map(), face: new Set(), objects:new Set() };
    const edgeKey = (a, b) => (a < b ? a + "-" + b : b + "-" + a);
    const summary = () => ({ mode: sel.mode, objects:[...sel.objects], points: [...sel.point].sort((a, b) => a - b),
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
      const P = raw.P, T = raw.tri, cut = sectionPlane();
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
        if (t > 1e-9 && t < bt) {
          if (cut && cut.n[0] * (o[0] + d[0] * t) + cut.n[1] * (o[1] + d[1] * t) + cut.n[2] * (o[2] + d[2] * t) > cut.d) continue;     // the part beyond the section plane is not there
          bt = t; best = { t, tri: i }; if (anyHit) return best;
        }
      }
      return best;
    }
    function visible(cam, x, y, z) {                                 // nothing solid between the eye and this point
      if (!raw.tri.length) return true;
      const d = [x - cam.eye[0], y - cam.eye[1], z - cam.eye[2]], L = Math.hypot(d[0], d[1], d[2]);
      return !raycast(cam.eye, [d[0] / L, d[1] / L, d[2] / L], L * (1 - 2e-3) - 1e-6, true);
    }
    function projectedPoints(cam) {
      const n = raw.P.length / 3, out = new Float32Array(n * 3), cut = sectionPlane();
      for (let i = 0; i < n; i++) {
        if (cut && cut.n[0] * raw.P[3 * i] + cut.n[1] * raw.P[3 * i + 1] + cut.n[2] * raw.P[3 * i + 2] > cut.d) { out[3 * i + 2] = -1; continue; }     // beyond the section plane: not selectable
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
    function ensurePieces() {                                        // connected pieces: faces that share points (union-find over the triangles)
      if (raw.facePiece) return;
      const np = raw.P.length / 3, parent = new Int32Array(np).map((_, i) => i);
      const find = (a) => { while (parent[a] !== a) { parent[a] = parent[parent[a]]; a = parent[a]; } return a; };
      const T = raw.tri, union = (x, y) => { const a = find(x), b = find(y); if (a !== b) parent[b] = a; };
      for (let i = 0, n = T.length / 3; i < n; i++) { union(T[3 * i], T[3 * i + 1]); union(T[3 * i], T[3 * i + 2]); }
      let nf = 0;
      for (let i = 0; i < raw.triPrim.length; i++) if (raw.triPrim[i] >= nf) nf = raw.triPrim[i] + 1;
      const facePiece = new Int32Array(nf).fill(-1), ids = new Map(), pieces = [];
      for (let i = 0; i < raw.triPrim.length; i++) {
        const f = raw.triPrim[i];
        if (facePiece[f] >= 0) continue;
        const root = find(T[3 * i]);
        if (!ids.has(root)) { ids.set(root, pieces.length); pieces.push([]); }
        facePiece[f] = ids.get(root); pieces[ids.get(root)].push(f);
      }
      raw.facePiece = facePiece; raw.pieces = pieces;
    }
    function piecesOf(faces) {                                       // every face of every piece that one of these faces belongs to
      ensurePieces();
      const hit = new Set(), out = [];
      for (const f of faces) { const p = raw.facePiece[f]; if (p >= 0 && !hit.has(p)) { hit.add(p); for (const g of raw.pieces[p]) out.push(g); } }
      return out;
    }
    function pickFace(cam, sx, sy) {
      const hit = raycast(cam.eye, rayAt(cam, sx, sy));
      return hit ? raw.triPrim[hit.tri] : -1;
    }
    function quadStrip(edge,ring=true) {
      const es=[];for(let i=0;i<raw.edge.length;i+=2)es.push([raw.edge[i],raw.edge[i+1]]);
      const faces=new Map();for(let i=0;i<raw.tri.length;i+=3){const f=raw.triPrim[i/3];if(!faces.has(f))faces.set(f,new Set());for(let j=0;j<3;j++)faces.get(f).add(raw.tri[i+j]);}
      const at=new Map(),incidentFaces=new Map();
      for(const e of es){for(const v of e){if(!at.has(v))at.set(v,[]);at.get(v).push(e);}incidentFaces.set(edgeKey(...e),[...faces.values()].filter(f=>f.has(e[0])&&f.has(e[1])));}
      const incident=e=>incidentFaces.get(edgeKey(...e))||[];
      const found=new Map([[edgeKey(...edge),edge]]),queue=[edge];
      for(let i=0;i<queue.length;i++){
        const e=queue[i],fs=incident(e);if(fs.length>2)continue;
        let next=[];
        if(ring)for(const f of fs){if(f.size!==4)continue;const opposite=es.find(o=>f.has(o[0])&&f.has(o[1])&&!o.some(v=>e.includes(v)));if(opposite)next.push(opposite);}
        else for(const v of e){
          const candidates=(at.get(v)||[]).filter(o=>edgeKey(...o)!==edgeKey(...e)&&incident(o).length<=2);
          // Preserve the unambiguous topological continuation through regular quads.
          const opposite=(at.get(v)||[]).length===4?candidates.filter(o=>!fs.some(f=>f.has(o[0])&&f.has(o[1]))):[];
          if(opposite.length===1){next.push(opposite[0]);continue;}
          // A cap perimeter is a coherent flow even when each vertex has valence 3.
          const perimeter=candidates.filter(o=>fs.some(f=>f.size>4&&f.has(o[0])&&f.has(o[1])));
          if(perimeter.length===1){next.push(perimeter[0]);continue;}
          // Irregular wire/boundary flow: choose one clearly forward direction, never fan out.
          const other=e[0]===v?e[1]:e[0],incoming=[0,1,2].map(k=>raw.P[v*3+k]-raw.P[other*3+k]),L=Math.hypot(...incoming);
          const ranked=candidates.map(o=>{const w=o[0]===v?o[1]:o[0],d=[0,1,2].map(k=>raw.P[w*3+k]-raw.P[v*3+k]),D=Math.hypot(...d);return {edge:o,score:L&&D?dot(incoming,d)/(L*D):-1};}).sort((a,b)=>b.score-a.score);
          if(ranked[0]?.score>.5&&(!ranked[1]||ranked[0].score-ranked[1].score>.15))next.push(ranked[0].edge);
        }
        for(const n of next){const k=edgeKey(...n);if(!found.has(k)){found.set(k,n);queue.push(n);}}
      }return [...found.values()];
    }
    function pathPick(sx,sy,how,method=interaction.pathMode||'direct') {
      const cam=camera(), edges=[];for(let i=0;i<raw.edge.length;i+=2)edges.push([raw.edge[i],raw.edge[i+1]]);
      const faces=new Map();for(let i=0;i<raw.tri.length;i+=3){const f=raw.triPrim[i/3];if(!faces.has(f))faces.set(f,new Set());for(let j=0;j<3;j++)faces.get(f).add(raw.tri[i+j]);}
      const mode=sel.mode, target=mode==='point'?pickPoint(cam,sx,sy,12):mode==='edge'?pickEdge(cam,sx,sy,8):pickFace(cam,sx,sy);
      const key=v=>Array.isArray(v)?edgeKey(...v):v, selected=mode==='point'?[...sel.point]:mode==='edge'?[...sel.edge.values()]:[...sel.face];
      if(target===null||target===undefined||target===-1)return;
      if(!selected.length){api.pickAt(sx,sy,how);return;}
      const values=mode==='point'?[...new Set(edges.flat())]:mode==='edge'?edges:[...faces.keys()], adjacency=new Map(values.map(v=>[key(v),[]]));
      const connect=(a,b)=>{adjacency.get(key(a))?.push(b);adjacency.get(key(b))?.push(a);};
      if(mode==='point')edges.forEach(e=>connect(...e));
      else if(mode==='edge'){const at=new Map();for(const e of edges)for(const v of e){if(!at.has(v))at.set(v,[]);at.get(v).push(e);}for(const es of at.values())for(let i=0;i<es.length;i++)for(let j=i+1;j<es.length;j++)connect(es[i],es[j]);}
      else {const at=new Map();for(const [f,vs]of faces)for(const e of edges)if(vs.has(e[0])&&vs.has(e[1])){const k=edgeKey(...e);if(!at.has(k))at.set(k,[]);at.get(k).push(f);}for(const fs of at.values())for(let i=0;i<fs.length;i++)for(let j=i+1;j<fs.length;j++)connect(fs[i],fs[j]);}
      const start=selected.at(-1);
      if(method==='direct'&&window.MiniSelectionMath){
        const position=v=>{const ids=mode==='point'?[v]:mode==='edge'?v:[...faces.get(v)];return [0,1,2].map(k=>ids.reduce((sum,i)=>sum+raw.P[i*3+k],0)/ids.length);};
        const path=MiniSelectionMath.directPath({adjacency,key,position,start,target});
        if(path.length)change({[mode==='point'?'points':mode==='edge'?'edges':'faces']:path},'add');return;
      }
      const queue=[start],previous=new Map([[key(start),null]]),items=new Map([[key(start),start]]);
      for(let i=0;i<queue.length&&!previous.has(key(target));i++)for(const v of adjacency.get(key(queue[i]))||[])if(!previous.has(key(v))){previous.set(key(v),key(queue[i]));items.set(key(v),v);queue.push(v);}
      if(!previous.has(key(target)))return;const path=[];for(let k=key(target);k!==null;k=previous.get(k))path.push(items.get(k));
      change({[mode==='point'?'points':mode==='edge'?'edges':'faces']:path},'add');
    }
    function boxPick(cam, x0, y0, x1, y1, region=null) {                          // everything inside the rectangle (X-ray)
      const lo = [Math.min(x0, x1), Math.min(y0, y1)], hi = [Math.max(x0, x1), Math.max(y0, y1)];
      const inside = (x, y) => region?MiniSelectionMath.contains(region,x,y):x >= lo[0] && x <= hi[0] && y >= lo[1] && y <= hi[1];
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
        out.faces = sel.mode === "piece" ? piecesOf(faces) : [...faces];
      }
      if(sel.mode==="piece")for(let i=0;i<sp.length/3;i++)if(sp[3*i+2]>=0&&inside(sp[3*i],sp[3*i+1]))out.points.push(i);
      return out;
    }
    function activeObject(){return api.previewObjects?.find(o=>o.node===api.activeObject)||api.previewObjects?.[0];}
    function change(found, how) {                                    // found: {points, edges, faces}; how: replace | add | subtract
      if(sel.mode==='piece'&&raw&&api.previewObjects?.length){
        const points=new Set(found.points||[]),faces=new Set(piecesOf(found.faces||[])),objects=new Set();
        for(const e of found.edges||[])for(const p of e)points.add(p);
        const explicit=(found.objects||[]).length&&!points.size&&!faces.size;
        for(const o of api.previewObjects){
          const hit=[...points].some(p=>p>=o.point_start&&p<o.point_start+o.point_count),hasFaces=raw.triPrim.some(f=>f>=o.prim_start&&f<o.prim_start+o.prim_count);
          if(explicit&&found.objects.includes(o.node)||hit&&!hasFaces){for(let p=o.point_start;p<o.point_start+o.point_count;p++)points.add(p);for(let f=o.prim_start;f<o.prim_start+o.prim_count;f++)faces.add(f);}
        }
        for(let i=0;i<raw.tri.length;i+=3)if(faces.has(raw.triPrim[i/3]))for(let c=0;c<3;c++)points.add(raw.tri[i+c]);
        for(const o of api.previewObjects)if([...points].some(p=>p>=o.point_start&&p<o.point_start+o.point_count))objects.add(o.node);
        const edges=[];for(let i=0;i<raw.edge.length;i+=2)if(points.has(raw.edge[i])&&points.has(raw.edge[i+1]))edges.push([raw.edge[i],raw.edge[i+1]]);
        found={points:[...points],faces:[...faces],edges,objects:[...objects]};
      }
      if(how!=="subtract"){
        const f=found.faces?.at(-1),p=found.points?.at(-1)??found.edges?.at(-1)?.[1];
        const object=api.previewObjects?.find(o=>f!==undefined?f>=o.prim_start&&f<o.prim_start+o.prim_count:p!==undefined&&p>=o.point_start&&p<o.point_start+o.point_count);
        if(object)api.activeObject=object.node;
      }
      if (how === "replace") { sel.point.clear(); sel.edge.clear(); sel.face.clear();sel.objects.clear(); }
      for (const i of found.points || []) how === "subtract" ? sel.point.delete(i) : sel.point.add(i);
      for (const [a, b] of found.edges || []) how === "subtract" ? sel.edge.delete(edgeKey(a, b)) : sel.edge.set(edgeKey(a, b), a < b ? [a, b] : [b, a]);
      for (const f of found.faces || []) how === "subtract" ? sel.face.delete(f) : sel.face.add(f);
      for(const id of found.objects||[])how==="subtract"?sel.objects.delete(id):sel.objects.add(id);
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


    function buildNormals() {
      nrmDirty = false; nrmCount = 0;
      if (!raw || !raw.tri.length) return;
      const T = raw.tri, P = raw.P, TP = raw.triPrim;
      let np = 0;
      for (let i = 0; i < TP.length; i++) if (TP[i] >= np) np = TP[i] + 1;
      const acc = new Float64Array(np * 7);                                  // area-weighted normal sum, centroid sum, weight sum per primitive
      for (let i = 0, n = T.length / 3; i < n; i++) {
        const a = T[3 * i] * 3, b = T[3 * i + 1] * 3, c = T[3 * i + 2] * 3;
        const e1x = P[b] - P[a], e1y = P[b + 1] - P[a + 1], e1z = P[b + 2] - P[a + 2], e2x = P[c] - P[a], e2y = P[c + 1] - P[a + 1], e2z = P[c + 2] - P[a + 2];
        const nx = e1y * e2z - e1z * e2y, ny = e1z * e2x - e1x * e2z, nz = e1x * e2y - e1y * e2x, w = Math.hypot(nx, ny, nz), k = TP[i] * 7;
        acc[k] += nx; acc[k + 1] += ny; acc[k + 2] += nz;
        acc[k + 3] += w * (P[a] + P[b] + P[c]) / 3; acc[k + 4] += w * (P[a + 1] + P[b + 1] + P[c + 1]) / 3; acc[k + 5] += w * (P[a + 2] + P[b + 2] + P[c + 2]) / 3; acc[k + 6] += w;
      }
      const [lo, hi] = data.bounds, L = Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) * 0.04, 1e-6);
      const step = Math.max(1, Math.ceil(np / MAX_NORMALS)), lines = [], tips = [];
      for (let q = 0; q < np; q += step) {
        const k = q * 7, w = acc[k + 6], len = Math.hypot(acc[k], acc[k + 1], acc[k + 2]);
        if (w <= 0 || len <= 0) continue;
        const cx = acc[k + 3] / w, cy = acc[k + 4] / w, cz = acc[k + 5] / w, tx = cx + acc[k] / len * L, ty = cy + acc[k + 1] / len * L, tz = cz + acc[k + 2] / len * L;
        lines.push(cx, cy, cz, tx, ty, tz); tips.push(tx, ty, tz);
      }
      gl.bindBuffer(gl.ARRAY_BUFFER, nrmBuf); gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(lines), gl.STATIC_DRAW);
      gl.bindBuffer(gl.ARRAY_BUFFER, tipBuf); gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(tips), gl.STATIC_DRAW);
      nrmCount = tips.length / 3;
      nrmLines = new Float32Array(lines);
      api.normalCount = nrmCount;
    }
    function drawNormals(cam) {
      if (nrmDirty) buildNormals();
      if (!nrmCount) return;
      gl.useProgram(colorProg.p);
      gl.uniformMatrix4fv(colorProg.u.uMV, false, cam.MV); gl.uniformMatrix4fv(colorProg.u.uP, false, cam.P);
      gl.uniform1f(colorProg.u.uBias, 0.0006); gl.uniform4f(colorProg.u.uColor, 0.25, 0.85, 0.95, 1);
      gl.bindVertexArray(nrmVao); gl.drawArrays(gl.LINES, 0, nrmCount * 2);
      gl.uniform1f(colorProg.u.uPointSize, 5);
      gl.bindVertexArray(tipVao); gl.drawArrays(gl.POINTS, 0, nrmCount);
      gl.uniform1f(colorProg.u.uPointSize, 4);
    }

    // ---------------------------------------------------------------- the section plane, and what is outside the view
    // The plane keeps what satisfies dot(n, p) <= d. Anything beyond it is hidden by the shaders and cannot be picked or painted (isClipped).
    function sectionPlane() {
      const s = opt.section;
      if (!s || !s.on) return null;
      const n = [0, 0, 0]; n[s.axis] = s.flip ? -1 : 1;
      return { n, d: s.flip ? -s.offset : s.offset };
    }
    function isClipped(x, y, z) { const p = sectionPlane(); return !!p && p.n[0] * x + p.n[1] * y + p.n[2] * z > p.d; }
    function mat4mul(a, b) {                                                           // column-major a * b
      const o = new Float32Array(16);
      for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) { let v = 0; for (let k = 0; k < 4; k++) v += a[k * 4 + r] * b[c * 4 + k]; o[c * 4 + r] = v; }
      return o;
    }
    function frustumPlanes(cam) {                                                      // [a, b, c, d] with a*x + b*y + c*z + d >= 0 inside: left, right, bottom, top, near, far (+ the section plane)
      const m = mat4mul(cam.P, cam.MV), row = (i) => [m[i], m[4 + i], m[8 + i], m[12 + i]];
      const r0 = row(0), r1 = row(1), r2 = row(2), r3 = row(3), add = (a, b, k) => a.map((v, i) => v + k * b[i]);
      const planes = [add(r3, r0, 1), add(r3, r0, -1), add(r3, r1, 1), add(r3, r1, -1), add(r3, r2, 1), add(r3, r2, -1)];
      const sp = sectionPlane();
      if (sp) planes.push([-sp.n[0], -sp.n[1], -sp.n[2], sp.d]);
      return planes;
    }
    function boxOutside(planes, t, o) {                                                // an axis-aligned box (min at t[o..o+2], max at t[o+3..o+5]) entirely outside one plane
      for (const [a, b, c, d] of planes) {
        const x = a > 0 ? t[o + 3] : t[o], y = b > 0 ? t[o + 4] : t[o + 1], z = c > 0 ? t[o + 5] : t[o + 2];
        if (a * x + b * y + c * z + d < 0) return true;
      }
      return false;
    }
    const stats = { tris: 0, trisTotal: 0, chunks: 0, chunksDrawn: 0, drawMs: 0, moving: false, scale: 1, draws: 0 };
    function triangleRuns(cam) {                                                       // [{first, n}] of the visible runs of triangles, or null = draw them all
      const t = raw && raw.chunks;
      if (!t || !opt.cull) return null;
      const planes = frustumPlanes(cam), n = t.length / 8, runs = [];
      let cur = null, drawn = 0;
      const k = cam.P[5] * cam.h, m = cam.MV;
      for (let c = 0; c < n; c++) {
        const o = 8 * c + 2;
        let skip = boxOutside(planes, t, o);
        if (!skip && opt.cullPx > 0) {                                                 // too small on screen to matter
          const cx = (t[o] + t[o + 3]) / 2, cy = (t[o + 1] + t[o + 4]) / 2, cz = (t[o + 2] + t[o + 5]) / 2, r = Math.hypot(t[o + 3] - t[o], t[o + 4] - t[o + 1], t[o + 5] - t[o + 2]) / 2;
          const d = -(m[2] * cx + m[6] * cy + m[10] * cz + m[14]);
          if (d > r && r * k / d < opt.cullPx) skip = true;
        }
        if (skip) continue;
        const first = t[8 * c], cnt = t[8 * c + 1];
        if (cur && cur.first + cur.n === first) cur.n += cnt; else runs.push(cur = { first, n: cnt });
        drawn += cnt;
      }
      stats.chunks = n; stats.chunksDrawn = runs.reduce((a, r) => a + Math.ceil(r.n / 8192), 0); stats.tris = drawn;
      return runs;
    }

    // ---------------------------------------------------------------- instances drawn as impostors (the engine sends each shape once at three levels
    // of detail and a transform per instance: minihou/impostors.py). Each camera move picks a level per instance from its size on screen and
    // keeps the total within a triangle budget; instances outside the view are not drawn at all.
    const inst = { on: false, head: null, N: 0, M: null, proto: null, sph: null, col: null, px: null, vis: null, posBuf: gl.createBuffer(), idxBuf: gl.createBuffer(),
      buckets: new Map(), dots: { vao: gl.createVertexArray(), buf: gl.createBuffer(), n: 0, data: null }, sig: null, last: 0, timer: 0,
      stats: { count: 0, visible: 0, lod: [0, 0, 0, 0], tris: 0, budget: 0, scale: 1, updates: 0, ms: 0 } };
    gl.bindVertexArray(inst.dots.vao); gl.bindBuffer(gl.ARRAY_BUFFER, inst.dots.buf); gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0); gl.bindVertexArray(null);
    const BOX_PX = 3, DOT_PX = 0.8, T0_PX = 70, T1_PX = 22;               // diameters, in pixels, where the levels change (before the bias and the budget)
    function clearInstances() {
      inst.on = false; inst.head = null; inst.N = 0; inst.M = inst.proto = inst.sph = inst.col = inst.px = inst.vis = null; inst.sig = null;
      for (const b of inst.buckets.values()) { b.count = 0; }
      inst.dots.n = 0; Object.assign(inst.stats, { count: 0, visible: 0, lod: [0, 0, 0, 0], tris: 0 });
    }
    function uploadInstances(head, arrays) {
      clearInstances();
      const h = head.instances;
      if (!h || !arrays.inst_M || !h.count) return;
      inst.on = true; inst.head = h; inst.N = h.count; inst.M = arrays.inst_M; inst.proto = arrays.inst_proto; inst.sph = arrays.inst_sph; inst.col = arrays.inst_col || null;
      inst.px = new Float32Array(inst.N); inst.vis = new Uint8Array(inst.N);
      gl.bindBuffer(gl.ARRAY_BUFFER, inst.posBuf); gl.bufferData(gl.ARRAY_BUFFER, arrays.ip_P, gl.STATIC_DRAW);
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, inst.idxBuf); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, arrays.ip_tri, gl.STATIC_DRAW);
      for (const b of inst.buckets.values()) { gl.deleteVertexArray(b.vao); gl.deleteBuffer(b.buf); }
      inst.buckets.clear();
      inst.stats.count = inst.N;
    }
    function bucketFor(m) {
      let b = inst.buckets.get(m);
      if (b) return b;
      const vao = gl.createVertexArray(), buf = gl.createBuffer(), stride = 60;
      gl.bindVertexArray(vao);
      gl.bindBuffer(gl.ARRAY_BUFFER, inst.posBuf); gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0);
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      for (const [loc, n, off] of [[1, 4, 0], [2, 4, 16], [3, 4, 32], [4, 3, 48]]) { gl.enableVertexAttribArray(loc); gl.vertexAttribPointer(loc, n, gl.FLOAT, false, stride, off); gl.vertexAttribDivisor(loc, 1); }
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, inst.idxBuf);
      gl.bindVertexArray(null);
      b = { vao, buf, count: 0, data: null };
      inst.buckets.set(m, b);
      return b;
    }
    function updateInstances(cam) {
      const t0 = performance.now(), h = inst.head, N = inst.N, sph = inst.sph, m = cam.MV, P = cam.P, px = inst.px, vis = inst.vis;
      const k = P[5] * cam.h, kx = P[0], ky = P[5], near = cam.near, protos = h.protos, proto = inst.proto, sp = sectionPlane();
      let nvis = 0;
      for (let i = 0; i < N; i++) {
        const cx = sph[4 * i], cy = sph[4 * i + 1], cz = sph[4 * i + 2], r = sph[4 * i + 3];
        const vx = m[0] * cx + m[4] * cy + m[8] * cz + m[12], vy = m[1] * cx + m[5] * cy + m[9] * cz + m[13], d = -(m[2] * cx + m[6] * cy + m[10] * cz + m[14]);
        if (d + r < near || Math.abs(vx) * kx > d + r * kx * 1.5 || Math.abs(vy) * ky > d + r * ky * 1.5) { vis[i] = 0; continue; }      // behind or outside the view
        if (sp && sp.n[0] * cx + sp.n[1] * cy + sp.n[2] * cz - sp.d > r) { vis[i] = 0; continue; }                                    // wholly beyond the section plane
        vis[i] = 1; nvis++;
        px[i] = r * k / Math.max(d, 1e-9);                                                  // the instance's diameter on screen, in CSS pixels
      }
      // thresholds grow (scale) until the triangles of every visible instance fit the budget
      const bias = Math.max(0.05, opt.instBias || 1), budget = Math.max(1000, opt.instTris || 1500000);
      let scale = 1, tris = 0;
      const lodOf = (p, f) => (p * bias >= T0_PX * f ? 0 : p * bias >= T1_PX * f ? 1 : p * bias >= BOX_PX * f ? 2 : p * bias >= DOT_PX ? 3 : 4);
      for (let pass = 0; pass < 14; pass++) {
        tris = 0;
        for (let i = 0; i < N; i++) { if (!vis[i]) continue; const c = lodOf(px[i], scale); if (c < 3) tris += protos[proto[i]].tris[c]; }
        if (tris <= budget) break;
        scale *= 1.6;
      }
      const counts = new Map(), lod = [0, 0, 0, 0];
      for (let i = 0; i < N; i++) {
        if (!vis[i]) continue;
        const c = lodOf(px[i], scale);
        if (c === 4) continue;
        lod[c]++;
        if (c < 3) { const mesh = protos[proto[i]].lods[c]; counts.set(mesh, (counts.get(mesh) || 0) + 1); }
      }
      const fill = new Map(), M = inst.M, col = inst.col;
      for (const [mesh, n] of counts) {
        const b = bucketFor(mesh);
        if (!b.data || b.data.length < n * 15) b.data = new Float32Array(Math.max(n * 15, 15 * 64));
        b.count = 0; fill.set(mesh, b);
      }
      for (const [mesh, b] of inst.buckets) if (!fill.has(mesh)) b.count = 0;
      let nd = 0;
      if (!inst.dots.data || inst.dots.data.length < lod[3] * 3) inst.dots.data = new Float32Array(Math.max(lod[3] * 3, 3 * 64));
      for (let i = 0; i < N; i++) {
        if (!vis[i]) continue;
        const c = lodOf(px[i], scale);
        if (c === 4) continue;
        if (c === 3) { inst.dots.data[3 * nd] = sph[4 * i]; inst.dots.data[3 * nd + 1] = sph[4 * i + 1]; inst.dots.data[3 * nd + 2] = sph[4 * i + 2]; nd++; continue; }
        const b = fill.get(protos[proto[i]].lods[c]), o = b.count * 15;
        for (let j = 0; j < 12; j++) b.data[o + j] = M[12 * i + j];
        if (col) { b.data[o + 12] = col[3 * i]; b.data[o + 13] = col[3 * i + 1]; b.data[o + 14] = col[3 * i + 2]; } else { b.data[o + 12] = b.data[o + 13] = b.data[o + 14] = 0; }
        b.count++;
      }
      for (const b of fill.values()) { gl.bindBuffer(gl.ARRAY_BUFFER, b.buf); gl.bufferData(gl.ARRAY_BUFFER, b.data.subarray(0, b.count * 15), gl.DYNAMIC_DRAW); }
      inst.dots.n = nd;
      if (nd) { gl.bindBuffer(gl.ARRAY_BUFFER, inst.dots.buf); gl.bufferData(gl.ARRAY_BUFFER, inst.dots.data.subarray(0, nd * 3), gl.DYNAMIC_DRAW); }
      Object.assign(inst.stats, { count: N, visible: nvis, lod, tris, budget, scale, updates: inst.stats.updates + 1, ms: Math.round(performance.now() - t0) });
    }
    function drawInstances(cam) {
      if (!inst.on) return;
      const sig = [...cam.MV, ...cam.P, opt.instBias, opt.instTris, cam.w, cam.h, JSON.stringify(opt.section)].join(",");
      if (sig !== inst.sig) {
        const now = performance.now();
        if (inst.N > 150000 && inst.sig !== null && now - inst.last < 70) { if (!inst.timer) inst.timer = setTimeout(() => { inst.timer = 0; schedule(); }, 80); }   // a huge set refreshes a few times a second while the camera moves
        else { updateInstances(cam); inst.sig = sig; inst.last = performance.now(); }
      }
      gl.enable(gl.DEPTH_TEST);
      gl.useProgram(instProg.p);
      gl.uniformMatrix4fv(instProg.u.uMV, false, cam.MV); gl.uniformMatrix4fv(instProg.u.uP, false, cam.P);
      const dim=opt.isolated&&api.activeObject!==(api.primaryNode??api.previewObjects?.[0]?.node)?Math.max(0,Math.min(1,opt.isolated_knockback??.75)):0;gl.uniform1f(instProg.u.uDim,dim);
      gl.uniform3fv(instProg.u.uBase, [0.62, 0.68, 0.76]); gl.uniform1f(instProg.u.uUseCol, inst.col ? 1 : 0); gl.uniform1f(instProg.u.uLighting, opt.lighting);
      const meshes = inst.head.meshes;
      for (const [mesh, b] of inst.buckets) {
        if (!b.count) continue;
        gl.bindVertexArray(b.vao);
        gl.drawElementsInstanced(gl.TRIANGLES, meshes[mesh].t_n * 3, gl.UNSIGNED_INT, meshes[mesh].t_off * 12, b.count);
      }
      if (inst.dots.n) {
        gl.useProgram(colorProg.p); gl.bindVertexArray(inst.dots.vao);
        gl.uniformMatrix4fv(colorProg.u.uMV, false, cam.MV); gl.uniformMatrix4fv(colorProg.u.uP, false, cam.P);
        gl.uniform1f(colorProg.u.uBias, 0); gl.uniform1f(colorProg.u.uPointSize, 2); gl.uniform4f(colorProg.u.uColor, 0.62*(1-dim)+.22*dim, 0.68*(1-dim)+.22*dim, 0.76*(1-dim)+.22*dim, 1);
        gl.drawArrays(gl.POINTS, 0, inst.dots.n);
      }
      gl.bindVertexArray(null);
    }
    function instanceStats() { return { ...inst.stats, lod: inst.stats.lod.slice(), on: inst.on, kind: inst.head ? inst.head.kind : null, approximate: !!(inst.head && inst.head.approximate),
      replaces: !!(inst.head && inst.head.replaces), total: inst.head ? inst.head.total : 0, fullTris: inst.head ? inst.head.full_tris : 0 }; }

    const NAV_SCALE = { full: 1, balanced: 0.6, fast: 0.45 };                 // the share of the resolution drawn while the camera moves
    let lastCam = null, settleTimer = 0;
    function camMoved(cam) {                                                // did the camera change since the last frame?
      const now = new Float32Array(32); now.set(cam.MV, 0); now.set(cam.P, 16);
      let moved = false;
      if (lastCam) for (let i = 0; i < 32; i++) if (lastCam[i] !== now[i]) { moved = true; break; }
      lastCam = now;
      return moved;
    }
    function draw() {
      raf = 0;
      const t0 = performance.now();
      const cam0 = camera(), moving = camMoved(cam0), navScale = moving ? (NAV_SCALE[opt.navQuality] || 1) : 1, skipEdges = moving && opt.navQuality === "fast";
      if (moving && opt.navQuality !== "full") { clearTimeout(settleTimer); settleTimer = setTimeout(() => { settleTimer = 0; schedule(); }, 200); }     // one full-quality frame once it stops
      resize(navScale);
      Object.assign(stats, { moving, scale: navScale, draws: stats.draws + 1 });
      gl.viewport(0, 0, canvas.width, canvas.height);
      const bg = opt.background.match(/[0-9a-f]{2}/gi).map(x => parseInt(x, 16) / 255);
      gl.clearColor(...bg, 1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      api.draws++;
      const cam = cam0, MV = cam.MV, P = cam.P;
      const sp = sectionPlane();
      for (const pr of [mesh, flat, colorProg, edgeProg, instProg]) {                          // the section plane, for every program that draws geometry
        gl.useProgram(pr.p); gl.uniform4f(pr.u.uClip, sp ? sp.n[0] : 0, sp ? sp.n[1] : 0, sp ? sp.n[2] : 0, sp ? sp.d : 0); gl.uniform1f(pr.u.uClipOn, sp ? 1 : 0);
      }
      if (opt.grid) {
        gl.disable(gl.DEPTH_TEST); gl.depthMask(false);
        gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.useProgram(gridProg.p); gl.bindVertexArray(null);
        gl.uniformMatrix4fv(gridProg.u.uMV, false, MV); gl.uniformMatrix4fv(gridProg.u.uP, false, P);
        gl.uniform3fv(gridProg.u.uEye, cam.eye); gl.uniform3fv(gridProg.u.uTarget, view.target);
        gl.uniform1f(gridProg.u.uSpacing, opt.gridSpacing || Math.pow(10, Math.floor(Math.log10(Math.max(view.dist / 5, 1e-8)))));
        gl.uniform1f(gridProg.u.uRange, view.dist * 12);
        gl.uniform1f(gridProg.u.uOpacity, opt.gridOpacity); gl.uniform1f(gridProg.u.uAxes, opt.axes ? 1 : 0);
        gl.drawArrays(gl.TRIANGLES, 0, 3);
        gl.disable(gl.BLEND); gl.depthMask(true);
      }
      renderGizmo(cam);
      if (api.onOverlayDraw) api.onOverlayDraw();
      if (!data.npoints && !templateRaw && !inst.on) return;
      gl.enable(gl.DEPTH_TEST);
      gl.bindVertexArray(vao);
      const setup = (pr, base, useCol, alpha) => {
        gl.useProgram(pr.p);
        gl.uniformMatrix4fv(pr.u.uMV, false, MV); gl.uniformMatrix4fv(pr.u.uP, false, P);
        gl.uniform3fv(pr.u.uBase, base); gl.uniform1f(pr.u.uUseCol, useCol && data.hasCd ? 1 : 0);
        gl.uniform1f(pr.u.uPointSize, opt.pointSize);
        gl.uniform1f(pr.u.uIsolate,opt.isolated?Math.max(0,Math.min(1,opt.isolated_knockback??.75)):0);
        gl.uniform1f(pr.u.uOwnerMode,0);const active=activeObject();gl.uniform1f(pr.u.uActiveStart,active?.point_start??0);gl.uniform1f(pr.u.uActiveEnd,active?active.point_start+active.point_count:data.npoints);
        if (pr.u.uLighting) gl.uniform1f(pr.u.uLighting, opt.lighting); gl.uniform1f(pr.u.uBias, 0);
        if (pr.u.uAlpha) gl.uniform1f(pr.u.uAlpha, alpha === undefined ? 1 : alpha);
        if (data.hasCd) gl.enableVertexAttribArray(1); else { gl.disableVertexAttribArray(1); gl.vertexAttrib3f(1, 1, 1, 1); }
      };
      if (opt.shaded && data.ntris) {
        const primView = attrView && attrView.domain === "prim" && primCount === data.ntris * 3;
        if (primView) { gl.bindVertexArray(primVao); data.hasCd = true; }
        setup(mesh, [0.62, 0.68, 0.76], true);
        if(primView){const a=activeObject(),start=a?.prim_start??0,end=a?start+a.prim_count:1e30;gl.uniform1f(mesh.u.uOwnerMode,1);gl.uniform1f(mesh.u.uActiveStart,start);gl.uniform1f(mesh.u.uActiveEnd,end);}
        gl.enable(gl.POLYGON_OFFSET_FILL); gl.polygonOffset(1, 1);
        const runs = triangleRuns(cam);                                                       // null = every triangle (no chunk table, or culling is off)
        stats.trisTotal = data.ntris;
        if (!runs) { stats.tris = data.ntris; stats.chunks = stats.chunksDrawn = 0; }
        if (primView) {
          if (runs) for (const r of runs) gl.drawArrays(gl.TRIANGLES, r.first * 3, r.n * 3); else gl.drawArrays(gl.TRIANGLES, 0, primCount);
          data.hasCd = !!cd0; gl.bindVertexArray(vao);
        } else {
          gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, triBuf);
          if (runs) for (const r of runs) gl.drawElements(gl.TRIANGLES, r.n * 3, gl.UNSIGNED_INT, r.first * 12); else gl.drawElements(gl.TRIANGLES, data.ntris * 3, gl.UNSIGNED_INT, 0);
        }
        gl.disable(gl.POLYGON_OFFSET_FILL);
      }
      if ((opt.edges || sel.mode === "edge") && data.nedges && !skipEdges) {
        const over = opt.shaded && data.ntris;                  // over a shaded surface the edges are soft dark lines
        setup(flat, over ? [0.04, 0.05, 0.06] : [0.75, 0.82, 0.9], false, over ? (sel.mode === "edge" ? 0.6 : 0.35) : 1);
        gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, edgeBuf); gl.drawElements(gl.LINES, data.nedges * 2, gl.UNSIGNED_INT, 0);
        gl.disable(gl.BLEND);
      }
      if (opt.points || sel.mode === "point" || (!data.ntris && !data.nedges && !inst.on)) {
        setup(flat, sel.mode === "point" ? [0.9, 0.93, 0.98] : [0.95, 0.75, 0.3], true);
        gl.uniform1f(flat.u.uBias, 0.0004);
        gl.drawArrays(gl.POINTS, 0, data.npoints);
      }
      if(!opt.points&&sel.mode!=="point")for(const o of api.previewObjects||[])if(!o.prim_count&&o.point_count&&!(inst.on&&o.node===api.primaryNode)){setup(flat,[.95,.75,.3],true);gl.drawArrays(gl.POINTS,o.point_start,o.point_count);}
      drawInstances(cam);
      gl.bindVertexArray(vao);
      if(templateRaw){
        gl.bindVertexArray(templateVao);setup(flat,[0.72,0.38,0.52],false,0.8);gl.uniform1f(flat.u.uActiveEnd,0);
        gl.uniform1f(flat.u.uBias,0.0003);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,templateEdge);gl.drawElements(gl.LINES,templateRaw.edge.length,gl.UNSIGNED_INT,0);
        if(!templateRaw.edge.length)gl.drawArrays(gl.POINTS,0,templateRaw.P.length/3);
        gl.bindVertexArray(vao);
      }
      if (aedCount) {
        gl.enable(gl.BLEND); gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        gl.useProgram(edgeProg.p); gl.bindVertexArray(aedVao);
        gl.uniformMatrix4fv(edgeProg.u.uMV, false, cam.MV); gl.uniformMatrix4fv(edgeProg.u.uP, false, cam.P);
        gl.uniform2f(edgeProg.u.uViewport, canvas.clientWidth, canvas.clientHeight); gl.uniform1f(edgeProg.u.uWidth, 3.2);
        gl.uniform1f(edgeProg.u.uBias, 0.0015); gl.uniform4f(edgeProg.u.uColor, 0.95, 0.25, 0.75, 1);
        gl.drawElements(gl.TRIANGLES, aedCount, gl.UNSIGNED_INT, 0);
        gl.disable(gl.BLEND); gl.bindVertexArray(vao);
      }
      drawSelection(cam);
      if (opt.normals) { gl.bindVertexArray(null); drawNormals(cam); }
      gl.bindVertexArray(null);
      renderHandles(cam);
      stats.drawMs = Math.round((performance.now() - t0) * 10) / 10;
    }
    function schedule() { if (!raf) raf = requestAnimationFrame(draw); }

    // ---------------------------------------------------------------- handles (gizmos): draggable arrows for node parameters
    const SVGNS = "http://www.w3.org/2000/svg";
    const hsvg = document.createElementNS(SVGNS, "svg");
    hsvg.setAttribute("class", "pv-handles");
    canvas.parentElement.append(hsvg);
    let handles = [], pendingHandles = null, handleDrag = null, selHandles = [], pendingSel = null;   // selHandles: the move/rotate/scale gizmo of a selection (they win while something is selected)
    const AXIS_COLOUR = { 0: "#ff5d5d", 1: "#4cc38a", 2: "#4aa3ff" };
    const IM = window.MiniInteraction;
    const interaction = { orientation: 'global', local: [0,0,0], cursor: [0,0,0], custom: IM.identity(), snap: false,
        proportional_enabled:false,proportional_radius:1,proportional_falloff:'smooth',proportional_connected:false,proportional_projected:false, element: 'vertex', target: 'center', moveStep: 0.1, angleStep: 15, scaleStep: 0.1, backface: false, includeSelected: false };
    const feedback = document.createElement('div');feedback.className='pv-drag-feedback';feedback.hidden=true;canvas.parentElement.append(feedback);
    let snapData = null, snapExclude = new Set(), snapMark = null, endingDrag=false;
    function orientationFrame() {
      if(interaction.orientation==='local')return IM.frame(interaction.local);
      if(interaction.orientation==='cursor')return IM.frame(interaction.cursor);
      if(interaction.orientation==='custom')return interaction.custom;
      if(interaction.orientation==='view'){const m=camera().MV;return [[m[0],m[4],m[8]],[m[1],m[5],m[9]],[m[2],m[6],m[10]]];}
      if(interaction.orientation==='normal') {
        const info=api.selectionInfo();if(!info||!raw)return IM.frame(interaction.local);
        const ids=new Set(info.points),n=[0,0,0];let tangent=null;
        for(let i=0;i<raw.tri.length;i+=3){const v=Array.from(raw.tri.slice(i,i+3));if(sel.mode==='face'||sel.mode==='piece'){if(!sel.face.has(raw.triPrim[i/3]))continue;}else if(!v.some(k=>ids.has(k)))continue;
          const [a,b,c]=v.map(k=>Array.from(raw.P.slice(k*3,k*3+3))),nn=cross(sub(b,a),sub(c,a));nn.forEach((x,j)=>n[j]+=x);}
        if(sel.mode==='edge'&&sel.edge.size){const [a,b]=[...sel.edge.values()].at(-1);tangent=sub(Array.from(raw.P.slice(b*3,b*3+3)),Array.from(raw.P.slice(a*3,a*3+3)));}
        return IM.length(n)<1e-9?IM.frame(interaction.local):IM.normalFrame(n,tangent);
      } return IM.identity();
    }
    function planePoint(sx,sy,anchor,normal=null) {
      const cam=camera(),n=normal||norm(sub(cam.eye,view.target)),ray=rayAt(cam,sx,sy),den=dot(ray,n);
      if(Math.abs(den)<1e-8)return null;return IM.add(cam.eye,IM.mul(ray,dot(sub(anchor,cam.eye),n)/den));
    }
    function snapHit(sx,sy,anchor=[0,0,0]) {
      const sources=[snapData||raw,templateRaw].filter(Boolean);
      for(const element of interaction.element.split(',').sort((a,b)=>(a==='face')-(b==='face'))){
        let bestHit=null,bestDistance=18;
        for(const source of sources){const hit=snapSource(source,sx,sy,anchor,source===templateRaw,element);if(hit){const p=toScreen(camera(),...hit),d=p?Math.hypot(p[0]-sx,p[1]-sy):Infinity;if(d<bestDistance){bestDistance=d;bestHit=hit;}}}
        if(bestHit)return bestHit;
      }return null;
    }
    function snapSource(source,sx,sy,anchor,reference,element=interaction.element) {
      if(!source)return null;const cam=camera(),P=source.P;
      const point=i=>Array.from(P.slice(i*3,i*3+3));let best=null,dist=18;
      const test=p=>{const q=toScreen(cam,...p);if(!q)return;const d=Math.hypot(q[0]-sx,q[1]-sy);if(d<dist){dist=d;best=p;}};
      const excluded=i=>!reference&&!interaction.includeSelected&&snapExclude.has(i);
      if(element==='vertex')for(let i=0;i<P.length/3;i++){if(!excluded(i))test(point(i));}
      if(['edge','midpoint','perpendicular'].includes(element))for(let i=0;i<source.edge.length;i+=2){const a=source.edge[i],b=source.edge[i+1];if(excluded(a)||excluded(b))continue;const p=point(a),q=point(b);
        if(element==='midpoint')test(IM.mul(IM.add(p,q),0.5));
        else if(element==='perpendicular'){const d=sub(q,p),t=dot(sub(anchor,p),d)/(dot(d,d)||1);if(t>=0&&t<=1)test(IM.add(p,IM.mul(d,t)));}
        else{const u=toScreen(cam,...p),v=toScreen(cam,...q);if(!u||!v)continue;const d=[v[0]-u[0],v[1]-u[1]],t=Math.max(0,Math.min(1,((sx-u[0])*d[0]+(sy-u[1])*d[1])/(d[0]*d[0]+d[1]*d[1]||1)));
          // Perspective-correct interpolation of the closest projected edge point.
          const k=(t/v[2])/((1-t)/u[2]+t/v[2]);test(IM.add(p,IM.mul(sub(q,p),k)));}}
      if(element==='face') {const ray=rayAt(cam,sx,sy);let nearest=Infinity;
        for(let i=0;i<source.tri.length;i+=3){const ids=Array.from(source.tri.slice(i,i+3));if(ids.some(excluded))continue;const [a,b,c]=ids.map(point),ab=sub(b,a),ac=sub(c,a),h=cross(ray,ac),det=dot(ab,h);if(Math.abs(det)<1e-10||(interaction.backface&&det<0))continue;const s=sub(cam.eye,a),u=dot(s,h)/det,q=cross(s,ab),v=dot(ray,q)/det,t=dot(ac,q)/det;
          if(u>=0&&v>=0&&u+v<=1&&t>0&&t<nearest){nearest=t;best=IM.add(cam.eye,IM.mul(ray,t));}}
      } return best;
    }
    function snapPosition(position,sx,sy,ctrl,anchor,axis,frame=orientationFrame()) {
      snapMark=null;if(interaction.snap===!!ctrl)return position;
      let p;
      if(interaction.element==='increment'||interaction.element==='grid') {
        const origin=interaction.element==='grid'?[0,0,0]:anchor,delta=sub(position,origin),step=Math.max(1e-8,interaction.moveStep);
        p=IM.add(origin,IM.world(frame,frame.map(d=>Math.round(dot(delta,d)/step)*step)));
      } else {p=snapHit(sx,sy,anchor);if(!p)return position;snapMark=p;}
      if(axis!==null&&axis!==undefined)p=IM.add(anchor,IM.mul(frame[axis],dot(sub(p,anchor),frame[axis])));
      return p;
    }
    const svg = (tag, attrs) => { const el = document.createElementNS(SVGNS, tag); for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v); return el; };
    function handleGeometry(cam, h) {                                // {s0, tip, u, ppu} in canvas pixels, or null when unusable
      const o = h.origin, s0 = toScreen(cam, o[0], o[1], o[2]);
      const s1 = toScreen(cam, o[0] + h.dir[0], o[1] + h.dir[1], o[2] + h.dir[2]);
      if (!s0 || !s1) return null;
      const dx = s1[0] - s0[0], dy = s1[1] - s0[1], ppu = Math.hypot(dx, dy);
      if (ppu < 1e-3) return null;                                  // pointing straight into the screen
      const u = [dx / ppu, dy / ppu];
      return { s0, tip: [s0[0] + u[0] * 64, s0[1] + u[1] * 64], u, ppu };
    }
    function planeCorners(cam,h){
      if(h.plane_axis===undefined)return null;
      const frame=h.axis_rotation?IM.frame(h.axis_rotation):IM.identity(), axes=[0,1,2].filter(i=>i!==h.plane_axis);
      const o=toScreen(cam,...h.origin);if(!o)return null;
      const a=planePoint(o[0],o[1],h.origin),b=planePoint(o[0]+25,o[1],h.origin);if(!a||!b)return null;
      const scale=IM.length(sub(b,a));
      return [[.5,.5],[1,.5],[1,1],[.5,1]].map(([a,b])=>toScreen(cam,...IM.add(h.origin,IM.add(IM.mul(frame[axes[0]],a*scale),IM.mul(frame[axes[1]],b*scale)))));
    }
    // a ring: basis vectors in its plane, and its outline projected to the screen (null where it passes behind the camera)
    function ringBasis(h) {
      const n = h.dir, a = Math.abs(n[2]) < 0.9 ? [0, 0, 1] : [1, 0, 0], u = norm(cross(a, n));
      return { u, v: cross(n, u) };
    }
    function ringPoint(h, t) {
      const { u, v } = ringBasis(h), c = Math.cos(t) * h.radius, d = Math.sin(t) * h.radius, o = h.origin;
      return [o[0] + u[0] * c + v[0] * d, o[1] + u[1] * c + v[1] * d, o[2] + u[2] * c + v[2] * d];
    }
    const RING_STEPS = 72;
    const shown = () => (selHandles.length ? selHandles : handles).filter((h) => h.kind !== "guide" && (!h.tool || h.tool === api.tool)).map(h=>{
      if(h.pending&&interaction.orientation==='view'){const frame=handleDrag?.h.axis_rotation?IM.frame(handleDrag.h.axis_rotation):orientationFrame();return {...h,dir:frame[h.comp??2],axis_rotation:IM.euler(frame)};}return h;
    });
    function renderHandles(cam) {
      hsvg.setAttribute("width", cam.w); hsvg.setAttribute("height", cam.h);
      const kids = [];
      for (const gd of handles) {                                                            // guides: outlines of the regions a node works on (not grabbable)
        if (gd.kind !== "guide") continue;
        const col = gd.color || "#e8b565";
        for (const line of gd.lines || []) {
          let d = "", pen = false;
          for (const q of line) {
            const sp = toScreen(cam, q[0], q[1], q[2]);
            d += sp ? (pen ? "L" : "M") + sp[0].toFixed(1) + " " + sp[1].toFixed(1) : "";
            pen = !!sp;
          }
          if (d) kids.push(svg("path", { d, fill: "none", stroke: col, "stroke-width": 1.5, "stroke-dasharray": "6 4", "stroke-opacity": 0.85, "pointer-events": "none", class: "pv-guide", "data-id": gd.id }));
        }
      }
      if (opt.bounds && data.npoints) {                                                      // bounding box of the displayed geometry, with its size
        const [lo, hi] = data.bounds, c = (i, j, k) => [i ? hi[0] : lo[0], j ? hi[1] : lo[1], k ? hi[2] : lo[2]];
        const edges = [];
        for (const [a, b] of [[[0,0,0],[1,0,0]],[[0,1,0],[1,1,0]],[[0,0,1],[1,0,1]],[[0,1,1],[1,1,1]],[[0,0,0],[0,1,0]],[[1,0,0],[1,1,0]],[[0,0,1],[0,1,1]],[[1,0,1],[1,1,1]],
                              [[0,0,0],[0,0,1]],[[1,0,0],[1,0,1]],[[0,1,0],[0,1,1]],[[1,1,0],[1,1,1]]]) {
          const p = toScreen(cam, ...c(...a)), q = toScreen(cam, ...c(...b));
          if (p && q) kids.push(svg("line", { x1: p[0], y1: p[1], x2: q[0], y2: q[1], stroke: "#8fb3d9", "stroke-width": 1, "stroke-dasharray": "3 3", opacity: 0.7, "pointer-events": "none", class: "pv-bounds" }));
        }
        const fmt = (v) => window.MiniUnits?MiniUnits.format(v):(Math.abs(v) >= 100 ? v.toFixed(0) : Math.abs(v) >= 10 ? v.toFixed(1) : v.toFixed(3)).replace(/(\.\d*?)0+$/, "$1").replace(/\.$/, "");
        for (const [axis, a, b] of [[0, c(0,0,0), c(1,0,0)], [1, c(0,0,0), c(0,1,0)], [2, c(0,0,0), c(0,0,1)]]) {
          const m = toScreen(cam, (a[0] + b[0]) / 2, (a[1] + b[1]) / 2, (a[2] + b[2]) / 2);
          if (!m) continue;
          const t = svg("text", { x: m[0] + 4, y: m[1] - 4, fill: AXIS_COLOUR[axis], "font-size": 11, "pointer-events": "none", class: "pv-bounds-label" });
          t.textContent = "XYZ"[axis] + " " + fmt(hi[axis] - lo[axis]);
          kids.push(t);
        }
      }
      if(interaction.proportional_enabled){
        const origin=handleDrag?.h.origin||shown()[0]?.origin;
        if(origin){const a=toScreen(cam,...origin),right=[cam.MV[0],cam.MV[4],cam.MV[8]],b=toScreen(cam,...IM.add(origin,IM.mul(right,interaction.proportional_radius)));
          if(a&&b){const radius=Math.hypot(b[0]-a[0],b[1]-a[1]);if(Number.isFinite(radius)&&radius>0)kids.push(svg('circle',{cx:a[0],cy:a[1],r:Math.min(radius,1e6),fill:'none',stroke:'#eee','stroke-width':1,'stroke-dasharray':'4 3','pointer-events':'none',class:'pv-proportional-radius'}));}
        }
      }
      for(const h of shown())for(const line of h.guide_lines||[]){const a=toScreen(cam,...line[0]),b=toScreen(cam,...line[1]);if(a&&b)kids.push(svg('line',{x1:a[0],y1:a[1],x2:b[0],y2:b[1],stroke:'#e8b565','stroke-width':1,'pointer-events':'none',opacity:.65}));}
      for (const h of shown()) {
        const active=handleDrag?.h.id===h.id;
        const col = active ? '#fff18c' : AXIS_COLOUR[h.axis_color??h.comp] || "#ffb04d";
        const grp = svg("g", { class: "pv-handle"+(active?' active':''), "data-id": h.id, opacity: handleDrag&&!active?0.3:1 });
        const tip = svg("title", {});
        tip.textContent = h.label + " (drag)";
        if (h.kind === "cage") {
          const p=toScreen(cam,...h.origin);if(!p)continue;
          grp.append(svg('rect',{x:p[0]-5,y:p[1]-5,width:10,height:10,fill:col,stroke:'#17202c',class:'pv-grab'}),tip);
        } else if (h.kind === "ring") {
          let d = "", pen = false;
          for (let k = 0; k <= RING_STEPS; k++) {
            const p = ringPoint(h, (k / RING_STEPS) * 2 * Math.PI), sp = toScreen(cam, p[0], p[1], p[2]);
            d += sp ? (pen ? "L" : "M") + sp[0].toFixed(1) + " " + sp[1].toFixed(1) : "";
            pen = !!sp;
          }
          grp.append(svg("path", { d, fill: "none", stroke: col, "stroke-width": 3, "stroke-opacity": 0.9, "pointer-events": "none" }),
            svg("path", { d, class: "pv-grab", fill: "none", stroke: "transparent", "stroke-width": 16, style: "cursor: grab" }), tip);
        } else if (h.kind === "uniform" || h.kind === "plane") {
          const g = handleGeometry(cam, h);
          const s0 = g ? g.s0 : toScreen(cam, h.origin[0], h.origin[1], h.origin[2]);
          if (!s0) continue;
          const corners=planeCorners(cam,h);
          if(corners && corners.every(Boolean)){grp.append(svg('polygon',{points:corners.map(p=>p.slice(0,2).join(',')).join(' '),fill:['#e65b5b','#6dcc73','#639bfa'][h.plane_axis],'fill-opacity':.6,stroke:'#eee8',class:'pv-grab',style:'cursor:move'}),tip);}
          else grp.append(svg("rect", { x: s0[0] - 8, y: s0[1] - 8, width: 16, height: 16, fill: "#ffb04d", stroke: "#0008", class: "pv-grab", style: "cursor: nesw-resize" }), tip);
        } else {
          const g = handleGeometry(cam, h);
          if (!g) continue;
          const [x0, y0] = g.s0, [x1, y1] = g.tip, nx = -g.u[1], ny = g.u[0];
          // only the end shape is grabbable, so a click on the surface under the shaft or the root still selects/orbits
          grp.append(svg("line", { x1: x0, y1: y0, x2: x1, y2: y1, stroke: col, "stroke-width": 3, "stroke-linecap": "round", "pointer-events": "none" }),
            svg("circle", { cx: x0, cy: y0, r: 4, fill: col, stroke: "#0008", "pointer-events": "none" }));
          if (h.kind === "scale") {
            grp.append(svg("rect", { x: x1 - 6, y: y1 - 6, width: 12, height: 12, fill: col, stroke: "#0008", class: "pv-grab", style: "cursor: ew-resize" }),
              svg("circle", { class: "pv-grab", cx: x1, cy: y1, r: 14, fill: "transparent", style: "cursor: ew-resize" }));
          } else {
            grp.append(svg("polygon", { points: `${x1 + g.u[0] * 12},${y1 + g.u[1] * 12} ${x1 + nx * 6},${y1 + ny * 6} ${x1 - nx * 6},${y1 - ny * 6}`, fill: col,
              style: "cursor: ew-resize", class: "pv-grab" }),
              svg("circle", { class: "pv-grab", cx: x1 + g.u[0] * 4, cy: y1 + g.u[1] * 4, r: 14, fill: "transparent", style: "cursor: ew-resize" }));
          }
          grp.append(tip);
        }
        const g=handleGeometry(cam,h),angle=g?Math.atan2(g.u[1],g.u[0])*180/Math.PI:0;
        const cursor=h.kind==='plane'?'move':h.kind==='ring'?'grab':h.kind==='uniform'?'nwse-resize':['ew-resize','nwse-resize','ns-resize','nesw-resize'][((Math.round(angle/45)%4)+4)%4];
        for (const el of grp.querySelectorAll(".pv-grab")) {el.style.cursor=cursor;el.addEventListener("pointerdown", (e) => startHandleDrag(e, h));}
        kids.push(grp);
      }
      if(handleDrag){
        const h=handleDrag.h,dir=handleDrag.guideDir||(h.kind!=='plane'&&h.kind!=='uniform'?h.dir:null);
        const p=toScreen(cam,...h.origin),g=dir&&handleGeometry(cam,{...h,dir});
        if(p&&g)kids.push(svg('line',{x1:p[0]-g.u[0]*2000,y1:p[1]-g.u[1]*2000,x2:p[0]+g.u[0]*2000,y2:p[1]+g.u[1]*2000,stroke:AXIS_COLOUR[h.axis_color??h.comp]||'#ffb04d','stroke-dasharray':'5 5',opacity:0.45,'pointer-events':'none'}));
      }
      if(snapMark){const p=toScreen(cam,...snapMark);if(p)kids.push(svg('circle',{cx:p[0],cy:p[1],r:8,fill:'none',stroke:'#ffef83','stroke-width':2,'pointer-events':'none'}));}
      hsvg.replaceChildren(...kids);
    }
    function startHandleDrag(e, h) {
      if(e.button!==0||handleDrag||endingDrag)return;
      // Add/subtract selection must reach geometry even when a handle overlaps it.
      if (!e.modal && sel.mode !== "view" && (e.shiftKey || e.ctrlKey || e.metaKey)) {
        e.stopPropagation(); beginCanvasDrag(e); return;
      }
      if(h.kind==='plane'){startPlaneDrag(e,h);return;}
      e.preventDefault(); e.stopPropagation();
      const cam = camera();
      if (!api.onHandleStart) return;
      const r = canvas.getBoundingClientRect();
      const x0 = e.clientX - r.left, y0 = e.clientY - r.top;
      const drag = handleDrag = { h, base: NaN, value: NaN, measure: null, last: null, released: false };
      snapData=raw;const selection=api.selectionInfo();snapExclude=new Set(selection?.points||[]);
      const candidates=shown().filter(x=>x.param===h.param);
      document.body.classList.add('pv-dragging');feedback.hidden=false;
      feedback.textContent=h.label+' · Shift: precise · Ctrl: snap · Esc: cancel';schedule();
      let finished = false;
      let precise=false,precisionRaw=null,precisionValue=null;
      const move = (ev) => {
        const x = ev.clientX - r.left, y = ev.clientY - r.top;
        if (!drag.measure) { drag.last = [x, y]; return; }                      // the node is still being created: remember where the pointer is
        let v = drag.measure(x, y);
        if(precisionRaw===null){precisionRaw=drag.base;precisionValue=drag.base;}
        if(!!ev.shiftKey!==precise){precise=!!ev.shiftKey;precisionRaw=v;precisionValue=drag.value;}
        v=precisionValue+(v-precisionRaw)*(precise?0.1:1);
        const snapping=interaction.snap!==!!ev.ctrlKey;
        if(snapping&&h.kind==='ring')v=drag.base+Math.round((v-drag.base)/interaction.angleStep)*interaction.angleStep;
        else if(snapping&&(h.kind==='scale'||h.kind==='uniform'))v=drag.base+Math.round((v-drag.base)/interaction.scaleStep)*interaction.scaleStep;
        else if(h.kind==='arrow') {
          const unsnappedValue=v;
          let anchor=h.origin;
          if(selection&&interaction.target==='median')anchor=selection.median;
          if(selection&&interaction.target==='active')anchor=selection.active;
          if(selection&&interaction.target==='closest'&&snapping){const hit=snapHit(x,y);if(hit){let d=Infinity;for(const i of selection.points){const p=Array.from(snapData.P.slice(i*3,i*3+3)),n=IM.length(sub(p,hit));if(n<d){d=n;anchor=p;}}}}
          const position=IM.add(anchor,IM.mul(h.dir,(v-drag.base)/h.scale));
          const p=snapPosition(position,x,y,ev.ctrlKey,anchor,null,h.axis_rotation?IM.frame(h.axis_rotation):IM.identity());
          v=drag.base+dot(sub(p,anchor),h.dir)*h.scale;
          if(snapping&&interaction.element==='increment'){const step=interaction.moveStep*Math.abs(h.scale);v=drag.base+Math.round((unsnappedValue-drag.base)/step)*step;}
        }
        if (h.min !== undefined) v = Math.max(h.min, v);
        if (h.max !== undefined) v = Math.min(h.max, v);
        const resolution=h.kind==='arrow'&&['amount','distance','thickness'].includes(h.param)&&!snapping?1000:1e5;
        v = Math.round(v * resolution) / resolution;
        if (v !== drag.value) { drag.value = v; if (api.onHandle) api.onHandle(h, v, false); }
        feedback.textContent=`${h.label} · ${h.length&&window.MiniUnits?MiniUnits.format(v):v.toFixed(3)}${snapping?' · SNAP '+interaction.element:''}${ev.shiftKey?' · PRECISE':''} · Esc cancels`;schedule();
      };
      const finish = () => {
        if (finished) return;
        finished = true;
        window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); window.removeEventListener("pointercancel", cancel);
        window.removeEventListener('keydown',key,true);window.removeEventListener('contextmenu',cancel,true);
        window.removeEventListener('pointerdown',right,true);
        if(drag.cancelled&&api.onHandleCancel){endingDrag=true;Promise.resolve(api.onHandleCancel(drag.h,drag.base)).finally(()=>{endingDrag=false;});}
        else if (api.onHandle && Number.isFinite(drag.value)) api.onHandle(drag.h, drag.value, true);
        handleDrag = null;
        snapData=null;snapExclude.clear();snapMark=null;feedback.hidden=true;document.body.classList.remove('pv-dragging');
        if (pendingHandles) { handles = pendingHandles; pendingHandles = null; }
        if (pendingSel) { selHandles = pendingSel; pendingSel = null; }
        schedule();
      };
      const up = () => { drag.released = true; if (drag.measure) finish(); };
      const cancel = ev => {ev.preventDefault();ev.stopImmediatePropagation();drag.cancelled=true;drag.released=true;if(drag.measure)finish();};
      const right=ev=>{if(ev.button===2)cancel(ev);else if(e.modal&&ev.button===0){ev.preventDefault();ev.stopImmediatePropagation();up();}};
      const key=ev=>{if(e.modal&&ev.key==='Enter'){ev.preventDefault();ev.stopImmediatePropagation();up();return;}if(ev.key==='Escape'){cancel(ev);return;}const axis='xyz'.indexOf(ev.key.toLowerCase()),next=candidates.find(x=>x.comp===axis);if(axis>=0&&next&&drag.measure&&api.onHandleAxis){ev.preventDefault();ev.stopImmediatePropagation();api.onHandle(h,drag.base,false);h=next;drag.h=h;begin(api.onHandleAxis(h));schedule();}};
      window.addEventListener("pointermove", move); if(!e.modal)window.addEventListener("pointerup", up); window.addEventListener("pointercancel", cancel);
      window.addEventListener('keydown',key,true);window.addEventListener('contextmenu',cancel,true);
      window.addEventListener('pointerdown',right,true);
      const begin = (base) => {                                                 // base: the parameter's value when the drag starts
        if (typeof base !== "number" || !Number.isFinite(base)) { drag.released = true; finish(); handleDrag = null; return; }
        drag.base = base; drag.value = base;
        precisionRaw=null;precisionValue=null;precise=false;
        if (h.kind === "ring") {                                                // the angle of the pointer around the ring, from where the ray meets its plane
          const { u, v } = ringBasis(h), o = h.origin, n = h.dir;
          const angleAt = (x, y) => {
            const ray = rayAt(cam, x, y), den = dot(ray, n);
            if (Math.abs(den) < 0.05) return null;                              // looking along the ring's plane: no stable angle
            const t = dot(sub(o, cam.eye), n) / den, rel = sub([cam.eye[0] + ray[0] * t, cam.eye[1] + ray[1] * t, cam.eye[2] + ray[2] * t], o);
            return Math.atan2(dot(rel, v), dot(rel, u));
          };
          let prev = angleAt(x0, y0), total = 0;
          drag.measure = (x, y) => {
            const a = angleAt(x, y);
            if (a === null || prev === null) { if (a !== null) prev = a; return base + total * h.scale; }
            let d = a - prev;
            if (d > Math.PI) d -= 2 * Math.PI; else if (d < -Math.PI) d += 2 * Math.PI;
            total += d; prev = a;
            return base + total * h.scale;
          };
        } else if (h.kind === "uniform") {
          drag.measure = (x, y) => base * Math.exp(((x - x0) - (y - y0)) / 120);
        } else {
          const g = handleGeometry(cam, h);
          if (!g) { drag.released = true; finish(); handleDrag = null; return; }
          const along = (x, y) => (x - x0) * g.u[0] + (y - y0) * g.u[1];
          drag.measure = h.kind === "scale" ? (x, y) => base * Math.exp(along(x, y) / 64) : (x, y) => base + (along(x, y) / g.ppu) * h.scale;
        }
        if (drag.last&&!drag.cancelled) { const [lx, ly] = drag.last; drag.last = null; move({ clientX: lx + r.left, clientY: ly + r.top }); }
        if (drag.released) finish();
      };
      const first = api.onHandleStart(h);                                       // a number, or a promise of one (a handle that must create its node first)
      if (first && typeof first.then === "function") first.then(begin, () => begin(NaN)); else begin(first);
    }

    function startPlaneDrag(e,h) {
      if(e.button!==0||handleDrag||endingDrag)return;e.preventDefault();e.stopPropagation();
      const r=canvas.getBoundingClientRect(),frame=h.axis_rotation?IM.frame(h.axis_rotation):IM.identity(),origin=h.origin.slice();
      const start=planePoint(e.clientX-r.left,e.clientY-r.top,origin,h.plane_axis===undefined?null:frame[h.plane_axis]),info=api.selectionInfo();if(!start)return;
      snapData=raw;snapExclude=new Set(info?.points||[]);
      const drag=handleDrag={h,value:null};let base=null,last=e,axis=h.plane_axis??null,plane=h.plane_axis!==undefined,released=false,cancelled=false;
      let snapBase=null,pickingBase=false,baseHover=null;
      const cam=camera(),pivotScreen=toScreen(cam,...origin),viewNormal=[cam.MV[2],cam.MV[6],cam.MV[10]];
      let rotateLast=pivotScreen?Math.atan2(-(e.clientY-r.top-pivotScreen[1]),e.clientX-r.left-pivotScreen[0]):0,rotateTotal=0;
      let precise=false,rawAnchor=[0,0,0],valueAnchor=[0,0,0],previousDelta=[0,0,0];
      document.body.classList.add('pv-dragging');feedback.hidden=false;
      function move(ev){last=ev;if(!base)return;const x=ev.clientX-r.left,y=ev.clientY-r.top;if(h.rotateVector){
          const angle=pivotScreen?Math.atan2(-(y-pivotScreen[1]),x-pivotScreen[0]):rotateLast;
          let step=angle-rotateLast;if(step>Math.PI)step-=2*Math.PI;if(step<-Math.PI)step+=2*Math.PI;rotateLast=angle;rotateTotal+=step*(ev.shiftKey?.1:1);
          let degrees=rotateTotal*180/Math.PI;if(interaction.snap!==!!ev.ctrlKey)degrees=Math.round(degrees/interaction.angleStep)*interaction.angleStep;
          const normal=axis===null?viewNormal:frame[axis],n=frame.map(v=>dot(v,normal)),a=degrees*Math.PI/180;
          const rotated=IM.frame(base).map(v=>IM.add(IM.add(IM.mul(v,Math.cos(a)),IM.mul(IM.cross(n,v),Math.sin(a))),IM.mul(n,dot(n,v)*(1-Math.cos(a)))));
          drag.value=IM.euler(rotated);api.onHandle(h,drag.value,false);feedback.textContent='Rotate '+degrees.toFixed(2)+'° · '+(axis===null?'view':'XYZ'[axis])+' · X/Y/Z constrain · Esc cancels';schedule();return;
        }if(h.scaleVector){let amount=((ev.clientX-e.clientX)-(ev.clientY-e.clientY))/120;
          if(!!ev.shiftKey!==precise){precise=!!ev.shiftKey;rawAnchor=[amount,0,0];valueAnchor=previousDelta;}
          amount=valueAnchor[0]+(amount-rawAnchor[0])*(precise?.1:1);previousDelta=[amount,0,0];let factor=Math.exp(amount);
          if(interaction.snap!==!!ev.ctrlKey)factor=1+Math.round((factor-1)/interaction.scaleStep)*interaction.scaleStep;
          drag.value=base.map((v,i)=>v*((axis===null||(plane?i!==axis:i===axis))?factor:1));api.onHandle(h,drag.value,false);feedback.textContent='Scale '+drag.value.map(v=>v.toFixed(3)).join(', ')+' · X/Y/Z constrain · Shift+axis plane · Esc cancels';schedule();return;}
        if(pickingBase){const old=interaction.includeSelected;interaction.includeSelected=true;baseHover=snapHit(x,y,origin)||planePoint(x,y,origin);interaction.includeSelected=old;snapMark=baseHover;feedback.textContent='Pick snap base · click confirms · Esc returns';schedule();return;}const p=planePoint(x,y,origin,h.plane_axis===undefined?null:frame[h.plane_axis]);if(!p)return;
        drag.guideDir=axis!==null&&!plane?frame[axis]:null;
        let d=sub(p,start);if(!!ev.shiftKey!==precise){precise=!!ev.shiftKey;rawAnchor=d;valueAnchor=previousDelta;}
        d=IM.add(valueAnchor,IM.mul(sub(d,rawAnchor),precise?0.1:1));previousDelta=d;
        if(axis!==null)d=plane?sub(d,IM.mul(frame[axis],dot(d,frame[axis]))):IM.mul(frame[axis],dot(d,frame[axis]));
        let anchor=snapBase||(interaction.target==='median'&&info?info.median:interaction.target==='active'&&info?info.active:origin);
        if(!snapBase&&info&&interaction.target==='closest'){const hit=snapHit(x,y,origin);if(hit){let nearest=Infinity;for(const i of info.points){const v=Array.from(snapData.P.slice(i*3,i*3+3)),n=IM.length(sub(v,hit));if(n<nearest){nearest=n;anchor=v;}}}}
        const snapped=snapPosition(IM.add(anchor,d),x,y,ev.ctrlKey,anchor,axis!==null&&!plane?axis:null,frame);
        d=sub(snapped,anchor);if(axis!==null&&plane)d=sub(d,IM.mul(frame[axis],dot(d,frame[axis])));
        drag.value=base.map((v,i)=>Math.round((v+dot(d,frame[i]))*1e5)/1e5);
        api.onHandle(h,drag.value,false);feedback.textContent=`Move ${axis===null?'view plane':(plane?'plane excluding ':'axis ')+'XYZ'[axis]} · ${drag.value.map(v=>window.MiniUnits?MiniUnits.format(v):v.toFixed(3)).join(', ')} · Esc cancels`;schedule();
      }
      function finish(){if(!base)return;window.removeEventListener('pointermove',move);window.removeEventListener('pointerup',up);window.removeEventListener('pointercancel',cancel);window.removeEventListener('contextmenu',cancel,true);window.removeEventListener('keydown',key,true);
        window.removeEventListener('pointerdown',right,true);
        if(cancelled&&base.length===3){endingDrag=true;Promise.resolve(api.onHandleCancel?.(h,base)).finally(()=>endingDrag=false);}else if(!cancelled)api.onHandle(h,drag.value||base,true);
        handleDrag=null;snapData=null;snapExclude.clear();snapMark=null;feedback.hidden=true;document.body.classList.remove('pv-dragging');if(pendingHandles){handles=pendingHandles;pendingHandles=null;}if(pendingSel){selHandles=pendingSel;pendingSel=null;}schedule();
      }
      function up(){released=true;finish();}function cancel(ev){ev.preventDefault();ev.stopImmediatePropagation();cancelled=true;released=true;finish();}
      function right(ev){if(pickingBase&&ev.button===0){ev.preventDefault();ev.stopImmediatePropagation();snapBase=baseHover;pickingBase=false;snapMark=null;return;}if(ev.button===2)cancel(ev);else if(e.modal&&ev.button===0){ev.preventDefault();ev.stopImmediatePropagation();up();}}
      function key(ev){if(!h.scaleVector&&!h.rotateVector&&ev.key.toLowerCase()==='b'){ev.preventDefault();ev.stopImmediatePropagation();pickingBase=!pickingBase;move(last);return;}if(pickingBase&&ev.key==='Escape'){ev.preventDefault();ev.stopImmediatePropagation();pickingBase=false;snapMark=null;move(last);return;}if(e.modal&&ev.key==='Enter'){ev.preventDefault();ev.stopImmediatePropagation();up();return;}if(ev.key==='Escape'){cancel(ev);return;}const k='xyz'.indexOf(ev.key.toLowerCase());if(k>=0){ev.preventDefault();ev.stopImmediatePropagation();axis=axis===k&&plane===ev.shiftKey?null:k;plane=ev.shiftKey;move(last);}}
      window.addEventListener('pointermove',move);if(!e.modal)window.addEventListener('pointerup',up);window.addEventListener('pointercancel',cancel);window.addEventListener('contextmenu',cancel,true);window.addEventListener('keydown',key,true);
      window.addEventListener('pointerdown',right,true);
      Promise.resolve(api.onHandleStart(h)).then(v=>{base=Array.isArray(v)&&v.length===3&&v.every(Number.isFinite)?v:[];if(base.length!==3)cancelled=released=true;if(!cancelled)move(last);if(released)finish();},()=>{base=[];cancelled=true;finish();});schedule();
    }


    window.addEventListener('wheel',e=>{if(!handleDrag||!interaction.proportional_enabled)return;e.preventDefault();e.stopImmediatePropagation();interaction.proportional_radius=Math.max(.0001,Math.min(100000,interaction.proportional_radius*Math.exp(e.deltaY*.001)));api.onProportional?.();schedule();},{capture:true,passive:false});
    // ---------------------------------------------------------------- navigation: axis views, orthographic, frame selection, axis gizmo
    const VIEWS = { front: [-Math.PI / 2, 0], back: [Math.PI / 2, 0], right: [0, 0], left: [Math.PI, 0], top: [-Math.PI / 2, PITCH_MAX], bottom: [-Math.PI / 2, -PITCH_MAX] };
    let tween = 0;
    function goTo(yaw, pitch, instant) {
      cancelAnimationFrame(tween); tween = 0;
      if (instant) { view.yaw = yaw; view.pitch = pitch; schedule(); return; }
      const y0 = view.yaw, p0 = view.pitch;
      let dy = (yaw - y0) % (2 * Math.PI);
      if (dy > Math.PI) dy -= 2 * Math.PI; else if (dy < -Math.PI) dy += 2 * Math.PI;
      const t0 = performance.now();
      const step = (now) => {
        const k = Math.min(1, (now - t0) / 180), e = k * k * (3 - 2 * k);
        view.yaw = y0 + dy * e; view.pitch = p0 + (pitch - p0) * e;
        schedule();
        tween = k < 1 ? requestAnimationFrame(step) : 0;
      };
      tween = requestAnimationFrame(step);
    }
    function viewAxis(name, opposite, instant) {
      const key = opposite ? { front: "back", back: "front", right: "left", left: "right", top: "bottom", bottom: "top" }[name] : name;
      const v = VIEWS[key];
      if (v) goTo(v[0], v[1], instant);
    }
    function frameSelection() {                                      // the selected elements, or everything when nothing is selected
      if (!raw) return;
      const ids = new Set();
      if (sel.point.size) for (const i of sel.point) ids.add(i);
      for (const [, e] of sel.edge) { ids.add(e[0]); ids.add(e[1]); }
      if (sel.face.size) for (let i = 0; i < raw.triPrim.length; i++) if (sel.face.has(raw.triPrim[i])) for (let c = 0; c < 3; c++) ids.add(raw.tri[3 * i + c]);
      if (!ids.size) { frameAll(); return; }
      const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
      for (const i of ids) for (let c = 0; c < 3; c++) { const v = raw.P[3 * i + c]; if (v < lo[c]) lo[c] = v; if (v > hi[c]) hi[c] = v; }
      view.target = [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2];
      const r = Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2, 1e-3);
      view.dist = fitDist(r, 1.3);
      schedule();
    }
    const gsvg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    gsvg.setAttribute("class", "pv-gizmo"); gsvg.setAttribute("width", 92); gsvg.setAttribute("height", 92);
    canvas.parentElement.append(gsvg);
    const GAXES = [["right", [1, 0, 0], "X", "#ff5d5d"], ["back", [0, 1, 0], "Y", "#4cc38a"], ["top", [0, 0, 1], "Z", "#4aa3ff"]];
    const gizmo = [];                                                // built once, so a click is never lost to a redraw: negatives first, positives on top
    for (const sign of [-1, 1]) for (const [name, a, letter, col] of GAXES) {
      const pos = sign > 0, g = svg("g", { class: "pv-axis", "data-axis": (pos ? "+" : "-") + letter, style: "cursor: pointer" });
      const line = svg("line", { stroke: col, "stroke-width": 2, x1: 46, y1: 46 });
      const dot = svg("circle", { r: pos ? 9 : 6.5, fill: pos ? col : "#2d3035", stroke: col, "stroke-width": 1.5 });
      const label = svg("text", { "text-anchor": "middle", "font-size": 10, "font-weight": 700, fill: "#0b0d10", "pointer-events": "none" });
      label.textContent = pos ? letter : "";
      if (pos) g.append(line);
      g.append(dot, label);
      g.addEventListener("pointerdown", (e) => e.stopPropagation());
      g.addEventListener("click", (e) => { e.stopPropagation(); viewAxis(name, !pos, false); });
      gizmo.push({ a: a.map((v) => v * sign), line, dot, label, g, pos });
      gsvg.append(g);
    }
    function renderGizmo(cam) {
      const m = cam.MV, R = 30, c = 46;
      for (const it of gizmo) {
        const [x, y, z] = it.a, sx = c + R * (m[0] * x + m[4] * y + m[8] * z), sy = c - R * (m[1] * x + m[5] * y + m[9] * z);
        const front = m[2] * x + m[6] * y + m[10] * z >= 0;           // towards the viewer
        it.dot.setAttribute("cx", sx); it.dot.setAttribute("cy", sy);
        it.label.setAttribute("x", sx); it.label.setAttribute("y", sy + 3.5);
        if (it.pos) { it.line.setAttribute("x2", sx); it.line.setAttribute("y2", sy); }
        it.g.setAttribute("opacity", front ? 1 : 0.45);
      }
    }

    // Fly navigation: hold RMB, or latch with Shift+` / Walk. Escape restores the camera.
    let fly=null, flyRAF=0, flySpeed=0;
    const flyKeys=new Set();
    function flyForward(){return [-Math.cos(view.pitch)*Math.cos(view.yaw),-Math.cos(view.pitch)*Math.sin(view.yaw),-Math.sin(view.pitch)];}
    function flyHint(){feedback.hidden=false;feedback.textContent=`Fly ${window.MiniUnits?MiniUnits.format(flySpeed):flySpeed.toFixed(2)+" units"}/s · WASD · Q/E down/up · Shift fast · Ctrl slow · wheel speed · Enter accept · Esc cancel`;}
    function endFly(cancel=false){
      if(!fly)return;const old=fly;fly=null;flyKeys.clear();cancelAnimationFrame(flyRAF);
      if(cancel)Object.assign(view,old.saved,{target:old.saved.target.slice()});
      if(document.pointerLockElement===canvas)document.exitPointerLock();
      canvas.style.cursor='';feedback.hidden=true;api.onNavigationChange?.(false);schedule();
    }
    function startFly(latched=false){
      if(fly || handleDrag || endingDrag)return;
      const saved={...view,target:view.target.slice()};view.ortho=false;
      fly={latched,saved,last:performance.now(),eye:camera().eye.slice(),x:null,y:null};
      if(!flySpeed)flySpeed=Math.max(.1,view.dist*.5);
      canvas.style.cursor='crosshair';api.onNavigationChange?.(true);flyHint();
      function tick(now){if(!fly)return;const dt=Math.min(.05,(now-fly.last)/1000);fly.last=now;
        const f=flyForward(),right=[-Math.sin(view.yaw),Math.cos(view.yaw),0];
        const forward=Number(flyKeys.has('KeyW'))-Number(flyKeys.has('KeyS')),side=Number(flyKeys.has('KeyD'))-Number(flyKeys.has('KeyA')),up=Number(flyKeys.has('KeyE'))-Number(flyKeys.has('KeyQ'));
        const delta=f.map((v,i)=>v*forward+right[i]*side+(i===2?up:0)),len=Math.hypot(...delta);
        const speed=flySpeed*(flyKeys.has('ShiftLeft')||flyKeys.has('ShiftRight')?4:1)*(flyKeys.has('ControlLeft')||flyKeys.has('ControlRight')?.2:1);
        if(len)fly.eye=fly.eye.map((v,i)=>v+delta[i]/len*speed*dt);
        view.target=fly.eye.map((v,i)=>v+f[i]*view.dist);schedule();flyRAF=requestAnimationFrame(tick);
      }
      flyRAF=requestAnimationFrame(tick);
      if(latched){try{const p=canvas.requestPointerLock?.();p?.catch(()=>{});}catch(_){}}
    }
    api.toggleWalk=()=>fly?endFly(false):startFly(true);
    api.navigationState=()=>({active:!!fly,latched:!!fly?.latched,speed:flySpeed,eye:camera().eye.slice(),target:view.target.slice(),yaw:view.yaw,pitch:view.pitch});
    document.addEventListener('keydown',e=>{
      const typing=e.target.closest?.('input,textarea,select,[contenteditable="true"]');
      if(!fly){if(!typing && e.shiftKey && e.code==='Backquote' && canvas.parentElement.matches(':hover')){e.preventDefault();e.stopImmediatePropagation();startFly(true);}return;}
      if(typing || document.querySelector('.app-dialog[open]')){endFly(false);return;}
      e.preventDefault();e.stopImmediatePropagation();
      if(e.code==='Escape'){endFly(true);return;}if(e.code==='Enter' || (e.shiftKey&&e.code==='Backquote')){endFly(false);return;}
      flyKeys.add(e.code);
    },true);
    document.addEventListener('keyup',e=>{if(!fly)return;flyKeys.delete(e.code);e.preventDefault();e.stopImmediatePropagation();},true);
    document.addEventListener('pointermove',e=>{if(!fly)return;
      const locked=document.pointerLockElement===canvas,dx=locked?e.movementX:fly.x===null?0:e.clientX-fly.x,dy=locked?e.movementY:fly.y===null?0:e.clientY-fly.y;
      fly.x=e.clientX;fly.y=e.clientY;view.yaw-=dx*.003;view.pitch=Math.max(-PITCH_MAX,Math.min(PITCH_MAX,view.pitch+dy*.003));
      const f=flyForward();view.target=fly.eye.map((v,i)=>v+f[i]*view.dist);schedule();
    },true);
    window.addEventListener('pointerup',e=>{if(fly&&!fly.latched&&e.button===2)endFly(false);},true);
    window.addEventListener('blur',()=>endFly(false));
    document.addEventListener('visibilitychange',()=>{if(document.hidden)endFly(false);});
    document.addEventListener('pointerlockchange',()=>{if(!fly&&document.pointerLockElement===canvas){document.exitPointerLock();return;}if(fly?.latched&&document.pointerLockElement!==canvas)endFly(true);});
    canvas.addEventListener('pointercancel',()=>endFly(false));
    // ---------------------------------------------------------------- mouse: orbit / pan / select
    const marquee = document.createElement("div");
    marquee.className = "pv-marquee hidden";
    canvas.parentElement.append(marquee);
    let dragging = null;
    function beginCanvasDrag(e) {
      if(fly){e.preventDefault();return;}
      if(e.button===2 && !e.altKey){e.preventDefault();canvas.setPointerCapture(e.pointerId);startFly(false);if(fly){fly.x=e.clientX;fly.y=e.clientY;}return;}
      canvas.setPointerCapture(e.pointerId);
      const r = canvas.getBoundingClientRect();
      const selecting = sel.mode !== "view" && e.button === 0 && !(opt.navigation==="houdini" && e.altKey) && raw;
      if(opt.navigation!=="houdini" && e.button===0 && !selecting && !e.altKey)return;
      const pan = opt.navigation!=="houdini" ? (e.button===1 && e.shiftKey || e.button===2) : sel.mode === "view" ? (e.button === 1 || e.button === 2 || e.shiftKey) : (e.button === 2 || (e.button === 1 && e.shiftKey));
      if(opt.orbitDepth && !pan && e.button===1 && !e.shiftKey && !e.ctrlKey && raw){
        const cam=camera(),direction=rayAt(cam,e.clientX-r.left,e.clientY-r.top),hit=raycast(cam.eye,direction);
        if(hit){
          // Use surface depth along the existing view axis, not the off-centre hit
          // as a new look-at target: retargeting changes orientation on mouse-down.
          const forward=norm(sub(view.target,cam.eye)),depth=dot(IM.mul(direction,hit.t),forward);
          const lensScale=view.ortho?Math.tan(FOV/2)/Math.tan(ORTHO_FOV/2):1;
          if(depth>1e-6){view.target=IM.add(cam.eye,IM.mul(forward,depth));view.dist=depth/lensScale;}
        }
      }
      dragging = { x: e.clientX, y: e.clientY, zoom: opt.navigation!=="houdini" && e.button===1 && e.ctrlKey, pan, select: !!selecting, x0: e.clientX - r.left, y0: e.clientY - r.top, moved: false,
        loop: e.altKey, ring: e.ctrlKey, path: e.ctrlKey && !e.altKey && !e.shiftKey, how: e.ctrlKey && e.shiftKey ? "subtract" : e.shiftKey ? "add" : (e.ctrlKey || e.metaKey) ? "subtract" : "replace" };
      e.preventDefault();
    }
    canvas.addEventListener("pointerdown", beginCanvasDrag);
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
      if (dragging.zoom) {
        view.dist=Math.max(0.001,view.dist*Math.exp(dy*0.01));
      } else if (dragging.pan) {
        const k = (view.dist * Math.tan(Math.PI / 8) * 2) / Math.max(canvas.clientHeight, 1);
        const right = [-Math.sin(view.yaw), Math.cos(view.yaw), 0];
        const up = [-Math.sin(view.pitch) * Math.cos(view.yaw), -Math.sin(view.pitch) * Math.sin(view.yaw), Math.cos(view.pitch)];
        for (let i = 0; i < 3; i++) view.target[i] += (-dx * right[i] + dy * up[i]) * k;
      } else {
        view.yaw -= dx * 0.008;
        view.pitch = Math.max(-PITCH_MAX, Math.min(PITCH_MAX, view.pitch + dy * 0.008));
      }
      schedule();
    });
    const endDrag = (e) => {
      const d = dragging; dragging = null;
      marquee.classList.add("hidden");
      if (!d || !d.select || (e && e.type === "pointercancel")) return;
      const cam = camera();
      if (d.moved && d.x1 !== undefined) api.boxSelect(d.x0, d.y0, d.x1, d.y1, d.how);
      else if(d.loop){const edge=pickEdge(cam,d.x0,d.y0,12);if(edge){const es=quadStrip(edge,d.ring);const fs=new Set();if(sel.mode==='face')for(let i=0;i<raw.tri.length;i+=3)for(const [a,b]of es){const tri=[raw.tri[i],raw.tri[i+1],raw.tri[i+2]];if(tri.includes(a)&&tri.includes(b))fs.add(raw.triPrim[i/3]);}change(sel.mode==='point'?{points:[...new Set(es.flat())]}:sel.mode==='face'?{faces:[...fs]}:{edges:es},d.how==='add'?'add':'replace');}}
      else if(d.path && sel.mode!=="piece")pathPick(d.x0,d.y0,d.how);
      else api.pickAt(d.x0, d.y0, d.how);
    };
    canvas.addEventListener("pointerup", endDrag); canvas.addEventListener("pointercancel", endDrag);
    canvas.addEventListener("contextmenu", (e) => e.preventDefault());
    canvas.addEventListener("wheel", (e) => { e.preventDefault(); if(fly){flySpeed=Math.max(.001,Math.min(100000,flySpeed*Math.exp(-e.deltaY*.0015)));flyHint();return;} const cam=camera(),r=canvas.getBoundingClientRect(),direction=rayAt(cam,e.clientX-r.left,e.clientY-r.top),hit=raw?raycast(cam.eye,direction):null;
      const anchor=hit?IM.add(cam.eye,IM.mul(direction,hit.t)):planePoint(e.clientX-r.left,e.clientY-r.top,view.target);
      const distance=Math.max(.0001,view.dist*Math.exp(Math.max(-500,Math.min(500,e.deltaY))*.0012)),factor=distance/view.dist;
      if(anchor)for(let i=0;i<3;i++)view.target[i]=anchor[i]+(view.target[i]-anchor[i])*factor;
      view.dist=distance;
      schedule(); }, { passive: false });
    canvas.addEventListener("dblclick", (e) => { if (sel.mode === "view") frameAll(); });
    if (window.ResizeObserver) new ResizeObserver(schedule).observe(canvas);

    // The triangles and edges the viewer holds (see minihou/preview.py): the server leaves them out of a reply when they have not changed.
    Object.defineProperty(api, "topology", { get: () => (raw ? raw.topology : null), enumerable: true });
    Object.assign(api, {
      showFeedback(text){feedback.hidden=!text;feedback.textContent=text||'';if(!text)snapMark=null;schedule();},
      interaction, orientationFrame, snapPosition, planePoint,
      sketchGeometry:()=>[raw,templateRaw].filter(Boolean),
      beginTransform(tool,clientX,clientY){
        api.setTool(tool);const hs=shown();let h=tool==='move'?hs.find(h=>h.kind==='plane'&&h.plane_axis===undefined):tool==='scale'?hs.find(h=>h.kind==='uniform'):hs.find(h=>h.kind==='ring'&&h.comp===2);
        if(tool==='rotate'){const rotation=hs.find(x=>x.param==='rotate');if(rotation)h={...rotation,kind:'plane',comp:null,rotateVector:true,plane_axis:undefined};}
        if(tool==='scale'){const scale=hs.find(x=>x.param==='scale');if(scale)h={...scale,kind:'plane',comp:null,scaleVector:true,plane_axis:undefined};}
        if(!h&&tool==='move'){const arrow=hs.find(h=>h.param==='translate');if(arrow)h={...arrow,kind:'plane',comp:null};}
        if(!h)return false;startHandleDrag({button:0,modal:true,clientX,clientY,preventDefault(){},stopPropagation(){}},h);return true;
      },
      edgeAt:(x,y)=>raw?pickEdge(camera(),x,y,14):null,
      pointAt:(x,y)=>raw?pickPoint(camera(),x,y,14):-1,
      knifeSelectedFaces(){
        if(sel.mode==='face'||sel.mode==='piece')return [...sel.face];if(!raw)return [];
        const faces=new Map();for(let i=0;i<raw.tri.length;i++){const f=raw.triPrim[Math.floor(i/3)];if(!faces.has(f))faces.set(f,new Set());faces.get(f).add(raw.tri[i]);}
        return [...faces].filter(([f,ids])=>{if(sel.mode==='point')return [...ids].every(i=>sel.point.has(i));if(sel.mode!=='edge')return false;
          for(let i=0;i<raw.edge.length;i+=2)if(ids.has(raw.edge[i])&&ids.has(raw.edge[i+1])&&!sel.edge.has(edgeKey(raw.edge[i],raw.edge[i+1])))return false;return true;}).map(([f])=>f);
      },
      knifeProjection(){const c=camera();return {matrix:Array.from(mat4mul(c.P,c.MV)),width:c.w,height:c.h};},
      knifeLinePoint(a,b,t){
        const m=mat4mul(camera().P,camera().MV),w=p=>m[3]*p[0]+m[7]*p[1]+m[11]*p[2]+m[15],u=(1-t)/w(a),v=t/w(b);return a.map((x,i)=>(x*u+b[i]*v)/(u+v));
      },
      knifeCrossings(a,b,through){
        if(!raw)return [];const c=camera(),u=toScreen(c,...a),v=toScreen(c,...b);if(!u||!v)return [];const dx=v[0]-u[0],dy=v[1]-u[1],out=[];
        for(let i=0;i<raw.edge.length;i+=2){const p=Array.from(raw.P.slice(raw.edge[i]*3,raw.edge[i]*3+3)),q=Array.from(raw.P.slice(raw.edge[i+1]*3,raw.edge[i+1]*3+3)),s=toScreen(c,...p),t=toScreen(c,...q);if(!s||!t)continue;
          const ex=t[0]-s[0],ey=t[1]-s[1],den=dx*ey-dy*ex;if(Math.abs(den)<1e-8)continue;const ax=s[0]-u[0],ay=s[1]-u[1],f=(ax*ey-ay*ex)/den,g=(ax*dy-ay*dx)/den;
          if(f<0||f>1||g<0||g>1)continue;const world=api.knifeLinePoint(p,q,g),screen=[u[0]+dx*f,u[1]+dy*f];if(!through){const ray=rayAt(c,...screen),h=raycast(c.eye,ray);if(h&&IM.length(sub(world,IM.add(c.eye,IM.mul(ray,h.t))))>1e-4)continue;}out.push(screen);
        }return out;
      },
      knifeFrame(anchor,reference=0){
        if(!raw)return null;const c=camera(),s=toScreen(c,...anchor);if(!s)return null;const h=raycast(c.eye,rayAt(c,s[0],s[1]));if(!h)return null;
        const tri=Array.from(raw.tri.slice(h.tri*3,h.tri*3+3)),pts=tri.map(i=>Array.from(raw.P.slice(i*3,i*3+3))),normal=norm(cross(sub(pts[1],pts[0]),sub(pts[2],pts[0])));
        const face=raw.triPrim[h.tri],ids=new Set();for(let i=0;i<raw.tri.length;i+=3)if(raw.triPrim[i/3]===face)for(let j=0;j<3;j++)ids.add(raw.tri[i+j]);
        const refs=[];for(let i=0;i<raw.edge.length;i+=2)if(ids.has(raw.edge[i])&&ids.has(raw.edge[i+1])){const a=Array.from(raw.P.slice(raw.edge[i]*3,raw.edge[i]*3+3)),b=Array.from(raw.P.slice(raw.edge[i+1]*3,raw.edge[i+1]*3+3));refs.push({a,b,distance:IM.length(sub(anchor,IM.mul(IM.add(a,b),.5)))});}
        refs.sort((a,b)=>a.distance-b.distance);const edge=refs[reference%refs.length];return edge?{normal,u:norm(sub(edge.b,edge.a)),edge:[edge.a,edge.b]}:null;
      },
      knifePick(x,y,midpoint=false,ignore=false){
        const c=camera();let p=null,label='Face';
        if(!ignore&&raw){if(!midpoint){const id=pickPoint(c,x,y,12);if(id>=0){p=Array.from(raw.P.slice(id*3,id*3+3));label='Vertex';}}
          if(!p){p=snapSource(raw,x,y,[0,0,0],true,midpoint?'midpoint':'edge');if(p)label=midpoint?'Midpoint':'Edge';}}
        if(!p&&raw){const d=rayAt(c,x,y),h=raycast(c.eye,d);if(h)p=IM.add(c.eye,IM.mul(d,h.t));}
        if(!p){p=planePoint(x,y,view.target);label='Through space';}
        return p?{point:p,screen:toScreen(c,...p).slice(0,2),label}:null;
      },
      geometry:()=>raw, quadStrip,
      measurementGeometry(){
        if(!raw)return [];
        if(measurementCache)return measurementCache;
        const split=(source,objects)=>objects.map(o=>{const start=o.point_start,end=start+o.point_count,tri=[],edge=[];
          for(let i=0;i<source.tri.length;i+=3)if(source.triPrim[i/3]>=o.prim_start&&source.triPrim[i/3]<o.prim_start+o.prim_count)tri.push(source.tri[i]-start,source.tri[i+1]-start,source.tri[i+2]-start);
          for(let i=0;i<source.edge.length;i+=2)if(source.edge[i]>=start&&source.edge[i]<end&&source.edge[i+1]>=start&&source.edge[i+1]<end)edge.push(source.edge[i]-start,source.edge[i+1]-start);
          return {...o,P:source.P.subarray(start*3,end*3),tri:new Uint32Array(tri),edge:new Uint32Array(edge)};});
        return measurementCache=[...split(raw,api.previewObjects||[]),...(templateRaw?.triPrim?split(templateRaw,api.referenceObjects||[]):[])];
      },
      project: p=>toScreen(camera(),...p),
      isDragging:()=>!!handleDrag||endingDrag,
      selectElements(domain, ids, how) {                                                // the spreadsheet's row click: select these elements (points, primitives or edge numbers)
        if (!raw) return false;
        const mode = domain === "point" ? "point" : domain === "edge" ? "edge" : "face";
        if (sel.mode !== mode) api.setMode(mode);
        const found = domain === "point" ? { points: ids } : domain === "prim" ? { faces: ids } : { edges: ids.filter((k) => 2 * k + 1 < raw.edge.length).map((k) => [raw.edge[2 * k], raw.edge[2 * k + 1]]) };
        change(found, how || "replace");
        return true;
      },
      camera: () => camera(), rayAt: (sx, sy) => rayAt(camera(), sx, sy), setAttrView, attrDirty: recolour, attrTouch: recolourTouched, attrViewInfo: () => attrView,
      edgeCount: () => data.nedges,
      view, opt, onSelect: null, onHandle: null, onHandleStart: null,
      setHandles(list) { if (handleDrag) pendingHandles = list || []; else { handles = list || []; schedule(); } },
      guideCount() { return handles.filter((h) => h.kind === "guide").length; },
      handleTips() {                                                                    // for tests: where each handle's grab point is, in canvas pixels
        const cam = camera();
        return shown().map((h) => {
          if (h.kind === "ring") {
            const sp = toScreen(cam, ...ringPoint(h, 0.6));
            return sp ? { id: h.id, comp: h.comp, param: h.param, kind: h.kind, tip: [sp[0], sp[1]], ppu: 1, u: [1, 0] } : null;
          }
          const g = handleGeometry(cam, h);
          if (h.kind === "uniform" || h.kind === "plane") { const corners=planeCorners(cam,h); const s0=corners&&corners.every(Boolean)?[corners.reduce((a,p)=>a+p[0],0)/4,corners.reduce((a,p)=>a+p[1],0)/4]:g ? g.s0 : toScreen(cam, ...h.origin); return s0 ? { id: h.id, comp: h.comp, param: h.param, kind: h.kind, tip: [s0[0], s0[1]], ppu: 1, u: [1, -1] } : null; }
          return g ? { id: h.id, comp: h.comp, param: h.param, kind: h.kind, tip: g.tip, ppu: g.ppu, u: g.u } : null;
        }).filter(Boolean);
      },
      ringScreen(id, t) {                                                               // for tests: a point on a ring at angle t, in canvas pixels
        const h = (selHandles.length ? selHandles : handles).find((x) => x.id === id && x.kind === "ring");
        return h ? toScreen(camera(), ...ringPoint(h, t)) : null;
      },
      tool: "move", hasToolHandles() { return (selHandles.length ? selHandles : handles).some((h) => h.tool); },
      setTool(t) { if (["move", "rotate", "scale"].includes(t)) { api.tool = t; schedule(); } },
      set(head, arrays, keepCamera) {
        measurementCache=null;
        api.previewObjects=head.proxy?.active?[]:(head.preview_objects||[]);api.referenceObjects=head.template_reduced?[]:(head.reference_objects||[]);if(!api.previewObjects.some(o=>o.node===api.activeObject))api.activeObject=api.previewObjects[0]?.node??head.node;api.primaryNode=head.node;
        let keepIndices = false;
        if (head.same_topology) {                                                         // only positions (and colours) came: reuse the rest
          if (!raw || raw.topology !== head.topology || raw.P.length !== arrays.P.length) return false;
          arrays = Object.assign({}, arrays, { tri: raw.tri, edge: raw.edge, tri_prim: raw.triPrim });
          keepIndices = true;
        }
        templateRaw=arrays.template_P?{P:arrays.template_P,edge:arrays.template_edge,tri:arrays.template_tri,triPrim:arrays.template_tri_prim}:null;
        api.templatePoints=templateRaw?templateRaw.P.length/3:0;
        gl.bindVertexArray(templateVao);
        gl.bindBuffer(gl.ARRAY_BUFFER,templatePos);gl.bufferData(gl.ARRAY_BUFFER,templateRaw?.P||new Float32Array(),gl.STATIC_DRAW);
        gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER,templateEdge);gl.bufferData(gl.ELEMENT_ARRAY_BUFFER,templateRaw?.edge||new Uint32Array(),gl.STATIC_DRAW);
        gl.bindVertexArray(null);
        // An inserted transform becomes the preview node mid-drag. Keep the
        // camera and selection stable while its asynchronous result arrives.
        keepCamera = keepCamera || !!handleDrag;
        const same = keepIndices || (keepCamera && data.npoints === arrays.P.length / 3 && data.ntris === arrays.tri.length / 3 &&
          data.nedges === arrays.edge.length / 2);                                      // same topology: indices still mean the same, so the selection stays
        upload(head, arrays, keepIndices);
        api.lastUpload = keepIndices ? "positions" : "all";
        if (!keepCamera && !fly) frameAll(); else schedule();
        if (!same && (sel.point.size || sel.edge.size || sel.face.size)) change({}, "replace");   // other geometry: drop the selection
        else rebuildOverlay();
        return true;
      },
      instanceStats, sectionPlane, isClipped,
      setSection(o) { Object.assign(opt.section, o); schedule(); },
      drawStats() { return { ...stats, instances: instanceStats() }; },
      clear() { measurementCache=null;clearInstances(); templateRaw=null;api.templatePoints=0;api.selectableTemplates=[];handles = []; data.npoints = 0; raw = null; Object.assign(api, { tris: 0, edges: 0, points: 0 }); change({}, "replace"); schedule(); },
      frameAll, frameSelection, viewAxis,
      oppositeView(){const key=Object.keys(VIEWS).find(k=>Math.abs(Math.sin((view.yaw-VIEWS[k][0])/2))<1e-5&&Math.abs(view.pitch-VIEWS[k][1])<1e-5);view.ortho=true;if(key)viewAxis(key,true,true);else goTo(view.yaw+Math.PI,-view.pitch,true);schedule();},
      redraw: schedule, bounds: () => data.bounds,
      normalSegments() { if (nrmDirty) buildNormals(); return Array.from(nrmLines); },   // for tests: [cx, cy, cz, tx, ty, tz] per primitive
      setOrtho(on) { view.ortho = !!on; schedule(); return view.ortho; },
      toggleOrtho() { return api.setOrtho(!view.ortho); },
      setMode(m) {
        if (!["view", "point", "edge", "face", "piece"].includes(m)) return;
        const source=sel.mode==="view"?(sel.lastMode||"point"):sel.mode;
        if(m==="view"){
          sel.lastMode=source;sel.mode=m;canvas.style.cursor="grab";
          ov.points=ov.edgeIdx=ov.faceIdx=0;if(api.onSelect)api.onSelect(summary());schedule();return;
        }
        const found={points:[],edges:[],faces:[]};
        if(raw&&m!==source){
          const es=[];for(let i=0;i<raw.edge.length;i+=2)es.push([raw.edge[i],raw.edge[i+1]]);
          const faces=new Map();for(let i=0;i<raw.tri.length;i+=3){const f=raw.triPrim[i/3];if(!faces.has(f))faces.set(f,new Set());for(let j=0;j<3;j++)faces.get(f).add(raw.tri[i+j]);}
          const points=(source==="point"||source==="piece")?new Set(sel.point):source==="edge"?new Set([...sel.edge.values()].flat()):new Set([...sel.face].flatMap(f=>[...(faces.get(f)||[])]));
          if(m==="point")found.points=[...points];
          else if(m==="edge")found.edges=es.filter(e=>source==="face"?[...sel.face].some(f=>faces.get(f)?.has(e[0])&&faces.get(f)?.has(e[1])):points.has(e[0])&&points.has(e[1]));
          else{
            if(source==="face"||source==="piece")found.faces=[...sel.face];
            else for(const [f,vs]of faces){const boundary=es.filter(e=>vs.has(e[0])&&vs.has(e[1]));
              if(source==="point"?[...vs].every(v=>points.has(v)):boundary.length&&boundary.every(e=>sel.edge.has(edgeKey(...e))))found.faces.push(f);
            }
            if(m==="piece"){found.faces=piecesOf(found.faces);found.points=[...points];}
          }
        }else Object.assign(found,{points:[...sel.point],edges:[...sel.edge.values()],faces:[...sel.face]});
        sel.mode=m;sel.lastMode=m;canvas.style.cursor="crosshair";change(found,"replace");schedule();
      },
      selection: summary,
      selectionInfo() {                                                                   // the selected points and where they are: the anchor of a move/rotate/scale gizmo
        if (!raw) return null;
        const ids = new Set();
        if (sel.mode === "point" || sel.mode === "piece") for (const i of sel.point) ids.add(i);
        else if (sel.mode === "edge") for (const [, e] of sel.edge) { ids.add(e[0]); ids.add(e[1]); }
        else if (sel.mode === "face" || sel.mode === "piece") {
          if (sel.face.size) for (let i = 0; i < raw.triPrim.length; i++) if (sel.face.has(raw.triPrim[i])) for (let c = 0; c < 3; c++) ids.add(raw.tri[3 * i + c]);
        }
        if (!ids.size) return null;
        const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
        const median=[0,0,0];for(const i of ids)for(let c=0;c<3;c++)median[c]+=raw.P[i*3+c]/ids.size;
        let active=Array.from(raw.P.slice([...ids].at(-1)*3,[...ids].at(-1)*3+3));
        if(sel.mode==='edge'&&sel.edge.size){const [a,b]=[...sel.edge.values()].at(-1);active=[0,1,2].map(c=>(raw.P[a*3+c]+raw.P[b*3+c])/2);}
        if((sel.mode==='face'||sel.mode==='piece')&&sel.face.size){const f=[...sel.face].at(-1),vertices=new Set();for(let i=0;i<raw.triPrim.length;i++)if(raw.triPrim[i]===f)for(let j=0;j<3;j++)vertices.add(raw.tri[i*3+j]);if(vertices.size)active=[0,1,2].map(c=>[...vertices].reduce((s,i)=>s+raw.P[i*3+c],0)/vertices.size);}
        for (const i of ids) for (let c = 0; c < 3; c++) { const v = raw.P[3 * i + c]; if (v < lo[c]) lo[c] = v; if (v > hi[c]) hi[c] = v; }
        return { points: [...ids].sort((a, b) => a - b), center: [(lo[0] + hi[0]) / 2, (lo[1] + hi[1]) / 2, (lo[2] + hi[2]) / 2],
          median,active,radius: Math.max(Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2, 1e-3) };
      },
      setSelectionHandles(list) { if (handleDrag) pendingSel = list || []; else { selHandles = list || []; schedule(); } },
      clearSelection() { change({}, "replace"); },
      restoreSelection(snapshot){if(snapshot.mode!==sel.mode)api.setMode(snapshot.mode);change(snapshot,'replace');},
      regionSelect(region,how){if(raw&&sel.mode!=='view'&&!(api.canPick&&!api.canPick()))change(boxPick(camera(),0,0,0,0,region),how||'replace');},
      pickAt(sx, sy, how) {                                                             // CSS pixels inside the canvas
        if (!raw || sel.mode === "view" || (api.canPick && !api.canPick())) return false;
        const cam = camera();
        if (sel.mode === "point") { const i = pickPoint(cam, sx, sy, 12); change({ points: i >= 0 ? [i] : [] }, i < 0 && how === "replace" ? "replace" : how); }
        else if (sel.mode === "edge") { const e = pickEdge(cam, sx, sy, 8); change({ edges: e ? [e] : [] }, !e && how === "replace" ? "replace" : how); }
        else { const f = pickFace(cam, sx, sy);if(sel.mode==='piece'){const p=pickPoint(cam,sx,sy,12),e=pickEdge(cam,sx,sy,8);change({faces:f<0?[]:[f],points:f<0&&p>=0?[p]:[],edges:f<0&&p<0&&e?[e]:[]},how);}else change({faces:f<0?[]:[f]},how); }
        return true;
      },
      boxSelect(x0, y0, x1, y1, how) { if (raw && sel.mode !== "view" && !(api.canPick && !api.canPick())) change(boxPick(camera(), x0, y0, x1, y1), how || "replace"); },
      selectAll() {
        if (!raw || sel.mode === "view" || (api.canPick && !api.canPick())) return;
        const all = { points: [], edges: [], faces: [] }, active=activeObject(),pointStart=active?.point_start??0, pointEnd=active?pointStart+active.point_count:raw.P.length/3,primStart=active?.prim_start??0, primEnd=active?primStart+active.prim_count:Infinity;
        if (sel.mode === "point") for (let i = pointStart; i < pointEnd; i++) all.points.push(i);
        else if (sel.mode === "edge") {for (let k = 0; k < raw.edge.length / 2; k++) if(raw.edge[2*k]>=pointStart&&raw.edge[2*k+1]>=pointStart&&raw.edge[2*k]<pointEnd&&raw.edge[2*k+1]<pointEnd)all.edges.push([raw.edge[2 * k], raw.edge[2 * k + 1]]);}
        else all.faces = [...new Set(raw.triPrim)].filter(f=>sel.mode==="piece"||(f>=primStart&&f<primEnd));
        if(sel.mode==="piece"){all.objects=(api.previewObjects||[]).filter(o=>o.point_count>0).map(o=>o.node);all.points=[];all.edges=[];all.faces=[];if(!api.previewObjects?.length)all.points=Array.from({length:raw.P.length/3},(_,i)=>i);}
        const keepActive=api.activeObject;change(all, "replace");api.activeObject=keepActive;
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
