// MiniHou node editor. Plain JS, no build step. Talks to the engine's HTTP API (see minihou/web/server.py);
// every change goes through POST /api/edit and comes back as a graph snapshot, and other clients' changes
// (Blender sliders, AI tools, file edits) arrive over Server-Sent Events.
"use strict";
(() => {
  const W = 160, H = 46;                       // node size (must match editor.css)
  const $ = (s, el = document) => el.querySelector(s);
  const canvas = $("#canvas"), world = $("#world"), wiresSvg = $("#wires"), nodesEl = $("#nodes"), overlay = $("#overlay");
  const SVGNS = "http://www.w3.org/2000/svg";

  const S = {
    graph: { nodes: {} }, rev: -1, path: null, canUndo: false, canRedo: false,
    types: {}, typeList: [], status: null, root: "",
    sel: new Set(), wireSel: null, view: { x: 80, y: 60, k: 1 },
    online: false, framed: false, dragPos: new Map(), mouse: [200, 200],
  };

  // ---------------------------------------------------------------- DOM helpers (text only, never raw HTML)
  function h(tag, props, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (v == null || v === false) continue;
      if (k === "class") el.className = v;
      else if (k === "style") Object.assign(el.style, v);
      else if (k === "dataset") Object.assign(el.dataset, v);
      else if (k.startsWith("on")) el.addEventListener(k.slice(2), v);
      else if (k in el) el[k] = v;
      else el.setAttribute(k, v);
    }
    for (const c of kids.flat(Infinity)) if (c != null && c !== false) el.append(c instanceof Node ? c : String(c));
    return el;
  }
  function svgEl(tag, attrs) {
    const el = document.createElementNS(SVGNS, tag);
    for (const [k, v] of Object.entries(attrs || {})) if (v != null) el.setAttribute(k, v);
    return el;
  }
  const fmt = (n) => (typeof n === "number" ? n.toLocaleString() : n);
  const round = (v) => (Math.abs(v) >= 100 ? +v.toFixed(2) : +v.toPrecision(4));

  let toastTimer = null;
  function toast(msg, isErr = true) {
    let t = $("#toast");
    if (!t) { t = h("div", { id: "toast" }); document.body.append(t); }
    t.textContent = msg;
    t.className = isErr ? "err" : "";
    t.style.display = "block";
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (t.style.display = "none"), isErr ? 6000 : 2500);
  }

  // ---------------------------------------------------------------- API
  async function api(path, body) {
    const opt = body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) };
    const r = await fetch(path, opt);
    let data = null;
    try { data = await r.json(); } catch (e) { data = { error: r.statusText }; }
    if (!r.ok) throw new Error(data.error || r.statusText);
    return data;
  }
  async function edit(ops) {
    try {
      const r = await api("/api/edit", { ops, origin: "editor" });
      applySnapshot(r.graph);
      return r;
    } catch (e) {
      toast(e.message);
      return null;
    }
  }

  // Parameter changes are coalesced: one request in flight, the latest values win (smooth slider drags).
  const pendingSet = new Map();
  let setInFlight = false;
  function queueSet(id, name, value) {
    const p = pendingSet.get(id) || {};
    p[name] = value;
    pendingSet.set(id, p);
    if (!setInFlight) flushSets();
  }
  async function flushSets() {
    if (!pendingSet.size) return;
    setInFlight = true;
    const ops = [...pendingSet].map(([id, params]) => ({ op: "set", id, params }));
    pendingSet.clear();
    await edit(ops);
    setInFlight = false;
    if (pendingSet.size) flushSets();
  }

  // ---------------------------------------------------------------- state from the engine
  async function loadState() {
    try {
      const st = await api("/api/state");
      setTypes(st.nodes, st.library);
      S.root = st.root;
      S.status = st.status;
      S.status.clients = st.link.clients;
      S.rev = -1;                                    // the engine may have restarted: accept its revision
      applySnapshot(st);
    } catch (e) {
      setOnline(false);
    }
  }
  let libErrors = new Set();
  function setTypes(nodes, library) {
    S.types = {};
    for (const t of nodes) S.types[t.name] = t;
    S.typeList = nodes.slice().sort((a, b) => (a.category || "").localeCompare(b.category || "") || a.name.localeCompare(b.name));
    S.library = library || null;
    const errs = new Set(((library && library.files) || []).filter((f) => f.error).map((f) => relPath(f.file) + ": " + f.error));
    const fresh = [...errs].filter((e) => !libErrors.has(e));
    if (fresh.length) toast("Node library: " + fresh.join("\n"));
    libErrors = errs;
  }
  function applySnapshot(snap) {
    if (!snap || snap.rev < S.rev) return;
    const fileChanged = snap.path !== S.path;
    S.rev = snap.rev; S.graph = snap.graph; S.path = snap.path;
    S.canUndo = snap.can_undo; S.canRedo = snap.can_redo;
    for (const id of [...S.sel]) if (!S.graph.nodes[id]) S.sel.delete(id);
    if (S.wireSel && !wireExists(S.wireSel)) S.wireSel = null;
    render();
    if (fileChanged || !S.framed) { frame(false); S.framed = true; }
  }
  function connectEvents() {
    const es = new EventSource("/api/events");
    es.addEventListener("open", () => { setOnline(true); loadState(); });
    es.addEventListener("graph", (e) => applySnapshot(JSON.parse(e.data)));
    es.addEventListener("nodes", (e) => {
      const d = JSON.parse(e.data);
      setTypes(d.nodes, d.library);
      panelKey = null;                               // parameter panels may show a reloaded node type
      for (const el of nodesEl.children) el._struct = null;
      render();
    });
    es.addEventListener("status", (e) => {
      S.status = JSON.parse(e.data);
      renderStatus(); renderNodes(); renderParams(true); scheduleInspect();
    });
    es.addEventListener("error", () => setOnline(false));
  }
  function setOnline(on) {
    S.online = on;
    $("#offline").classList.toggle("hidden", on);
    if (!on) { const c = $("#cook-status"); c.className = "status"; c.querySelector(".text").textContent = "offline"; }
  }

  // ---------------------------------------------------------------- geometry of the drawing
  const nodeOf = (id) => S.graph.nodes[id];
  const typeOf = (id) => S.types[(nodeOf(id) || {}).type] || { inputs: [], params: [], category: "" };
  function pos(id) {
    if (S.dragPos.has(id)) return S.dragPos.get(id);
    const p = (nodeOf(id) || {}).pos;
    return Array.isArray(p) ? p : [0, 0];
  }
  function inputs(id) {                                       // [[port, [sources]]] in declaration order
    const n = nodeOf(id), t = typeOf(id);
    return t.inputs.map((spec) => {
      const ref = (n.inputs || {})[spec.name];
      return [spec, ref == null ? [] : Array.isArray(ref) ? ref : [ref]];
    });
  }
  function inPortCenter(id, portIndex, k = 0, m = 1) {
    const [x, y] = pos(id), n = typeOf(id).inputs.length;
    const spec = typeOf(id).inputs[portIndex];
    const off = spec && spec.multi && m > 1 ? (k - (m - 1) / 2) * Math.min(7, 26 / (m - 1)) : 0;
    return [x + (W * (portIndex + 1)) / (n + 1) + off, y];
  }
  const outPortCenter = (id) => { const [x, y] = pos(id); return [x + W / 2, y + H]; };
  function wirePath([x1, y1], [x2, y2]) {
    const dy = Math.max(40, Math.abs(y2 - y1) / 2) + (y2 < y1 ? 60 : 0);
    return `M${x1},${y1} C${x1},${y1 + dy} ${x2},${y2 - dy} ${x2},${y2}`;
  }
  const toWorld = (cx, cy) => {
    const r = canvas.getBoundingClientRect();
    return [(cx - r.left - S.view.x) / S.view.k, (cy - r.top - S.view.y) / S.view.k];
  };
  const toScreen = ([x, y]) => [x * S.view.k + S.view.x, y * S.view.k + S.view.y];
  function applyView() {
    const { x, y, k } = S.view;
    world.style.transform = `translate(${x}px, ${y}px) scale(${k})`;
    canvas.style.backgroundSize = `${24 * k}px ${24 * k}px`;
    canvas.style.backgroundPosition = `${x}px ${y}px`;
  }
  function frame(selectedOnly) {
    const ids = selectedOnly && S.sel.size ? [...S.sel] : Object.keys(S.graph.nodes);
    const r = canvas.getBoundingClientRect();
    if (!ids.length) { S.view = { x: r.width / 2 - W / 2, y: r.height / 3, k: 1 }; applyView(); return; }
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const id of ids) { const [x, y] = pos(id); x0 = Math.min(x0, x); y0 = Math.min(y0, y - 30); x1 = Math.max(x1, x + W); y1 = Math.max(y1, y + H + 10); }
    const pad = 60;
    const k = Math.max(0.25, Math.min(1.3, (r.width - 2 * pad) / (x1 - x0), (r.height - 2 * pad) / (y1 - y0)));
    S.view = { k, x: r.width / 2 - ((x0 + x1) / 2) * k, y: r.height / 2 - ((y0 + y1) / 2) * k };
    applyView();
  }

  // ---------------------------------------------------------------- rendering
  function render() {
    renderNodes(); renderTop(); renderStatus(); renderParams(); scheduleInspect();
    $("#empty-hint").classList.toggle("hidden", Object.keys(S.graph.nodes).length > 0);
  }
  function upstreamOfDisplay() {
    const seen = new Set(), d = displayNode();
    const stack = d ? [d] : [];
    while (stack.length) {
      const id = stack.pop();
      if (seen.has(id) || !nodeOf(id)) continue;
      seen.add(id);
      for (const [, srcs] of inputs(id)) stack.push(...srcs);
    }
    return seen;
  }
  function displayNode() {
    if (S.graph.display && nodeOf(S.graph.display)) return S.graph.display;
    return Object.keys(S.graph.nodes).find((id) => nodeOf(id).type === "output") || null;
  }
  function exposedOf(id) { return (S.graph.exposed || []).filter((e) => e.node === id); }

  function renderNodes() {
    const st = S.status || {}, errNode = st.ok === false && st.error ? st.error.node : null;
    const cook = st.cook || {}, nodeMs = cook.node_ms || {}, cached = new Set(cook.cached || []);
    const live = upstreamOfDisplay(), disp = displayNode();
    const keep = new Set();
    for (const [id, n] of Object.entries(S.graph.nodes)) {
      keep.add(id);
      const t = typeOf(id);
      let el = nodesEl.querySelector(`.node[data-id="${CSS.escape(id)}"]`);
      if (!el) { el = h("div", { class: "node", dataset: { id } }); nodesEl.append(el); }
      const [x, y] = pos(id);
      el.style.left = x + "px"; el.style.top = y + "px";
      el.className = ["node", "cat-" + (t.category || "other"), S.sel.has(id) && "selected", id === disp && "display",
        id === errNode && "error", !live.has(id) && "unused"].filter(Boolean).join(" ");
      const ex = exposedOf(id);
      // Rebuild a node's DOM only when its structure changes (wiring, comment, flags); status updates touch just the
      // status line. Stable elements keep hover state and never vanish under a click or a drag.
      const subSig = JSON.stringify([id === errNode && st.error.message, nodeMs[id], cached.has(id), n.type]);
      const structSig = JSON.stringify([n.type, n.inputs, n.comment, ex.map((e) => e.name), t.inputs.length, id === disp]);
      if (el._struct === structSig) {
        if (el._sub !== subSig) { el._sub = subSig; el.querySelector(".sub").replaceChildren(...subLine(id, n, st, errNode, nodeMs, cached)); }
        continue;
      }
      el._struct = structSig; el._sub = subSig;
      const kids = [h("div", { class: "band" }), h("div", { class: "title", title: id }, id),
        h("div", { class: "sub" }, subLine(id, n, st, errNode, nodeMs, cached)),
        h("div", { class: "flag", title: id === disp ? "Display flag (this is what Blender shows)" : "Set display flag" })];
      inputs(id).forEach(([spec, srcs], i) => {
        const cx = (W * (i + 1)) / (t.inputs.length + 1);
        const missing = !srcs.length && !spec.optional;
        kids.push(h("div", { class: ["port", "in", spec.multi && "multi", missing && "missing"].filter(Boolean).join(" "),
          dataset: { port: spec.name, node: id }, title: spec.name + (spec.multi ? " (any number)" : ""),
          style: { left: cx - (spec.multi ? 17 : 7) + "px" } }));
        kids.push(h("div", { class: "port-label", style: { left: cx + "px" } }, spec.name));
      });
      kids.push(h("div", { class: "port out", dataset: { node: id }, title: "output: drag to an input" }));
      if (n.comment) kids.push(h("div", { class: "comment" }, String(n.comment)));
      if (ex.length) kids.push(h("div", { class: "badge", title: "Exposed to Blender: " + ex.map((e) => e.name).join(", ") }, "B"));
      el.replaceChildren(...kids);
    }
    for (const el of [...nodesEl.children]) if (!keep.has(el.dataset.id)) el.remove();
    renderWires();
  }
  function subLine(id, n, st, errNode, nodeMs, cached) {
    if (id === errNode) return [h("span", null, "⚠ " + shortError(st.error.message))];
    if (nodeMs[id] != null) return [n.type, " · ", h("span", { class: "ms" }, nodeMs[id].toFixed(1) + " ms")];
    if (cached.has(id)) return [n.type, " · cached"];
    return [n.type];
  }
  function shortError(msg) { const m = String(msg || ""); return m.length > 60 ? m.slice(0, 57) + "…" : m; }
  function wireExists(w) {
    const n = nodeOf(w.to); if (!n) return false;
    const ref = (n.inputs || {})[w.port];
    return Array.isArray(ref) ? ref.includes(w.from) : ref === w.from;
  }
  function renderWires() {
    const kids = [];
    for (const id of Object.keys(S.graph.nodes)) {
      inputs(id).forEach(([spec, srcs], i) => {
        srcs.forEach((src, k) => {
          if (!nodeOf(src)) return;
          if (drag && drag.pickup && drag.pickup.to === id && drag.pickup.port === spec.name && drag.pickup.from === src) return;
          const d = wirePath(outPortCenter(src), inPortCenter(id, i, k, srcs.length));
          const sel = S.wireSel && S.wireSel.to === id && S.wireSel.port === spec.name && S.wireSel.from === src;
          kids.push(svgEl("path", { d, class: sel ? "sel" : null }));
          const hit = svgEl("path", { d, class: "hit" });
          hit.dataset.to = id; hit.dataset.port = spec.name; hit.dataset.from = src;
          kids.push(hit);
        });
      });
    }
    wiresSvg.replaceChildren(...kids);
  }
  function renderTop() {
    $("#file-name").textContent = S.path ? relPath(S.path) : "(unsaved graph)";
    $("#file-btn").title = S.path || "";
    $("#undo-btn").disabled = !S.canUndo;
    $("#redo-btn").disabled = !S.canRedo;
  }
  function relPath(p) {
    const root = (S.root || "").replace(/\\/g, "/").replace(/\/$/, ""), q = String(p).replace(/\\/g, "/");
    return root && q.startsWith(root + "/") ? q.slice(root.length + 1) : q;
  }
  function renderStatus() {
    const st = S.status;
    const c = $("#cook-status"), txt = c.querySelector(".text");
    if (!S.online || !st) return;
    if (st.ok === false) {
      c.className = "status err";
      txt.textContent = "error" + (st.error && st.error.node ? " in " + st.error.node : "");
    } else if (st.busy) {
      c.className = "status busy"; txt.textContent = "cooking…";
    } else if (st.ok) {
      c.className = "status ok";
      txt.textContent = `${fmt(st.points)} pts · ${fmt(st.prims)} prims` + (st.cook ? ` · ${st.cook.ms.toFixed(1)} ms` : "");
    } else { c.className = "status"; txt.textContent = "idle"; }
    const fi = $("#frame-input");
    if (st.frame !== undefined && document.activeElement !== fi && Number(fi.value) !== st.frame) fi.value = st.frame;
    if (st.frame !== undefined && document.activeElement !== $("#frame-slider")) syncSlider(st.frame);
    const clients = st.clients || [], blender = clients.filter((n) => /^blender/i.test(n));
    const b = $("#blender-status");
    b.className = "status" + (blender.length ? " ok" : "");
    b.querySelector(".text").textContent = blender.length ? "Blender" : "Blender —";
    b.title = clients.length ? "Connected: " + clients.join(", ") : "No Blender connected. Run tools/launch_blender.py, or Connect in Blender's MiniHou tab.";
    const bar = $("#errorbar");
    const err = st.ok === false && st.error;
    bar.classList.toggle("hidden", !err);
    document.body.classList.toggle("has-error", !!err);
    if (err) { bar.textContent = (err.node ? err.node + ": " : "") + err.message; bar.dataset.node = err.node || ""; }
  }

  // ---------------------------------------------------------------- frame (time)
  function syncSlider(f) {
    const sl = $("#frame-slider");
    if (f > Number(sl.max)) sl.max = Math.ceil(f / 250) * 250;           // the range grows to hold the frame
    sl.value = Math.max(Number(sl.min), f);
  }
  let framePending = null, frameSending = false;
  async function setFrame(f) {                                          // sends only the latest value while one is in flight
    if (!Number.isFinite(f)) return;
    $("#frame-input").value = f;
    syncSlider(f);
    framePending = f;
    if (frameSending) return;
    frameSending = true;
    while (framePending !== null) {
      const v = framePending; framePending = null;
      try { await api("/api/frame", { frame: v }); } catch (e) { /* offline: the status refresh will correct it */ }
    }
    frameSending = false;
  }
  $("#frame-slider").addEventListener("input", (e) => setFrame(Number(e.target.value)));
  $("#frame-input").addEventListener("change", (e) => setFrame(Number(e.target.value)));
  $("#frame-prev").onclick = () => setFrame(Number($("#frame-input").value) - 1);
  $("#frame-next").onclick = () => setFrame(Number($("#frame-input").value) + 1);

  // ---------------------------------------------------------------- parameter panel
  let panelKey = null, scrubbing = false;
  const effective = (id, p) => { const v = ((nodeOf(id) || {}).params || {})[p.name]; return v === undefined ? p.default : v; };
  const isSet = (id, p) => ((nodeOf(id) || {}).params || {})[p.name] !== undefined && JSON.stringify(effective(id, p)) !== JSON.stringify(p.default);
  const EXPOSABLE = new Set(["float", "int", "bool", "vec3"]);

  function renderParams(statusOnly) {
    const panel = $("#params");
    const ids = [...S.sel].filter((id) => nodeOf(id));
    document.body.classList.toggle("no-selection", ids.length === 0);
    const key = ids.length === 1 ? ids[0] + ":" + nodeOf(ids[0]).type : ids.length ? "multi:" + ids.join(",") : "graph";
    const busy = scrubbing || panel.contains(document.activeElement);
    if (key === panelKey && (busy || statusOnly)) { syncParams(ids[0]); return; }
    panelKey = key;
    if (ids.length === 1) panel.replaceChildren(...nodePanel(ids[0]));
    else if (ids.length > 1) panel.replaceChildren(...multiPanel(ids));
    else panel.replaceChildren(...graphPanel());
  }
  function nodeErrorText(id) {
    const st = S.status;
    return st && st.ok === false && st.error && st.error.node === id ? st.error.message : "";
  }
  function nodePanel(id) {
    const n = nodeOf(id), t = typeOf(id);
    const out = [];
    const nid = h("input", { class: "nid", value: id, title: "Node name (Enter to rename)", spellcheck: false,
      onkeydown: (e) => { if (e.key === "Enter") e.target.blur(); if (e.key === "Escape") { e.target.value = id; e.target.blur(); } },
      onchange: async (e) => {
        const to = e.target.value.trim();
        if (!to || to === id) return;
        const r = await edit([{ op: "rename", id, to }]);
        if (r) { S.sel = new Set([to]); panelKey = null; renderParams(); renderNodes(); } else e.target.value = id;
      } });
    out.push(h("div", { class: "ph" }, nid));
    out.push(h("div", { class: "ptype" }, n.type + (t.category ? " · " + t.category : "") + (t.source ? " · " + relPath(t.source) : "")));
    if (t.doc) out.push(h("div", { class: "pdoc" }, t.doc));
    out.push(h("div", { class: "perr", dataset: { role: "err" }, style: { display: nodeErrorText(id) ? "" : "none" } }, nodeErrorText(id)));
    const isDisp = displayNode() === id;
    out.push(h("div", { class: "pactions" },
      h("button", { disabled: isDisp, onclick: () => edit([{ op: "display", id }]), title: "Show this node's output in Blender" }, isDisp ? "Displayed" : "Display"),
      h("button", { onclick: () => duplicate([id]) }, "Duplicate"),
      h("button", { onclick: () => removeNodes([id]) }, "Delete"),
      t.category === "system" && t.source && h("button", { title: "Open this system's graph to edit it (saves reload it everywhere)",
        onclick: () => openFile(t.source) }, "Edit system")));
    const ins = inputs(id);
    if (ins.length) {
      out.push(h("div", { class: "section" }, "Inputs"));
      for (const [spec, srcs] of ins) {
        out.push(h("div", { class: "conn" }, spec.name + " ← ",
          srcs.length ? srcs.map((s, i) => [i ? ", " : "", h("b", { style: { cursor: "pointer" }, onclick: () => select([s]) }, s)])
            : h("span", { style: { color: spec.optional ? "" : "var(--err)" } }, spec.optional ? "(optional)" : "not connected")));
      }
    }
    if (t.params.length) out.push(h("div", { class: "section" }, "Parameters"));
    for (const p of t.params) out.push(paramRow(id, p));
    out.push(h("div", { class: "section" }, "Note"));
    out.push(h("div", { class: "pw" }, h("input", { type: "text", value: n.comment || "", placeholder: "comment shown above the node",
      onchange: (e) => edit([{ op: "meta", id, set: { comment: e.target.value.trim() || null } }]) })));
    return out;
  }
  function paramRow(id, p) {
    const v = effective(id, p);
    const label = h("span", { class: "pl" + (isSet(id, p) ? " changed" : ""), title: (p.doc ? p.doc + "\n" : "") + p.name + " (" + p.kind + ")" }, p.label || p.name);
    const row = h("div", { class: "prow " + p.kind + (p.ui === "code" ? " code" : ""), dataset: { param: p.name } });
    let widget;
    if (p.kind === "float" || p.kind === "int") {
      const [lo, hi] = rangeFor(p, v);
      const isInt = p.kind === "int";
      const num = h("input", { type: "number", step: isInt ? 1 : "any", value: v, dataset: { role: "num" } });
      const rng = h("input", { type: "range", min: lo, max: hi, step: isInt ? 1 : (hi - lo) / 1000, value: v, dataset: { role: "rng" } });
      rng.addEventListener("input", () => { const x = isInt ? Math.round(+rng.value) : round(+rng.value); num.value = x; queueSet(id, p.name, x); });
      num.addEventListener("change", () => { if (num.value === "") return; const x = isInt ? Math.round(+num.value) : +num.value; rng.value = x; queueSet(id, p.name, x); });
      widget = h("div", { class: "pw" }, rng, num);
      scrubber(label, () => +num.value, (x) => { x = isInt ? Math.round(x) : round(x); num.value = x; rng.value = x; queueSet(id, p.name, x); }, isInt ? 1 / 6 : (hi - lo) / 300, p);
    } else if (p.kind === "bool") {
      const cb = h("input", { type: "checkbox", checked: !!v, dataset: { role: "bool" }, onchange: (e) => queueSet(id, p.name, e.target.checked) });
      widget = h("div", { class: "pw" }, cb);
    } else if (p.kind === "vec3") {
      const fields = [0, 1, 2].map((i) => h("input", { type: "number", step: "any", value: v[i], title: "XYZ"[i], dataset: { role: "v" + i } }));
      const send = () => queueSet(id, p.name, fields.map((f) => +f.value || 0));
      fields.forEach((f) => f.addEventListener("change", send));
      widget = h("div", { class: "pw" }, h("div", { class: "vec" }, fields));
      const span = Math.max(1, ...v.map((c) => Math.abs(c)), ...p.default.map((c) => Math.abs(c)));
      fields.forEach((f, i) => scrubber(f, () => +f.value, (x) => { f.value = round(x); send(); }, span / 300, p, true));
    } else if (p.kind === "enum") {
      const sel = h("select", { dataset: { role: "enum" }, onchange: (e) => queueSet(id, p.name, e.target.value) },
        p.choices.map((c) => h("option", { value: c, selected: c === v }, c)));
      widget = h("div", { class: "pw" }, sel);
    } else if (p.ui === "code") {
      const ta = h("textarea", { value: v, spellcheck: false, rows: Math.min(14, Math.max(3, String(v).split("\n").length + 1)), dataset: { role: "code" } });
      const apply = () => { ta.classList.remove("dirty"); if (ta.value !== effective(id, p)) queueSet(id, p.name, ta.value); };
      ta.addEventListener("input", () => ta.classList.toggle("dirty", ta.value !== effective(id, p)));
      ta.addEventListener("keydown", (e) => {
        if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); apply(); }
        else if (e.key === "Tab") { e.preventDefault(); ta.setRangeText("    ", ta.selectionStart, ta.selectionEnd, "end"); ta.dispatchEvent(new Event("input")); }
        else if (e.key === "Escape") { ta.value = effective(id, p); ta.classList.remove("dirty"); ta.blur(); }
        e.stopPropagation();
      });
      ta.addEventListener("blur", apply);
      widget = h("div", { class: "pw" }, ta);
      row.append(label, widget, resetBtn(id, p), h("span"));
      row.append(h("div", { class: "hint", style: { gridColumn: "1 / -1" } }, "Ctrl+Enter or click away to apply · Tab indents · Esc reverts"));
      return row;
    } else {
      const inp = h("input", { type: "text", value: v, dataset: { role: "str" }, onchange: (e) => queueSet(id, p.name, e.target.value) });
      widget = h("div", { class: "pw" }, inp);
    }
    row.append(label, widget, EXPOSABLE.has(p.kind) ? exposeBtn(id, p) : h("span"), resetBtn(id, p));
    return row;
  }
  function rangeFor(p, v) {
    const span = Math.max(1, Math.abs(p.default) * 2, Math.abs(v) * 1.5);
    let lo = p.soft_min ?? p.min ?? (p.default >= 0 && v >= 0 ? 0 : -span);
    let hi = p.soft_max ?? p.max ?? span;
    if (p.kind === "int" && p.soft_max == null && p.max == null) hi = Math.max(10, Math.ceil(Math.abs(p.default) * 4), Math.ceil(v * 1.5));
    lo = Math.min(lo, v); hi = Math.max(hi, v);
    if (p.min != null) lo = Math.max(lo, p.min);
    if (p.max != null) hi = Math.min(hi, p.max);
    return [lo, hi > lo ? hi : lo + 1];
  }
  function exposeBtn(id, p) {
    const e = (S.graph.exposed || []).find((x) => x.node === id && x.param === p.name);
    return h("button", { class: "pbtn expose" + (e ? " on" : ""), dataset: { role: "expose" },
      title: e ? `Exposed to Blender as "${e.name}" (click to remove)` : "Expose to Blender as a modifier input",
      onclick: () => edit([e ? { op: "unexpose", name: e.name } : { op: "expose", id, param: p.name }]) }, "B");
  }
  function resetBtn(id, p) {
    return h("button", { class: "pbtn reset", title: "Reset to default (" + JSON.stringify(p.default) + ")", dataset: { role: "reset" },
      style: { visibility: isSet(id, p) ? "visible" : "hidden" }, onclick: () => edit([{ op: "set", id, params: { [p.name]: null } }]) }, "↺");
  }
  function scrubber(el, get, set, step, p, onField) {
    // Drag horizontally on a label (or a vector field) to scrub; Shift = fine, Ctrl = coarse.
    el.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      const x0 = e.clientX, v0 = get();
      let moved = false;
      const move = (ev) => {
        const dx = ev.clientX - x0;
        if (!moved && Math.abs(dx) < 3) return;
        if (!moved) { moved = true; scrubbing = true; if (onField) el.blur(); }
        const mul = ev.shiftKey ? 0.1 : ev.ctrlKey ? 10 : 1;
        let v = v0 + dx * step * mul;
        if (p.min != null) v = Math.max(p.min, v);
        if (p.max != null) v = Math.min(p.max, v);
        set(v);
      };
      const up = () => {
        window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up);
        if (moved) { scrubbing = false; setTimeout(() => renderParams(), 0); }
      };
      window.addEventListener("pointermove", move); window.addEventListener("pointerup", up);
    });
  }
  function syncParams(id) {
    // Update values in place (keeps focus and an in-progress drag) when the same node's panel is showing.
    const panel = $("#params");
    if (!id || !nodeOf(id)) return;
    const err = panel.querySelector('[data-role="err"]');
    if (err) { const m = nodeErrorText(id); err.textContent = m; err.style.display = m ? "" : "none"; }
    for (const p of typeOf(id).params) {
      const row = panel.querySelector(`.prow[data-param="${CSS.escape(p.name)}"]`);
      if (!row) continue;
      const v = effective(id, p);
      const set = (role, fn) => { const el = row.querySelector(`[data-role="${role}"]`); if (el && el !== document.activeElement) fn(el); };
      if (p.kind === "float" || p.kind === "int") { if (!scrubbing) { set("num", (el) => (el.value = v)); set("rng", (el) => (el.value = v)); } }
      else if (p.kind === "bool") set("bool", (el) => (el.checked = !!v));
      else if (p.kind === "vec3") { if (!scrubbing) [0, 1, 2].forEach((i) => set("v" + i, (el) => (el.value = v[i]))); }
      else if (p.kind === "enum") set("enum", (el) => (el.value = v));
      else if (p.ui === "code") set("code", (el) => { if (!el.classList.contains("dirty")) el.value = v; });
      else set("str", (el) => (el.value = v));
      const lab = row.querySelector(".pl"); if (lab) lab.classList.toggle("changed", isSet(id, p));
      const rb = row.querySelector('[data-role="reset"]'); if (rb) rb.style.visibility = isSet(id, p) ? "visible" : "hidden";
      const eb = row.querySelector('[data-role="expose"]');
      if (eb) eb.classList.toggle("on", !!(S.graph.exposed || []).find((x) => x.node === id && x.param === p.name));
    }
  }
  function multiPanel(ids) {
    return [h("div", { class: "ph" }, h("b", null, ids.length + " nodes selected")),
      h("div", { class: "pactions" }, h("button", { onclick: () => duplicate(ids) }, "Duplicate"), h("button", { onclick: () => removeNodes(ids) }, "Delete"))];
  }
  function graphPanel() {
    const ex = S.graph.exposed || [];
    const out = [h("div", { class: "ph" }, h("b", null, S.path ? relPath(S.path) : "Graph")),
      h("div", { class: "ptype" }, `${Object.keys(S.graph.nodes).length} nodes · display: ${displayNode() || "none"}`),
      h("div", { class: "section" }, "Blender modifier inputs")];
    if (!ex.length) out.push(h("div", { class: "hint" }, "None yet. Select a node and press B next to a parameter to expose it."));
    for (const e of ex) {
      out.push(h("div", { class: "prow" },
        h("span", { class: "pl", style: { cursor: "pointer" }, title: "Select " + e.node, onclick: () => select([e.node]) }, e.node + "." + e.param),
        h("div", { class: "pw" }, h("input", { type: "text", value: e.name, title: "Name in Blender",
          onchange: (ev) => { const name = ev.target.value.trim(); if (name && name !== e.name) edit([{ op: "expose", id: e.node, param: e.param, name }]); } })),
        h("button", { class: "pbtn", title: "Remove from Blender", onclick: () => edit([{ op: "unexpose", name: e.name }]) }, "✕"), h("span")));
    }
    const lib = S.library;
    if (lib) {
      out.push(h("div", { class: "section" }, "Node library"));
      if (!lib.files.length) out.push(h("div", { class: "hint" }, "No node files yet. Python files in " +
        lib.dirs.map(relPath).join(", ") + " become node types (python -m minihou new-node NAME writes a starter)."));
      for (const f of lib.files) {
        out.push(h("div", { class: "conn" }, h("b", null, relPath(f.file)), " ",
          f.error ? h("span", { style: { color: "var(--err)" } }, "⚠ " + f.error) : f.nodes.join(", ")));
      }
    }
    out.push(h("div", { class: "section" }, "Tips"));
    out.push(h("div", { class: "hint" }, "Tab adds a node. Drag from a node's bottom dot to another node to wire it. The blue bar is the display flag. Press ? for all controls."));
    return out;
  }

  // ---------------------------------------------------------------- geometry inspector
  let inspectTimer = null, inspectSeq = 0, inspectKey = null;
  function scheduleInspect() {
    clearTimeout(inspectTimer);
    inspectTimer = setTimeout(runInspect, 150);
  }
  async function runInspect() {
    const ids = [...S.sel].filter((id) => nodeOf(id));
    const node = ids.length === 1 ? ids[0] : displayNode();
    const detail = $("#info-detail").checked;
    $("#info-node").textContent = node ? "· " + node : "";
    if (!node) { $("#info-body").replaceChildren("—"); return; }
    const key = `${node}:${S.rev}:${S.status ? S.status.cooked_rev : ""}:${detail}`;
    if (key === inspectKey) return;
    inspectKey = key;
    const seq = ++inspectSeq;
    let r;
    try { r = await api("/api/inspect", { node, detail }); } catch (e) { return; }
    if (seq !== inspectSeq) return;
    $("#info-body").replaceChildren(...infoView(r));
  }
  function infoView(r) {
    if (!r.ok) return [h("div", { class: "perr" }, (r.error.node && r.error.node !== r.node ? r.error.node + ": " : "") + r.error.message)];
    const rows = [["Points", fmt(r.points)], ["Primitives", fmt(r.prims)], ["Vertices", fmt(r.vertices)]];
    if (r.verts_per_prim) rows.push(["Verts per prim", r.verts_per_prim[0] === r.verts_per_prim[1] ? r.verts_per_prim[0] : r.verts_per_prim.join("–")]);
    if (r.bounds) { rows.push(["Size", r.bounds.size.join(", ")]); rows.push(["Center", r.bounds.center.join(", ")]); }
    rows.push(["Cook", `${r.ms} ms` + (r.cooked.length ? ` (${r.cooked.length} cooked)` : " (cached)")]);
    const out = [h("table", { class: "kv" }, rows.map(([k, v]) => h("tr", null, h("td", null, k), h("td", null, String(v)))))];
    out.push(h("div", { class: "section" }, "Attributes"));
    out.push(h("table", { class: "kv attrs" }, r.attributes.map((a) => h("tr", null,
      h("td", null, a.name, h("span", { class: "muted" }, " " + a.domain)),
      h("td", null, a.domain === "detail" ? String(a.value) : `${a.type}${a.components > 1 ? "×" + a.components : ""}` +
        (a.min ? `  [${a.min.join(",")}] … [${a.max.join(",")}]` : ""))))));
    if (r.blender) {                     // blender_object: what the add-on read (native data, skips)
      const b = r.blender, rows = [];
      if (b.vertex_groups && b.vertex_groups.length) rows.push(["Vertex groups", b.vertex_groups.join(", ")]);
      if (b.skipped && b.skipped.length) rows.push(["Skipped", b.skipped.join("; ")]);
      if (b.read_ms !== undefined) rows.push(["Read in Blender", `${b.read_ms} ms`]);
      if (b.error) rows.push(["Error", b.error]);
      out.push(h("div", { class: "section" }, "From Blender"));
      out.push(h("table", { class: "kv" }, rows.map(([k, v]) => h("tr", null, h("td", null, k), h("td", null, String(v))))));
    }
    return out;
  }

  // ---------------------------------------------------------------- editing helpers
  function select(ids, add) {
    if (!add) S.sel.clear();
    for (const id of ids) S.sel.add(id);
    S.wireSel = null;
    renderNodes(); renderParams(); scheduleInspect();
  }
  function uniqueId(base, taken) {
    base = String(base).replace(/[^A-Za-z0-9_]/g, "_").replace(/^(\d)/, "n$1") || "node";
    let i = 1;
    while (S.graph.nodes[base + i] || (taken && taken.has(base + i))) i++;
    return base + i;
  }
  async function removeNodes(ids) {
    if (!ids.length) return;
    const r = await edit([{ op: "remove", id: ids }]);
    if (r) select([]);
  }
  async function duplicate(ids) {
    const map = {}, taken = new Set(), ops = [];
    for (const id of ids) { map[id] = uniqueId(id.replace(/\d+$/, "") + "_", taken); taken.add(map[id]); }
    for (const id of ids) {
      const n = nodeOf(id), [x, y] = pos(id);
      ops.push({ op: "add", id: map[id], type: n.type, params: n.params || {}, pos: [x + 40, y + 40], comment: n.comment || undefined });
    }
    for (const id of ids) {
      for (const [spec, srcs] of inputs(id)) for (const s of srcs) ops.push({ op: "connect", from: map[s] || s, to: map[id], port: spec.name });
    }
    const r = await edit(ops);
    if (r) select(Object.values(map));
  }
  async function deleteSelection() {
    if (S.wireSel) {
      const w = S.wireSel; S.wireSel = null;
      await edit([{ op: "disconnect", to: w.to, port: w.port, from: w.from }]);
    } else if (S.sel.size) removeNodes([...S.sel]);
  }
  async function tidy() {
    const r = await edit([{ op: "layout" }]);
    if (r) frame(false);
  }
  async function undo(redo) {
    try { const r = await api(redo ? "/api/redo" : "/api/undo", { origin: "editor" }); applySnapshot(r.graph); }
    catch (e) { toast(e.message); }
  }

  // ---------------------------------------------------------------- Tab menu
  let menuCtx = null, menuItems = [], menuActive = 0;
  function openMenu(cx, cy, ctx) {
    menuCtx = ctx || {};
    const m = $("#tabmenu");
    m.classList.remove("hidden");
    const w = 260, hgt = 360;
    m.style.left = Math.max(4, Math.min(cx, innerWidth - w - 4)) + "px";
    m.style.top = Math.max(4, Math.min(cy, innerHeight - hgt - 4)) + "px";
    const inp = $("#tab-search");
    inp.value = "";
    fillMenu("");
    inp.focus();
  }
  function closeMenu() { $("#tabmenu").classList.add("hidden"); menuCtx = null; canvas.focus({ preventScroll: true }); }
  function fillMenu(q) {
    q = q.trim().toLowerCase();
    let list = S.typeList;
    if (menuCtx && (menuCtx.from || menuCtx.fromSel)) list = list.filter((t) => t.inputs.length);   // must accept a wire
    if (q) {
      const score = (t) => (t.name.startsWith(q) ? 0 : t.name.includes(q) ? 1 : (t.category || "").includes(q) ? 2 : (t.doc || "").toLowerCase().includes(q) ? 3 : 9);
      list = list.map((t) => [score(t), t]).filter(([s]) => s < 9).sort((a, b) => a[0] - b[0] || a[1].name.localeCompare(b[1].name)).map(([, t]) => t);
    }
    menuItems = list; menuActive = 0;
    const kids = [];
    let cat = null;
    list.forEach((t, i) => {
      if (!q && t.category !== cat) { cat = t.category; kids.push(h("div", { class: "cat" }, cat || "other")); }
      kids.push(h("div", { class: "it" + (i === 0 ? " active" : ""), dataset: { i }, title: t.doc || "",
        onmousedown: (e) => { e.preventDefault(); pickMenu(i); } }, h("span", null, t.name), h("small", null, (t.doc || "").split("\n")[0])));
    });
    if (!list.length) kids.push(h("div", { class: "cat" }, "no match"));
    $("#tab-list").replaceChildren(...kids);
  }
  function moveMenu(d) {
    if (!menuItems.length) return;
    menuActive = (menuActive + d + menuItems.length) % menuItems.length;
    const items = [...$("#tab-list").querySelectorAll(".it")];
    items.forEach((el, i) => el.classList.toggle("active", i === menuActive));
    items[menuActive].scrollIntoView({ block: "nearest" });
  }
  async function pickMenu(i) {
    const t = menuItems[i], ctx = menuCtx || {};
    closeMenu();
    if (!t) return;
    const id = uniqueId(t.name);
    let [x, y] = ctx.world || toWorld(...S.mouse);
    x -= W / 2; y -= 12;
    const op = { op: "add", type: t.name, id, pos: [x, y] };
    const ops = [op];
    const first = t.inputs[0] && t.inputs[0].name;
    if (ctx.from && first) op.inputs = { [first]: ctx.from };
    else if (ctx.fromSel && first) {
      op.inputs = { [first]: ctx.fromSel };
      const [sx, sy] = pos(ctx.fromSel);
      op.pos = [sx, sy + 110];
      const below = Object.entries(S.graph.nodes).find(([nid, n]) => nid !== ctx.fromSel && (n.inputs || {}) && Object.values(n.inputs || {}).flat().includes(ctx.fromSel));
      if (below) op.pos = [sx + 190, sy + 110];
    }
    if (ctx.feeds) { op.pos = [x, y - 60]; ops.push({ op: "connect", from: id, to: ctx.feeds.to, port: ctx.feeds.port }); }
    if (t.name === "output" && !displayNode()) op.display = true;
    const r = await edit(ops);
    if (r) select([id]);
  }

  // ---------------------------------------------------------------- mouse
  let drag = null;
  function portUnder(x, y) {
    const el = document.elementFromPoint(x, y);
    return el ? { port: el.closest(".port"), node: el.closest(".node") } : {};
  }
  canvas.addEventListener("pointermove", (e) => { S.mouse = [e.clientX, e.clientY]; });
  canvas.addEventListener("pointerdown", (e) => {
    if (!$("#tabmenu").classList.contains("hidden")) closeMenu();
    canvas.focus({ preventScroll: true });
    const t = e.target;
    const port = t.closest(".port"), nodeEl = t.closest(".node");
    if (e.button === 1 || (e.button === 0 && !nodeEl && !t.classList.contains("hit") && !e.shiftKey)) {
      drag = { kind: "pan", x0: e.clientX, y0: e.clientY, vx: S.view.x, vy: S.view.y, moved: false };
      canvas.classList.add("panning");
    } else if (e.button !== 0) {
      return;
    } else if (port && port.classList.contains("out")) {
      drag = { kind: "wire", from: port.dataset.node };
    } else if (port) {
      const id = port.dataset.node, pname = port.dataset.port;
      const ref = (nodeOf(id).inputs || {})[pname];
      const srcs = ref == null ? [] : Array.isArray(ref) ? ref : [ref];
      if (srcs.length) { drag = { kind: "wire", from: srcs[srcs.length - 1], pickup: { to: id, port: pname, from: srcs[srcs.length - 1] } }; renderWires(); }
      else drag = { kind: "wire-rev", to: id, port: pname };
    } else if (t.classList.contains("flag")) {
      edit([{ op: "display", id: nodeEl.dataset.id }]);
      return;
    } else if (nodeEl) {
      const id = nodeEl.dataset.id;
      if (e.shiftKey || e.ctrlKey || e.metaKey) {
        if (S.sel.has(id)) S.sel.delete(id); else S.sel.add(id);
        S.wireSel = null; renderNodes(); renderParams(); scheduleInspect();
        return;
      }
      if (!S.sel.has(id)) select([id]);
      const [wx, wy] = toWorld(e.clientX, e.clientY);
      drag = { kind: "move", ids: [...S.sel], wx, wy, start: new Map([...S.sel].map((i) => [i, pos(i).slice()])), moved: false };
    } else if (t.classList.contains("hit")) {
      S.wireSel = { to: t.dataset.to, port: t.dataset.port, from: t.dataset.from };
      S.sel.clear(); renderNodes(); renderParams();
      return;
    } else if (e.shiftKey) {
      drag = { kind: "box", x0: e.clientX, y0: e.clientY };
    }
    if (drag) {
      e.preventDefault();
      canvas.classList.toggle("wiring", drag.kind === "wire" || drag.kind === "wire-rev");
      window.addEventListener("pointermove", onDrag); window.addEventListener("pointerup", onDrop);
    }
  });
  function onDrag(e) {
    S.mouse = [e.clientX, e.clientY];
    if (!drag) return;
    if (drag.kind === "pan") {
      const dx = e.clientX - drag.x0, dy = e.clientY - drag.y0;
      if (Math.abs(dx) + Math.abs(dy) > 2) drag.moved = true;
      S.view.x = drag.vx + dx; S.view.y = drag.vy + dy; applyView();
    } else if (drag.kind === "move") {
      const [wx, wy] = toWorld(e.clientX, e.clientY), dx = wx - drag.wx, dy = wy - drag.wy;
      if (Math.abs(dx) + Math.abs(dy) > 1) drag.moved = true;
      for (const id of drag.ids) { const [x, y] = drag.start.get(id); S.dragPos.set(id, [Math.round(x + dx), Math.round(y + dy)]); }
      for (const id of drag.ids) { const el = nodesEl.querySelector(`.node[data-id="${CSS.escape(id)}"]`); if (el) { const [x, y] = pos(id); el.style.left = x + "px"; el.style.top = y + "px"; } }
      renderWires();
    } else if (drag.kind === "wire" || drag.kind === "wire-rev") {
      const r = canvas.getBoundingClientRect();
      const m = [e.clientX - r.left, e.clientY - r.top];
      let d;
      if (drag.kind === "wire") d = wirePathScreen(toScreen(outPortCenter(drag.from)), m);
      else {
        const i = typeOf(drag.to).inputs.findIndex((s) => s.name === drag.port);
        d = wirePathScreen(m, toScreen(inPortCenter(drag.to, i)));
      }
      overlay.replaceChildren(svgEl("path", { d }));
      document.querySelectorAll(".port.target").forEach((el) => el.classList.remove("target"));
      const { port } = portUnder(e.clientX, e.clientY);
      if (port && (drag.kind === "wire") === port.classList.contains("in")) port.classList.add("target");
    } else if (drag.kind === "box") {
      const b = $("#boxsel"), r = canvas.getBoundingClientRect();
      const x = Math.min(drag.x0, e.clientX) - r.left, y = Math.min(drag.y0, e.clientY) - r.top;
      b.classList.remove("hidden");
      Object.assign(b.style, { left: x + "px", top: y + "px", width: Math.abs(e.clientX - drag.x0) + "px", height: Math.abs(e.clientY - drag.y0) + "px" });
    }
  }
  function wirePathScreen(a, b) {
    const k = S.view.k, dy = Math.max(40 * k, Math.abs(b[1] - a[1]) / 2);
    return `M${a[0]},${a[1]} C${a[0]},${a[1] + dy} ${b[0]},${b[1] - dy} ${b[0]},${b[1]}`;
  }
  async function onDrop(e) {
    window.removeEventListener("pointermove", onDrag); window.removeEventListener("pointerup", onDrop);
    const d = drag; drag = null;
    overlay.replaceChildren();
    canvas.classList.remove("panning", "wiring");
    document.querySelectorAll(".port.target").forEach((el) => el.classList.remove("target"));
    if (!d) return;
    if (d.kind === "pan") {
      if (!d.moved && e.button === 0) { S.sel.clear(); S.wireSel = null; renderNodes(); renderParams(); scheduleInspect(); }
    } else if (d.kind === "move") {
      if (d.moved) {
        const positions = {};
        for (const id of d.ids) positions[id] = pos(id);
        const r = await edit([{ op: "move", positions }]);
        S.dragPos.clear();
        if (!r) renderNodes();
      } else S.dragPos.clear();
    } else if (d.kind === "wire") {
      const { port, node } = portUnder(e.clientX, e.clientY);
      const ops = d.pickup ? [{ op: "disconnect", to: d.pickup.to, port: d.pickup.port, from: d.pickup.from }] : [];
      if (port && port.classList.contains("in") && port.dataset.node !== d.from) {
        if (d.pickup && d.pickup.to === port.dataset.node && d.pickup.port === port.dataset.port) { renderWires(); return; }
        ops.push({ op: "connect", from: d.from, to: port.dataset.node, port: port.dataset.port });
      } else if (node && node.dataset.id !== d.from && (!d.pickup || node.dataset.id !== d.pickup.to) && typeOf(node.dataset.id).inputs.length) {
        ops.push({ op: "connect", from: d.from, to: node.dataset.id });
      } else if (!node && !d.pickup) {
        openMenu(e.clientX, e.clientY, { from: d.from, world: toWorld(e.clientX, e.clientY + 50) });
        return;
      }
      if (ops.length) await edit(ops); else renderWires();
    } else if (d.kind === "wire-rev") {
      const { port, node } = portUnder(e.clientX, e.clientY);
      const src = port && port.classList.contains("out") ? port.dataset.node : node ? node.dataset.id : null;
      if (src && src !== d.to) await edit([{ op: "connect", from: src, to: d.to, port: d.port }]);
      else if (!node) openMenu(e.clientX, e.clientY, { feeds: { to: d.to, port: d.port }, world: toWorld(e.clientX, e.clientY) });
    } else if (d.kind === "box") {
      $("#boxsel").classList.add("hidden");
      const [ax, ay] = toWorld(Math.min(d.x0, e.clientX), Math.min(d.y0, e.clientY));
      const [bx, by] = toWorld(Math.max(d.x0, e.clientX), Math.max(d.y0, e.clientY));
      const hits = Object.keys(S.graph.nodes).filter((id) => { const [x, y] = pos(id); return x + W > ax && x < bx && y + H > ay && y < by; });
      select(hits, true);
    }
  }
  canvas.addEventListener("dblclick", (e) => { if (!e.target.closest(".node")) openMenu(e.clientX, e.clientY, { world: toWorld(e.clientX, e.clientY) }); });
  canvas.addEventListener("contextmenu", (e) => { e.preventDefault(); if (!e.target.closest(".node")) openMenu(e.clientX, e.clientY, { world: toWorld(e.clientX, e.clientY) }); });
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    const trackpadPan = !e.ctrlKey && e.deltaMode === 0 && Math.abs(e.deltaX) > 0 && Math.abs(e.deltaY) < 50;
    if (trackpadPan) { S.view.x -= e.deltaX; S.view.y -= e.deltaY; applyView(); return; }
    const r = canvas.getBoundingClientRect(), mx = e.clientX - r.left, my = e.clientY - r.top;
    const k = Math.max(0.15, Math.min(2.5, S.view.k * Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015))));
    S.view.x = mx - ((mx - S.view.x) * k) / S.view.k;
    S.view.y = my - ((my - S.view.y) * k) / S.view.k;
    S.view.k = k;
    applyView();
  }, { passive: false });

  // ---------------------------------------------------------------- keyboard
  document.addEventListener("keydown", (e) => {
    const menuOpen = !$("#tabmenu").classList.contains("hidden");
    if (menuOpen) {
      if (e.key === "Escape") { closeMenu(); e.preventDefault(); }
      else if (e.key === "ArrowDown") { moveMenu(1); e.preventDefault(); }
      else if (e.key === "ArrowUp") { moveMenu(-1); e.preventDefault(); }
      else if (e.key === "Enter") { pickMenu(menuActive); e.preventDefault(); }
      else if (e.key === "Tab") e.preventDefault();
      return;
    }
    const tag = (e.target.tagName || "").toLowerCase();
    if (tag === "input" || tag === "textarea" || tag === "select") return;
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === "Escape") { $("#help").classList.add("hidden"); $("#file-menu").classList.add("hidden"); }
    else if (e.key === "Tab") {
      e.preventDefault();
      const one = S.sel.size === 1 ? [...S.sel][0] : null;
      openMenu(S.mouse[0], S.mouse[1], one ? { fromSel: one } : { world: toWorld(...S.mouse) });
    } else if (mod && e.key.toLowerCase() === "z") { e.preventDefault(); undo(e.shiftKey); }
    else if (mod && e.key.toLowerCase() === "y") { e.preventDefault(); undo(true); }
    else if (mod && e.key.toLowerCase() === "d") { e.preventDefault(); if (S.sel.size) duplicate([...S.sel]); }
    else if (mod && e.key.toLowerCase() === "a") { e.preventDefault(); select(Object.keys(S.graph.nodes)); }
    else if (e.key === "Delete" || e.key === "Backspace") { e.preventDefault(); deleteSelection(); }
    else if (e.key === "f" || e.key === "F") frame(true);
    else if (e.key === "l" || e.key === "L") tidy();
    else if (e.key === "?") $("#help").classList.toggle("hidden");
  });
  $("#tab-search").addEventListener("input", (e) => fillMenu(e.target.value));
  $("#tab-search").addEventListener("blur", () => setTimeout(() => { if (document.activeElement !== $("#tab-search")) $("#tabmenu").classList.add("hidden"); }, 100));

  // ---------------------------------------------------------------- top bar, file menu, splitter
  $("#undo-btn").onclick = () => undo(false);
  $("#redo-btn").onclick = () => undo(true);
  $("#add-btn").onclick = (e) => { const r = canvas.getBoundingClientRect(); openMenu(e.clientX, e.clientY + 20, { world: toWorld(r.left + r.width / 2, r.top + r.height / 2) }); };
  $("#help-btn").onclick = () => $("#help").classList.toggle("hidden");
  $("#tidy-btn").onclick = () => tidy();
  $("#info-detail").onchange = () => { inspectKey = null; scheduleInspect(); };
  $("#errorbar").onclick = () => { const id = $("#errorbar").dataset.node; if (id && nodeOf(id)) { select([id]); frame(true); } };
  $("#file-btn").onclick = async (e) => {
    e.stopPropagation();
    const menu = $("#file-menu");
    if (!menu.classList.contains("hidden")) { menu.classList.add("hidden"); return; }
    let files = [];
    try { files = (await api("/api/files")).files; } catch (err) { toast(err.message); return; }
    const cur = S.path ? relPath(S.path) : null;
    menu.replaceChildren(
      ...files.map((f) => h("div", { class: "item" + (f === cur ? " cur" : ""), onclick: () => openFile(f) }, f)),
      h("div", { class: "sep" }),
      h("div", { class: "item", onclick: newFile }, "New graph…"));
    menu.classList.remove("hidden");
  };
  document.addEventListener("click", (e) => { if (!e.target.closest(".file")) $("#file-menu").classList.add("hidden"); });
  async function openFile(path, create) {
    $("#file-menu").classList.add("hidden");
    try { const snap = await api("/api/open", { path, create: !!create }); S.sel.clear(); applySnapshot(snap); }
    catch (e) { toast(e.message); }
  }
  function newFile() {
    let name = prompt("New graph file (inside " + (S.root || "the workspace") + "):", "graphs/my_system.json");
    if (!name) return;
    name = name.trim();
    if (!name.toLowerCase().endsWith(".json")) name += ".json";
    openFile(name, true);
  }
  (() => {
    const side = $("#side");
    try { const w = +localStorage.getItem("minihou.side"); if (w > 200) side.style.width = w + "px"; } catch (e) { /* storage unavailable */ }
    $("#splitter").addEventListener("pointerdown", (e) => {
      const x0 = e.clientX, w0 = side.getBoundingClientRect().width;
      const move = (ev) => { side.style.width = Math.max(260, Math.min(innerWidth * 0.7, w0 - (ev.clientX - x0))) + "px"; };
      const up = () => {
        window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up);
        try { localStorage.setItem("minihou.side", parseInt(side.style.width, 10)); } catch (err) { /* ignore */ }
      };
      window.addEventListener("pointermove", move); window.addEventListener("pointerup", up);
    });
  })();

  applyView();
  connectEvents();
  window.minihou = { S, edit, frame };                  // handy from the browser console
})();
