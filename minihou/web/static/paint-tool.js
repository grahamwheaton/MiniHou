/* Attribute paint and Quick Assign.
 *
 * Paint mode brushes values of one attribute onto points, faces (primitives) or edges (an edge group) of the viewed geometry. The strokes are
 * worked out here, on the real geometry the viewer holds: a depth buffer is rasterised once per stroke, each dab finds the surface under the
 * cursor, the elements within the brush radius of that spot are blended toward the value, and the colours are redrawn at once. When the
 * button is released the touched elements go to the engine as ONE paint_assign edit (one undo step) on an attrib_paint node, which is made
 * after the viewed node when there is none yet. Quick Assign writes the same edit for the viewport's selection.
 *
 * The module only needs the preview (pv) and a few functions of the editor (host); it makes no request of its own except to read the base values for Erase.
 */
window.MiniPaint = (pv, canvas, host) => {
  const KEY = "minihou.paint";
  const DEFAULTS = { attr: "paint", domain: "point", kind: "float", value: 1, vec: [1, 0, 0], radius: 40, strength: 1, falloff: "smooth", mode: "set",
    min: 0, max: 1, ramp: "heat", visible: true, show: true, tab: "paint",
    mask: "none", maskGroup: "", maskInvert: false, symX: false, symY: false, symZ: false, symAbout: "origin" };
  const st = Object.assign({}, DEFAULTS);
  try { Object.assign(st, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch (e) { /* no storage */ }
  st.on = false;                                                   // paint mode is never restored on its own
  const save = () => { try { const { on, ...rest } = st; localStorage.setItem(KEY, JSON.stringify(rest)); } catch (e) { /* no storage */ } };
  const el = (tag, props, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (k === "class") e.className = v; else if (k === "dataset") Object.assign(e.dataset, v); else if (k.startsWith("on")) e[k] = v; else if (v !== false && v != null) e.setAttribute(k, v === true ? "" : v);
    }
    for (const kid of kids.flat()) if (kid != null) e.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
    return e;
  };
  const comps = () => (st.kind === "vec3" ? 3 : 1);
  const kindOf = () => (st.domain === "edge" ? "int" : st.kind);

  // ---------------------------------------------------------------- what is shown and what the node holds
  let ctx = null;                                                  // {node, head, vals, base, n}: the viewed geometry and the working copy of the attribute
  const elementCount = (head) => (st.domain === "point" ? head.points : st.domain === "prim" ? head.prims : head.edges);
  function defaultValue() { return comps() === 3 ? st.vec.slice() : [st.domain === "edge" ? 0 : 0]; }
  const displayOn = () => st.on || (st.show && !!panel && !panel.classList.contains("hidden"));       // never colour geometry while the panel is closed
  function wantAttr() { return displayOn() ? { name: st.attr, domain: st.domain } : null; }
  const panelOpen = () => !!panel && !panel.classList.contains("hidden");
  function wantMask() { return st.mask === "group" && st.maskGroup && panelOpen() ? { name: st.maskGroup, domain: st.domain } : null; }

  let maskArr = null, maskHead = null;                             // the masking group's membership as the viewer numbers the elements (from the preview)
  function onPreview(head, arrays, node) {
    ctx = null;
    maskArr = null; maskHead = head.mask || null;
    if (head.mask && arrays.mask && !head.mask.missing && !head.mask.reduced && head.mask.name === st.maskGroup && head.mask.domain === st.domain) maskArr = arrays.mask;
    if (!displayOn()) { pv.setAttrView(null); return; }
    if (!head.proxy || head.proxy.active) { pv.setAttrView(null); return; }          // a reduced view's elements are not the real ones
    const n = elementCount(head), nc = comps(), vals = new Float32Array(n * nc);
    const info = head.attr, have = info && !info.missing && !info.reduced && arrays.attr && info.name === st.attr && info.domain === st.domain && info.count === n;
    if (have) {
      const src = arrays.attr, sc = info.comps;
      for (let i = 0; i < n; i++) for (let c = 0; c < nc; c++) vals[i * nc + c] = sc === 1 ? src[i] : (c < sc ? src[i * sc + c] : src[i * sc]);
    } else {                                                                              // nothing there yet: what the new node would start with
      const d = defaultNodeValue();
      for (let i = 0; i < n; i++) for (let c = 0; c < nc; c++) vals[i * nc + c] = d[c];
    }
    ctx = { node, head, vals, n, base: null, baseFor: null, existing: !!have };
    applyView();
    if (panel) refreshInfo();
  }
  const defaultNodeValue = () => (comps() === 3 ? [0, 0, 0] : [0]);
  function applyView() {
    if (!ctx) return;
    const nc = comps(), color = nc === 3;
    pv.setAttrView({ domain: st.domain, comps: nc, values: ctx.vals, min: st.min, max: st.max, ramp: st.ramp, color });
  }

  // ---------------------------------------------------------------- the surface under the cursor: a depth / triangle-id raster, once per stroke
  let ras = null, grid = null;
  function projectAll(cam, P) {
    const n = P.length / 3, out = new Float32Array(n * 3), m = cam.MV, pr = cam.P, w = cam.w, h = cam.h, near = cam.near;
    for (let i = 0; i < n; i++) {
      const x = P[3 * i], y = P[3 * i + 1], z = P[3 * i + 2];
      const vx = m[0] * x + m[4] * y + m[8] * z + m[12], vy = m[1] * x + m[5] * y + m[9] * z + m[13], vz = m[2] * x + m[6] * y + m[10] * z + m[14];
      if (vz > -near) { out[3 * i + 2] = -1; continue; }
      out[3 * i] = (pr[0] * vx / -vz * 0.5 + 0.5) * w; out[3 * i + 1] = (0.5 - pr[5] * vy / -vz * 0.5) * h; out[3 * i + 2] = -vz;
    }
    return out;
  }
  function raster() {
    const raw = pv.geometry(), cam = pv.camera(), S = 0.5, W = Math.max(1, Math.ceil(cam.w * S)), H = Math.max(1, Math.ceil(cam.h * S));
    const depth = new Float32Array(W * H).fill(Infinity), ids = new Int32Array(W * H).fill(-1), sp = projectAll(cam, raw.P), T = raw.tri;
    for (let t = 0, nt = T.length / 3; t < nt; t++) {
      const a = T[3 * t] * 3, b = T[3 * t + 1] * 3, c = T[3 * t + 2] * 3;
      const za = sp[a + 2], zb = sp[b + 2], zc = sp[c + 2];
      if (za < 0 || zb < 0 || zc < 0) continue;
      const ax = sp[a] * S, ay = sp[a + 1] * S, bx = sp[b] * S, by = sp[b + 1] * S, cx = sp[c] * S, cy = sp[c + 1] * S;
      const minX = Math.max(0, Math.floor(Math.min(ax, bx, cx))), maxX = Math.min(W - 1, Math.ceil(Math.max(ax, bx, cx)));
      const minY = Math.max(0, Math.floor(Math.min(ay, by, cy))), maxY = Math.min(H - 1, Math.ceil(Math.max(ay, by, cy)));
      if (minX > maxX || minY > maxY) continue;
      const den = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy);
      if (Math.abs(den) < 1e-12) continue;
      for (let y = minY; y <= maxY; y++) for (let x = minX; x <= maxX; x++) {
        const px = x + 0.5, py = y + 0.5;
        const l0 = ((by - cy) * (px - cx) + (cx - bx) * (py - cy)) / den, l1 = ((cy - ay) * (px - cx) + (ax - cx) * (py - cy)) / den, l2 = 1 - l0 - l1;
        if (l0 < -1e-6 || l1 < -1e-6 || l2 < -1e-6) continue;
        const z = l0 * za + l1 * zb + l2 * zc, k = y * W + x;
        if (z < depth[k]) { depth[k] = z; ids[k] = t; }
      }
    }
    ras = { W, H, S, depth, ids, cam, raw };
  }
  function surfaceAt(sx, sy) {                                     // {p: world point, depth: view depth} or null
    const x = Math.min(ras.W - 1, Math.max(0, Math.floor(sx * ras.S))), y = Math.min(ras.H - 1, Math.max(0, Math.floor(sy * ras.S))), t = ras.ids[y * ras.W + x];
    if (t < 0) return null;
    const raw = ras.raw, T = raw.tri, P = raw.P, cam = ras.cam, d = pv.rayAt(sx, sy);
    const a = T[3 * t] * 3, b = T[3 * t + 1] * 3, c = T[3 * t + 2] * 3;
    const e1 = [P[b] - P[a], P[b + 1] - P[a + 1], P[b + 2] - P[a + 2]], e2 = [P[c] - P[a], P[c + 1] - P[a + 1], P[c + 2] - P[a + 2]];
    const pv_ = [d[1] * e2[2] - d[2] * e2[1], d[2] * e2[0] - d[0] * e2[2], d[0] * e2[1] - d[1] * e2[0]], det = e1[0] * pv_[0] + e1[1] * pv_[1] + e1[2] * pv_[2];
    let tt;
    if (Math.abs(det) < 1e-18) tt = null;
    else {
      const inv = 1 / det, tv = [cam.eye[0] - P[a], cam.eye[1] - P[a + 1], cam.eye[2] - P[a + 2]];
      const qv = [tv[1] * e1[2] - tv[2] * e1[1], tv[2] * e1[0] - tv[0] * e1[2], tv[0] * e1[1] - tv[1] * e1[0]];
      tt = (e2[0] * qv[0] + e2[1] * qv[1] + e2[2] * qv[2]) * inv;
    }
    if (tt == null || !(tt > 0)) return null;
    return { p: [cam.eye[0] + d[0] * tt, cam.eye[1] + d[1] * tt, cam.eye[2] + d[2] * tt], depth: ras.depth[y * ras.W + x] };
  }

  // ---------------------------------------------------------------- the elements, in a grid, with their positions
  function elementPositions(raw, head) {
    if (st.domain === "point") return raw.P;
    const out = [];
    if (st.domain === "edge") {
      const E = raw.edge, n = E.length / 2, o = new Float32Array(n * 3);
      for (let k = 0; k < n; k++) for (let c = 0; c < 3; c++) o[3 * k + c] = (raw.P[E[2 * k] * 3 + c] + raw.P[E[2 * k + 1] * 3 + c]) / 2;
      return o;
    }
    const np = head.prims, acc = new Float64Array(np * 3), cnt = new Float64Array(np), T = raw.tri, TP = raw.triPrim;
    for (let t = 0; t < TP.length; t++) {
      const f = TP[t]; if (f >= np) continue;
      for (let c = 0; c < 3; c++) acc[3 * f + c] += (raw.P[T[3 * t] * 3 + c] + raw.P[T[3 * t + 1] * 3 + c] + raw.P[T[3 * t + 2] * 3 + c]) / 3;
      cnt[f]++;
    }
    const o = new Float32Array(np * 3);
    for (let f = 0; f < np; f++) if (cnt[f]) for (let c = 0; c < 3; c++) o[3 * f + c] = acc[3 * f + c] / cnt[f];
    return o;
  }
  function buildGrid(pos) {
    const n = pos.length / 3, lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < n; i++) for (let c = 0; c < 3; c++) { const v = pos[3 * i + c]; if (v < lo[c]) lo[c] = v; if (v > hi[c]) hi[c] = v; }
    if (!n) return { pos, n, empty: true };
    const ext = [0, 1, 2].map((c) => hi[c] - lo[c]), diag = Math.max(Math.hypot(...ext), 1e-9);
    let cell = diag / 2, dims = [1, 1, 1];
    for (let k = 0; k < 40; k++) {
      dims = ext.map((e) => Math.max(1, Math.ceil(e / cell)));
      if (dims[0] * dims[1] * dims[2] >= n / 2 || cell < diag * 1e-5) break;
      cell *= 0.7;
    }
    while (dims[0] * dims[1] * dims[2] > 4e6) { cell *= 1.3; dims = ext.map((e) => Math.max(1, Math.ceil(e / cell))); }
    const nc = dims[0] * dims[1] * dims[2], start = new Int32Array(nc + 1), cellOf = new Int32Array(n);
    const ix = (v, c) => Math.min(dims[c] - 1, Math.max(0, Math.floor((v - lo[c]) / cell)));
    for (let i = 0; i < n; i++) { const k = ix(pos[3 * i], 0) + dims[0] * (ix(pos[3 * i + 1], 1) + dims[1] * ix(pos[3 * i + 2], 2)); cellOf[i] = k; start[k + 1]++; }
    for (let k = 0; k < nc; k++) start[k + 1] += start[k];
    const fill = start.slice(0, nc), items = new Int32Array(n);
    for (let i = 0; i < n; i++) items[fill[cellOf[i]]++] = i;
    return { pos, n, lo, cell, dims, start, items, ix };
  }
  function query(g, c, r, fn) {
    if (g.empty) return;
    const lo = [0, 1, 2].map((k) => g.ix(c[k] - r, k)), hi = [0, 1, 2].map((k) => g.ix(c[k] + r, k)), r2 = r * r, P = g.pos;
    for (let z = lo[2]; z <= hi[2]; z++) for (let y = lo[1]; y <= hi[1]; y++) for (let x = lo[0]; x <= hi[0]; x++) {
      const k = x + g.dims[0] * (y + g.dims[1] * z);
      for (let j = g.start[k]; j < g.start[k + 1]; j++) {
        const i = g.items[j], dx = P[3 * i] - c[0], dy = P[3 * i + 1] - c[1], dz = P[3 * i + 2] - c[2], d2 = dx * dx + dy * dy + dz * dz;
        if (d2 <= r2) fn(i, Math.sqrt(d2));
      }
    }
  }

  // ---------------------------------------------------------------- one dab
  const falloffOf = (t) => (st.falloff === "constant" ? (t < 1 ? 1 : 0) : st.falloff === "linear" ? Math.max(0, 1 - t) : (1 - Math.min(1, t) ** 2) ** 2);
  let stroke = null;
  function mirrorCentres(p) {                                      // the brush position and its reflections across the enabled planes
    const axes = [st.symX, st.symY, st.symZ], on = [0, 1, 2].filter((a) => axes[a]);
    const out = [{ p, primary: true }];
    if (!on.length) return out;
    const b = ctx.head.bounds, about = [0, 1, 2].map((a) => (st.symAbout === "bounds" && b ? (b[0][a] + b[1][a]) / 2 : 0));
    for (let m = 1; m < 1 << on.length; m++) {
      const q = p.slice();
      on.forEach((a, k) => { if (m & (1 << k)) q[a] = 2 * about[a] - q[a]; });
      out.push({ p: q, primary: false });
    }
    return out;
  }
  function dab(sx, sy, mode) {
    const hit = surfaceAt(sx, sy);
    if (!hit) return;
    const cam = ras.cam, r = (st.radius * 2 * hit.depth) / (cam.P[5] * cam.h);          // the brush radius in world units at the surface
    const nc = comps(), vals = ctx.vals, picks = [], allow = stroke.allow, seen = stroke.seen, wbuf = stroke.wbuf, no = ++stroke.dabNo, order = [];
    for (const c of mirrorCentres(hit.p)) {
      query(stroke.grid, c.p, r, (i, d) => {
        if (c.primary && st.visible) {                                                      // not through the surface: its depth at that pixel must be about the element's
          const q = stroke.proj ? stroke.proj(i) : null;
          if (q && q[2] > 0) {
            const x = Math.floor(q[0] * ras.S), y = Math.floor(q[1] * ras.S);
            if (x >= 0 && y >= 0 && x < ras.W && y < ras.H && q[2] > ras.depth[y * ras.W + x] + 0.03 * q[2] + r * 0.35) return;
          }
        }                                                                                   // a mirrored brush sits on the far side: there is no surface of ours to test it against
        const w = falloffOf(d / r) * st.strength;
        if (seen[i] !== no) { seen[i] = no; wbuf[i] = w; order.push(i); } else if (w > wbuf[i]) wbuf[i] = w;   // an element in reach of two brushes takes the stronger, not both
      });
    }
    for (const i of order) picks.push(i, wbuf[i]);
    if (!picks.length) return;
    const target = comps() === 3 ? st.vec : [st.value];
    let avg = null;
    if (mode === "smooth") {                                                                // the weighted mean of what is there now (masked-out neighbours still count)
      avg = new Float64Array(nc); let wsum = 0;
      for (let j = 0; j < picks.length; j += 2) { const i = picks[j], w = picks[j + 1]; wsum += w; for (let c = 0; c < nc; c++) avg[c] += vals[i * nc + c] * w; }
      if (wsum <= 0) return;
      for (let c = 0; c < nc; c++) avg[c] /= wsum;
    }
    const gain = mode === "add" || mode === "subtract" || mode === "multiply" ? 0.25 : 1;      // dabs overlap about four deep, so these build up at roughly Strength per pass
    for (let j = 0; j < picks.length; j += 2) {
      const i = picks[j], w = picks[j + 1] * gain;
      if (w <= 0 || (allow && !allow[i])) continue;
      for (let c = 0; c < nc; c++) {
        const cur = vals[i * nc + c], t = target[c];
        let v;
        if (mode === "add") v = cur + t * w;
        else if (mode === "subtract") v = cur - t * w;
        else if (mode === "multiply") v = cur * (1 + (t - 1) * w);
        else if (mode === "smooth") v = cur + (avg[c] - cur) * w;
        else if (mode === "erase") v = cur + (baseValue(i, c) - cur) * w;
        else v = cur + (t - cur) * w;
        vals[i * nc + c] = v;
      }
      stroke.touched.add(i); stroke.fresh.add(i);
    }
    stroke.dirty = true;
  }
  const baseValue = (i, c) => (ctx.base ? ctx.base[i * comps() + c] : defaultNodeValue()[c]);

  // ---------------------------------------------------------------- the mask: which elements a brush (or Assign) may change
  function selectedForMask() {                                     // the viewport selection as elements of the painted domain (faces and edges stand for the points they use)
    const raw = pv.geometry();
    let ids = host.selectionFor(st.domain);
    if (ids || !raw) return ids;
    if (st.domain === "point") {
      const faces = host.selectionFor("prim"), edges = host.selectionFor("edge");
      if (faces) { const f = new Set(faces), out = new Set(); for (let t = 0; t < raw.triPrim.length; t++) if (f.has(raw.triPrim[t])) for (let c = 0; c < 3; c++) out.add(raw.tri[3 * t + c]); return [...out]; }
      if (edges) { const out = new Set(); for (const k of edges) { out.add(raw.edge[2 * k]); out.add(raw.edge[2 * k + 1]); } return [...out]; }
    }
    return null;
  }
  let maskSnap = null;                                             // {domain, ids}: the selection taken as the mask (entering paint mode clears the viewport selection, this stays)
  function snapshotMask(quiet) {
    const ids = selectedForMask();
    if (!ids || !ids.length) { if (!quiet) host.toast("Select some " + (st.domain === "point" ? "points or faces" : st.domain === "prim" ? "faces" : "edges") + " first (1, 2, 3), then set the mask."); return false; }
    maskSnap = { domain: st.domain, ids: ids.slice() };
    refreshInfo();
    return true;
  }
  function buildAllow() {                                          // {allow: Uint8Array|null, why}: null allow = everything
    if (st.mask === "none" || !ctx) return { allow: null, why: "" };
    const n = ctx.n, allow = new Uint8Array(n), inv = st.maskInvert;
    if (st.mask === "group") {
      if (!st.maskGroup) return { allow: null, why: "Mask is set to a group: name the group." };
      if (!maskArr || maskArr.length !== n) return { allow: null, why: maskHead && maskHead.missing ? `There is no group "${st.maskGroup}" on this geometry.` : `The mask group "${st.maskGroup}" has not loaded yet.` };
      for (let i = 0; i < n; i++) allow[i] = (maskArr[i] !== 0) !== inv ? 1 : 0;
    } else {
      const ids = maskSnap && maskSnap.domain === st.domain ? maskSnap.ids : selectedForMask();
      if (!ids || !ids.length) return { allow: null, why: "The mask is the selection, but there is none: select " + (st.domain === "point" ? "points or faces" : st.domain === "prim" ? "faces" : "edges") + ", then press Set from selection." };
      if (inv) allow.fill(1);
      for (const i of ids) if (i < n) allow[i] = inv ? 0 : 1;
    }
    return { allow, why: "" };
  }

  // ---------------------------------------------------------------- the stroke: pointer events
  const modeFor = (e) => (e.shiftKey ? "erase" : e.ctrlKey || e.metaKey ? "smooth" : st.mode);
  function blocked() {
    if (!ctx) return "There is no geometry to paint on yet.";
    if (!pv.geometry()) return "There is no geometry to paint on yet.";
    if (ctx.head.cook_level != null && ctx.head.cook_level < 1) return "Painting needs the full cook: element numbers at a proxy level are not the real ones. Set the cook Proxy to Full.";
    return "";
  }
  function begin(e) {
    const why = blocked();
    if (why) { host.toast(why); return; }
    const raw = pv.geometry(), r = canvas.getBoundingClientRect();
    const t0 = performance.now();
    const masked = buildAllow();
    if (masked.why) { host.toast(masked.why); return; }
    raster();
    const pos = elementPositions(raw, ctx.head), g = buildGrid(pos), cam = pv.camera();
    api.stats.beginMs = Math.round(performance.now() - t0);
    stroke = { grid: g, touched: new Set(), fresh: new Set(), dirty: false, last: null, mode: modeFor(e), allow: masked.allow, seen: new Int32Array(ctx.n), wbuf: new Float32Array(ctx.n), dabNo: 0, proj: (i) => {
      const x = pos[3 * i], y = pos[3 * i + 1], z = pos[3 * i + 2], m = cam.MV, pr = cam.P;
      const vx = m[0] * x + m[4] * y + m[8] * z + m[12], vy = m[1] * x + m[5] * y + m[9] * z + m[13], vz = m[2] * x + m[6] * y + m[10] * z + m[14];
      if (vz > -cam.near) return null;
      return [(pr[0] * vx / -vz * 0.5 + 0.5) * cam.w, (0.5 - pr[5] * vy / -vz * 0.5) * cam.h, -vz];
    } };
    if (stroke.mode === "erase") ensureBase();
    canvas.setPointerCapture && canvas.setPointerCapture(e.pointerId);
    move(e);
  }
  function move(e) {
    if (!stroke) return;
    const t0 = performance.now();
    const r = canvas.getBoundingClientRect(), x = e.clientX - r.left, y = e.clientY - r.top, mode = stroke.mode === "erase" || modeFor(e) === "erase" ? "erase" : modeFor(e);
    if (mode === "erase") ensureBase();
    const gap = Math.max(2, st.radius * 0.22);
    if (!stroke.last) dab(x, y, mode);
    else {
      const dx = x - stroke.last[0], dy = y - stroke.last[1], len = Math.hypot(dx, dy), n = Math.max(1, Math.floor(len / gap));
      for (let k = 1; k <= n; k++) dab(stroke.last[0] + (dx * k) / n, stroke.last[1] + (dy * k) / n, mode);
    }
    stroke.last = [x, y];
    if (stroke.dirty) { stroke.dirty = false; pv.attrTouch([...stroke.fresh]); stroke.fresh.clear(); }
    api.stats.strokeMs = Math.round(performance.now() - t0);
  }
  async function end(e, cancelled) {
    const s = stroke; stroke = null;
    if (!s) return;
    try { canvas.releasePointerCapture && canvas.releasePointerCapture(e.pointerId); } catch (err) { /* already released */ }
    if (cancelled || !s.touched.size) return;
    await commit([...s.touched].sort((a, b) => a - b));
  }

  // ---------------------------------------------------------------- writing the edit
  function ensureBase() {
    if (!ctx || ctx.baseFor === ctx.node) return;
    ctx.baseFor = ctx.node;
    host.fetchBase(ctx.node, { name: st.attr, domain: st.domain }).then((res) => {            // the input's value of the attribute, for Erase
      if (!ctx || !res) return;
      const nc = comps(), n = ctx.n, b = new Float32Array(n * nc), d = defaultNodeValue();
      for (let i = 0; i < n; i++) for (let c = 0; c < nc; c++) b[i * nc + c] = res.values ? (res.comps === 1 ? res.values[i] : (c < res.comps ? res.values[i * res.comps + c] : res.values[i * res.comps])) : d[c];
      ctx.base = b;
    });
  }
  function rows(ids) {
    const nc = comps(), out = [], vals = ctx.vals, round = kindOf() === "int";
    for (const i of ids) {
      const row = [];
      for (let c = 0; c < nc; c++) { const v = vals[i * nc + c]; row.push(round ? Math.round(v) : Math.round(v * 1e6) / 1e6); }
      out.push(nc === 1 ? row[0] : row);
    }
    return out;
  }
  const specNow = () => ({ attr: st.attr, domain: st.domain, kind: kindOf() });
  let lastNode = null;
  function target(spec) {                                          // the paint node to write to: the viewed one when it matches, else a new one after it
    const v = host.viewedNode(), info = v && host.nodeInfo(v);
    const fits = (i) => !!i && i.type === "attrib_paint" && i.params.attribute === spec.attr && i.params.domain === spec.domain && i.params.kind === spec.kind;
    if (fits(info)) return { id: v };
    const last = lastNode && host.nodeInfo(lastNode);                // the node the previous edit made, before the view has caught up with it
    if (fits(last)) return { id: lastNode };
    return { id: null, after: v };
  }
  async function writeOps(spec, extra) {                           // extra: paint_assign ops without the node id
    const t = target(spec), ops = [];
    let id = t.id;
    if (!id) {
      if (!t.after) { host.toast("Nothing to attach the paint node to."); return null; }
      id = host.uniqueId("paint");
      ops.push({ op: "insert", after: t.after, type: "attrib_paint", id, params: { attribute: spec.attr, domain: spec.domain, kind: spec.kind } });
    }
    for (const op of extra) ops.push({ ...op, op: "paint_assign", id });
    const r = await host.edit(ops);
    if (r) { lastNode = id; host.select([id]); }
    return r ? id : null;
  }
  async function commit(ids) {
    if (!ids.length) return;
    if (ids.length > 3_000_000) { host.toast("That is too many elements for one stroke."); return; }
    const nc = comps(), erase = [], assign = [];
    for (const i of ids) {
      let same = !!ctx.base;
      if (same) for (let c = 0; c < nc; c++) if (Math.abs(ctx.vals[i * nc + c] - ctx.base[i * nc + c]) > 1e-4) { same = false; break; }
      (same ? erase : assign).push(i);
    }
    const ops = [];
    if (assign.length) ops.push({ indices: assign, values: rows(assign) });
    if (erase.length) ops.push({ indices: erase, erase: true });
    await writeOps(specNow(), ops);
  }

  // ---------------------------------------------------------------- quick assign
  async function assignSelection(ids) {
    if (!ids || !ids.length) { host.toast("Select some " + (st.domain === "point" ? "points" : st.domain === "prim" ? "faces" : "edges") + " first, or use Assign to all."); return; }
    const nc = comps(), v = kindOf() === "int" ? Math.round(st.value) : st.value, val = nc === 3 ? st.vec.slice() : st.domain === "edge" ? 1 : v;
    await writeOps(specNow(), [{ indices: ids, value: val }]);
  }
  async function assignAll() {
    const nc = comps(), val = nc === 3 ? st.vec.slice() : (kindOf() === "int" ? Math.round(st.value) : st.value);
    if (st.mask !== "none") {                                      // with a mask, "all" is the masked elements
      if (!ctx) { host.toast("Assigning through a mask needs the real geometry in the view."); return; }
      const m = buildAllow();
      if (m.why) { host.toast(m.why); return; }
      const ids = []; for (let i = 0; i < ctx.n; i++) if (m.allow[i]) ids.push(i);
      if (!ids.length) { host.toast("The mask holds no elements."); return; }
      await assignSelection(ids);
      return;
    }
    await writeOps(specNow(), [{ clear: true, default: val }]);
  }
  async function clearPainted() { await writeOps(specNow(), [{ clear: true }]); }

  // ---------------------------------------------------------------- the panel
  let panel = null, active = false, cursor = null, info = null;
  const numberInput = (get, set, props) => { const i = el("input", Object.assign({ type: "number", step: "any" }, props)); i.value = get(); i.onchange = () => { const v = Number(i.value); if (Number.isFinite(v)) { set(v); save(); } }; return i; };
  function select(get, set, options, props) {
    const s = el("select", props, options.map(([v, l]) => el("option", { value: v }, l)));
    s.value = get(); s.onchange = () => { set(s.value); save(); };
    return s;
  }
  function buildPanel() {
    const attrList = el("datalist", { id: "paint-attr-names" });
    const attrInput = el("input", { type: "text", list: "paint-attr-names", class: "pa-attr", "aria-label": "Attribute name" });
    attrInput.value = st.attr;
    attrInput.onchange = () => { if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(attrInput.value) && attrInput.value !== "P") { st.attr = attrInput.value; save(); host.refresh(); } else { attrInput.value = st.attr; host.toast("An attribute name is letters, digits and _, and not P."); } };
    const domainSel = select(() => st.domain, (v) => { st.domain = v; if (v === "edge") st.kind = "int"; kindSel.value = kindOf(); kindSel.disabled = v === "edge"; syncValueRows(); host.refresh(); host.groupNames(v).then(fillGroups); },
      [["point", "Points"], ["prim", "Faces / primitives"], ["edge", "Edges (edge group)"]], { class: "pa-domain", "aria-label": "Domain" });
    const kindSel = select(() => kindOf(), (v) => { st.kind = v; syncValueRows(); host.refresh(); }, [["float", "float"], ["int", "int"], ["vec3", "vector / colour"]], { class: "pa-kind", "aria-label": "Type" });
    kindSel.disabled = st.domain === "edge";
    const valueNum = numberInput(() => st.value, (v) => (st.value = v), { class: "pa-value", "aria-label": "Value" });
    const vecInputs = [0, 1, 2].map((c) => numberInput(() => st.vec[c], (v) => { st.vec[c] = v; syncColor(); }, { class: "pa-vec", "aria-label": "XYZ"[c], dataset: { c } }));
    const color = el("input", { type: "color", class: "pa-color", "aria-label": "Colour" });
    const hex = (v) => Math.round(Math.min(1, Math.max(0, v)) * 255).toString(16).padStart(2, "0");
    const syncColor = () => { color.value = "#" + hex(st.vec[0]) + hex(st.vec[1]) + hex(st.vec[2]); };
    color.oninput = () => { st.vec = [1, 3, 5].map((k) => parseInt(color.value.slice(k, k + 2), 16) / 255); vecInputs.forEach((i, c) => (i.value = Math.round(st.vec[c] * 1000) / 1000)); save(); };
    syncColor();
    const scalarRow = el("label", { class: "pa-row pa-scalar" }, "Value", valueNum);
    const vecRow = el("label", { class: "pa-row pa-vector" }, "Value", el("span", { class: "pa-vecs" }, vecInputs, color));
    const syncValueRows = () => { const v3 = comps() === 3 && st.domain !== "edge"; scalarRow.classList.toggle("hidden", v3); vecRow.classList.toggle("hidden", !v3); valueNum.step = kindOf() === "int" ? 1 : "any"; };
    const mode = select(() => st.mode, (v) => (st.mode = v), [["set", "Paint"], ["add", "Add"], ["subtract", "Subtract"], ["multiply", "Multiply"], ["smooth", "Smooth"], ["erase", "Erase"]], { class: "pa-mode", "aria-label": "Brush mode" });
    const radius = el("input", { type: "range", min: 4, max: 300, step: 1, class: "pa-radius", "aria-label": "Brush radius" });
    radius.value = st.radius; radius.oninput = () => { st.radius = Number(radius.value); save(); moveCursor(); };
    const strength = el("input", { type: "range", min: 0.01, max: 1, step: 0.01, class: "pa-strength", "aria-label": "Brush strength" });
    strength.value = st.strength; strength.oninput = () => { st.strength = Number(strength.value); save(); };
    const falloff = select(() => st.falloff, (v) => (st.falloff = v), [["smooth", "Smooth"], ["linear", "Linear"], ["constant", "Constant"]], { class: "pa-falloff", "aria-label": "Falloff" });
    const visible = el("input", { type: "checkbox", class: "pa-visible" }); visible.checked = st.visible; visible.onchange = () => { st.visible = visible.checked; save(); };
    const show = el("input", { type: "checkbox", class: "pa-show" }); show.checked = st.show; show.onchange = () => { st.show = show.checked; save(); host.refresh(); };
    const rmin = numberInput(() => st.min, (v) => { st.min = v; applyView(); }, { class: "pa-min", "aria-label": "Range min" });
    const rmax = numberInput(() => st.max, (v) => { st.max = v; applyView(); }, { class: "pa-max", "aria-label": "Range max" });
    const fit = el("button", { type: "button", class: "pa-fit", title: "Set the range to the values the attribute has now", onclick: () => {
      if (!ctx || !ctx.vals.length) return;
      let lo = Infinity, hi = -Infinity;
      for (const v of ctx.vals) { if (v < lo) lo = v; if (v > hi) hi = v; }
      st.min = lo; st.max = hi > lo ? hi : lo + 1; rmin.value = st.min; rmax.value = st.max; save(); applyView();
    } }, "Fit");
    const ramp = select(() => st.ramp, (v) => { st.ramp = v; applyView(); }, [["heat", "Heat"], ["gray", "Gray"]], { class: "pa-ramp", "aria-label": "Colour ramp" });
    const maskSel = select(() => st.mask, (v) => { st.mask = v; if (v === "selection") snapshotMask(true); syncMask(); host.refresh(); }, [["none", "None"], ["selection", "Selection"], ["group", "Group"]], { class: "pa-mask", "aria-label": "Mask" });
    const groupInput = el("input", { type: "text", list: "paint-group-names", class: "pa-maskgroup", "aria-label": "Mask group", placeholder: "group name" });
    groupInput.value = st.maskGroup;
    groupInput.onchange = () => { st.maskGroup = groupInput.value.trim(); save(); host.refresh(); };
    const groupList = el("datalist", { id: "paint-group-names" });
    const invert = el("input", { type: "checkbox", class: "pa-maskinvert" }); invert.checked = st.maskInvert; invert.onchange = () => { st.maskInvert = invert.checked; save(); refreshInfo(); };
    const snap = el("button", { type: "button", class: "pa-masksnap", title: "Use the points, edges or faces selected now as the mask (it is kept when paint mode clears the selection)", onclick: () => snapshotMask(false) }, "Set from selection");
    const maskRow = el("label", { class: "pa-row" }, "Mask", el("span", { class: "pa-maskbox" }, maskSel, groupInput, snap));
    const invertRow = el("label", { class: "pa-row pa-check pa-maskoptions" }, invert, "Invert (paint outside the mask)");
    const syncMask = () => {
      groupInput.classList.toggle("hidden", st.mask !== "group"); snap.classList.toggle("hidden", st.mask !== "selection"); invertRow.classList.toggle("hidden", st.mask === "none");
      const all = panel && panel.querySelector(".pa-assign-all"); if (all) all.textContent = st.mask === "none" ? "Assign to all" : "Assign to mask";
      save(); refreshInfo();
    };
    const symBox = (key, label) => { const c = el("input", { type: "checkbox", class: "pa-sym pa-sym" + label.toLowerCase(), "aria-label": "Mirror " + label }); c.checked = st[key]; c.onchange = () => { st[key] = c.checked; save(); }; return el("label", { class: "pa-symlabel" }, c, label); };
    const about = select(() => st.symAbout, (v) => (st.symAbout = v), [["origin", "World origin"], ["bounds", "Centre of the geometry"]], { class: "pa-symabout", "aria-label": "Mirror about" });
    const symRow = el("div", { class: "pa-row pa-symrow" }, "Mirror", el("span", { class: "pa-symaxes" }, symBox("symX", "X"), symBox("symY", "Y"), symBox("symZ", "Z")));
    const aboutRow = el("label", { class: "pa-row" }, "About", about);
    info = el("div", { class: "pa-info muted" });
    const paintTab = el("div", { class: "pa-tab pa-paint" },
      el("label", { class: "pa-row" }, "Mode", mode), el("label", { class: "pa-row" }, "Radius", radius), el("label", { class: "pa-row" }, "Strength", strength),
      el("label", { class: "pa-row" }, "Falloff", falloff), el("label", { class: "pa-row pa-check" }, visible, "Visible surface only"),
      symRow, aboutRow,
      el("div", { class: "pa-hint muted" }, "Shift: erase · Ctrl: smooth · [ ]: radius · Esc: leave paint"));
    const assignTab = el("div", { class: "pa-tab pa-assign hidden" },
      el("div", { class: "pa-selinfo muted" }),
      el("div", { class: "pa-buttons" },
        el("button", { type: "button", class: "primary pa-assign-sel", onclick: () => assignSelection(host.selectionFor(st.domain)) }, "Assign to selection"),
        el("button", { type: "button", class: "pa-assign-all", onclick: assignAll }, "Assign to all"),
        el("button", { type: "button", class: "pa-clear", onclick: clearPainted }, "Clear painted")),
      el("div", { class: "pa-hint muted" }, "Select points (1), edges (2) or faces (3) in the view, set the attribute and value above, and assign. Each press is one undo step."));
    const tabs = el("div", { class: "pa-tabs" },
      ["paint", "assign"].map((t) => el("button", { type: "button", class: "pa-tabbtn" + (st.tab === t ? " on" : ""), dataset: { tab: t }, onclick: () => setTab(t) }, t === "paint" ? "Paint" : "Quick assign")),
      el("span", { class: "spacer" }), el("button", { type: "button", class: "pa-close", title: "Close", "aria-label": "Close", onclick: () => api.close() }, "×"));
    const p = el("div", { class: "pv-attr-card hidden", id: "attr-card", role: "group", "aria-label": "Attribute paint and assign" }, tabs,
      el("label", { class: "pa-row" }, "Attribute", attrInput), attrList, el("label", { class: "pa-row" }, "Domain", domainSel), el("label", { class: "pa-row" }, "Type", kindSel),
      scalarRow, vecRow, maskRow, groupList, invertRow, paintTab, assignTab,
      el("details", { class: "pa-display" }, el("summary", null, "Display"), el("label", { class: "pa-row pa-check" }, show, "Show the attribute as colour"),
        el("label", { class: "pa-row" }, "Range", el("span", { class: "pa-range" }, rmin, rmax, fit)), el("label", { class: "pa-row" }, "Ramp", ramp)), info);
    syncValueRows();
    p.groupList = groupList; p.syncMask = syncMask;
    p.addEventListener("keydown", (e) => e.stopPropagation());              // typing in a field is not a viewer shortcut
    canvas.parentElement.append(p);
    p.attrList = attrList;
    tabs.style.cursor = "move"; tabs.title = "Drag to move the panel out of the way";                  // it sits over the view: let it be moved
    tabs.addEventListener("pointerdown", (e) => {
      if (e.target.closest("button")) return;
      const wr = canvas.parentElement.getBoundingClientRect(), pr = p.getBoundingClientRect(), dx = e.clientX - pr.left, dy = e.clientY - pr.top;
      const mv = (ev) => { p.style.left = Math.max(0, Math.min(wr.width - 60, ev.clientX - wr.left - dx)) + "px"; p.style.top = Math.max(0, Math.min(wr.height - 30, ev.clientY - wr.top - dy)) + "px"; p.style.right = "auto"; };
      const upx = () => { window.removeEventListener("pointermove", mv, true); window.removeEventListener("pointerup", upx, true); };
      window.addEventListener("pointermove", mv, true); window.addEventListener("pointerup", upx, true);
      e.preventDefault();
    });
    return p;
  }
  function setTab(t) {
    st.tab = t; save();
    panel.querySelectorAll(".pa-tabbtn").forEach((b) => b.classList.toggle("on", b.dataset.tab === t));
    panel.querySelector(".pa-paint").classList.toggle("hidden", t !== "paint");
    panel.querySelector(".pa-assign").classList.toggle("hidden", t !== "assign");
    if (t === "paint" && !active) api.setPaint(true); else if (t === "assign" && active) api.setPaint(false);
    refreshInfo();
  }
  function refreshInfo() {
    if (!panel) return;
    const sel = panel.querySelector(".pa-selinfo"), ids = host.selectionFor(st.domain);
    if (sel) sel.textContent = ids ? `${ids.length} selected ${st.domain === "point" ? "point(s)" : st.domain === "prim" ? "face(s)" : "edge(s)"}` : "Nothing selected for this domain";
    const why = blocked();
    let mtxt = "";
    if (st.mask === "group" && maskHead && !maskHead.reduced) mtxt = maskHead.missing ? ` · no group "${st.maskGroup}"` : maskArr ? ` · mask: ${(st.maskInvert ? maskHead.count - maskHead.members : maskHead.members).toLocaleString()} of ${maskHead.count.toLocaleString()}` : "";
    else if (st.mask === "selection") { const ids = maskSnap && maskSnap.domain === st.domain ? maskSnap.ids : selectedForMask(); mtxt = ` · mask: ${ids ? ids.length.toLocaleString() : 0} selected`; }
    info.textContent = ctx ? (ctx.existing ? `"${st.attr}" on ${ctx.n.toLocaleString()} ${st.domain === "prim" ? "faces" : st.domain === "edge" ? "edges" : "points"}` : `"${st.attr}" does not exist yet: it starts at ${defaultNodeValue().join(", ")}`) + (why ? " · " + why : "") + mtxt : (host.reducedMessage() || "");
  }
  function fillGroups(names) { if (panel) panel.groupList.replaceChildren(...names.map((n) => el("option", { value: n }))); }
  function fillNames(names) {
    if (!panel) return;
    panel.attrList.replaceChildren(...names.map((n) => el("option", { value: n })));
  }
  function moveCursor(e) {
    if (!cursor) return;
    if (e) cursor._at = [e.clientX, e.clientY];
    if (!cursor._at || !active) { cursor.classList.add("hidden"); return; }
    const r = canvas.getBoundingClientRect(), [x, y] = cursor._at;
    if (x < r.left || x > r.right || y < r.top || y > r.bottom) { cursor.classList.add("hidden"); return; }
    cursor.classList.remove("hidden");
    Object.assign(cursor.style, { left: x - r.left - st.radius + "px", top: y - r.top - st.radius + "px", width: st.radius * 2 + "px", height: st.radius * 2 + "px" });
  }

  // ---------------------------------------------------------------- window handlers while paint mode is on
  const down = (e) => {
    if (!active || e.target !== canvas || e.button !== 0 || e.altKey) return;
    e.preventDefault(); e.stopImmediatePropagation();
    if (document.activeElement && document.activeElement !== document.body && panel && panel.contains(document.activeElement)) document.activeElement.blur();   // keys go to the view again
    begin(e);
  };
  const onMove = (e) => { moveCursor(e); if (stroke) { e.preventDefault(); e.stopImmediatePropagation(); move(e); } };
  const up = (e) => { if (stroke) { e.preventDefault(); e.stopImmediatePropagation(); end(e, e.type === "pointercancel"); } };
  const keydown = (e) => {
    if (!active || e.target.closest && e.target.closest("input, select, textarea, [contenteditable]")) return;
    if (e.key === "[" || e.key === "]") { st.radius = Math.max(4, Math.min(300, st.radius * (e.key === "]" ? 1.15 : 1 / 1.15))); save(); if (panel) panel.querySelector(".pa-radius").value = st.radius; moveCursor(); e.preventDefault(); e.stopImmediatePropagation(); }
    else if (e.key === "Escape") { if (stroke) { stroke = null; applyViewFromScratch(); } else api.setPaint(false); e.preventDefault(); e.stopImmediatePropagation(); }
  };
  function applyViewFromScratch() { host.refresh(); }                  // a cancelled stroke: reload the real values
  window.addEventListener("pointerdown", down, true);
  window.addEventListener("pointermove", onMove, true);
  window.addEventListener("pointerup", up, true);
  window.addEventListener("pointercancel", up, true);
  window.addEventListener("keydown", keydown, true);

  const api = {
    state: st, wantAttr, wantMask, onPreview, stats: { beginMs: 0, strokeMs: 0 },
    active: () => active,
    open(tab) {
      if (!panel) { panel = buildPanel(); cursor = el("div", { class: "pv-brush hidden" }); canvas.parentElement.append(cursor); }
      panel.classList.remove("hidden");
      setTab(tab || st.tab);
      host.refresh();
      host.attributeNames(st.domain).then(fillNames);
      host.groupNames(st.domain).then(fillGroups);
      panel.syncMask();
    },
    close() { api.setPaint(false); if (panel) panel.classList.add("hidden"); host.refresh(); },
    isOpen: () => !!panel && !panel.classList.contains("hidden"),
    setPaint(on) {
      on = !!on;
      if (on) {
        const why = host.paintBlocked();
        if (why) { host.toast(why); return false; }
        if (st.mask === "selection" && !maskSnap) snapshotMask(true);              // paint mode clears the viewport selection: keep it as the mask first
        if (!panel || panel.classList.contains("hidden")) api.open("paint");
      }
      if (on === active) return active;
      active = st.on = on;
      canvas.classList.toggle("painting", on);
      if (!on) { stroke = null; if (cursor) cursor.classList.add("hidden"); }
      if (panel && on && st.tab !== "paint") setTab("paint");
      host.paintChanged(on);
      host.refresh();
      return active;
    },
    toggle() { return api.setPaint(!active); },
    refreshInfo, fillNames, fillGroups, applyView, blocked, allowed: () => buildAllow(),
    assign: (spec, ids, value) => writeOps(spec, [{ indices: ids, value }]),                 // spreadsheet cells
    assignValues: (spec, ids, values) => writeOps(spec, [{ indices: ids, values }]),         // pasted cells: one value (or 3-vector) per element
    values: () => (ctx ? { vals: ctx.vals, n: ctx.n, node: ctx.node } : null),
    setBase: (b) => { if (ctx) ctx.base = b; },
  };
  return api;
};
