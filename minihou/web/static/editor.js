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
    graph: { nodes: {} }, rev: -1, path: null, documentId: null, canUndo: false, canRedo: false,
    types: {}, typeList: [], status: null, root: "",
    sel: new Set(), wireSel: null, view: { x: 80, y: 60, k: 1 },
    online: false, framed: false, dragPos: new Map(), mouse: [200, 200],
    trail: [], nav: null,                       // the path of nested graphs we opened ([{path, via}]) and what the next file switch means
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
  const recoveryKey = "minihou.unsent-edits";
  let recoveredEdits = [];
  try {
    const stored = JSON.parse(localStorage.getItem(recoveryKey) || "[]");
    if (Array.isArray(stored)) recoveredEdits = stored;
  } catch (e) { /* Keep an in-memory recovery list when storage is unavailable. */ }
  function stashEdits(ops, context) {
    recoveredEdits.push({ path: context.path, document_id: context.documentId, ops });
    let persisted = true;
    try {
      localStorage.setItem(recoveryKey, JSON.stringify(recoveredEdits));
    } catch (e) { persisted = false; }
    recoveryButton();
    toast(persisted ? "Graph switched before these edits were submitted. Download Unsent edits to recover them for the original graph."
      : "Unsent edits are kept in this tab only: browser storage is unavailable. Download Unsent edits before closing it.");
  }
  function recoveryButton() {
    if (!recoveredEdits.length || $("#unsent-edits")) return;
    $("#topbar").append(h("button", { id: "unsent-edits", title: "Download edits rejected after a graph switch, with their original graph paths",
      onclick: () => {
        const url = URL.createObjectURL(new Blob([JSON.stringify(recoveredEdits, null, 2)], { type: "application/json" }));
        const a = h("a", { href: url, download: "minihou-unsent-edits.json" });
        a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
      } }, "Unsent edits"));
  }
  async function edit(ops, context = { path: S.path, documentId: S.documentId }) {
    try {
      const r = await api("/api/edit", { ops, origin: "editor", document_id: context.documentId });
      applySnapshot(r.graph);
      return r;
    } catch (e) {
      if (context.documentId !== S.documentId || e.message.includes("Editor graph changed")) stashEdits(ops, context);
      else toast(e.message);
      return null;
    }
  }

  // Parameter changes are coalesced: one request in flight, the latest values win (smooth slider drags).
  const pendingSet = new Map();
  let pendingContext = null;
  let setInFlight = false;
  function queueSet(id, name, value) {
    if (!pendingContext) pendingContext = { path: S.path, documentId: S.documentId };
    const p = pendingSet.get(id) || {};
    p[name] = value;
    pendingSet.set(id, p);
    if (!setInFlight) flushSets();
  }
  async function flushSets() {
    if (!pendingSet.size) return;
    setInFlight = true;
    const ops = [...pendingSet].map(([id, params]) => ({ op: "set", id, params }));
    const context = pendingContext;
    pendingSet.clear();
    pendingContext = null;
    await edit(ops, context);
    setInFlight = false;
    if (pendingSet.size) flushSets();
  }

  // ---------------------------------------------------------------- state from the engine
  async function loadState() {
    try {
      const st = await api("/api/state");
      S.menuExtras = (st.menu && st.menu.extras) || []; S.menuGroups = (st.menu && st.menu.groups) || [];
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
    if (S.documentId !== null && snap.document_id !== S.documentId && pendingSet.size) {
      stashEdits([...pendingSet].map(([id, params]) => ({ op: "set", id, params })), pendingContext);
      pendingSet.clear(); pendingContext = null;
    }
    const fileChanged = snap.path !== S.path;
    if (fileChanged && snap.path) updateTrail(snap.path);                           // a snapshot without a file (transient) leaves the path alone
    S.rev = snap.rev; S.graph = snap.graph; S.path = snap.path; S.documentId = snap.document_id;
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
    renderBoxes(); renderNodes(); renderTop(); renderStatus(); renderParams(); scheduleInspect();
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
      el.className = ["node", "cat-" + (t.category || "other"), t.category === "system" && t.source && "group", S.sel.has(id) && "selected", id === disp && "display",
        id === errNode && "error", !live.has(id) && "unused"].filter(Boolean).join(" ");
      const ex = exposedOf(id);
      // Rebuild a node's DOM only when its structure changes (wiring, comment, flags); status updates touch just the
      // status line. Stable elements keep hover state and never vanish under a click or a drag.
      const subSig = JSON.stringify([id === errNode && st.error.message, nodeMs[id], cached.has(id), n.type]);
      const structSig = JSON.stringify([n.type, n.inputs, n.comment, ex.map((e) => e.name), t.inputs.length, id === disp, t.category === "system" && !!t.source]);
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
      if (t.category === "system" && t.source) kids.push(h("div", { class: "groupmark", title: "A group: double-click to open it" }, "▣"));
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
    for (const [end, n] of Object.entries(S.graph.nodes)) {
      if (n.type !== "loop_end" || nodeOf(n.inputs?.begin)?.type !== "loop_begin") continue;
      const points = loopRegion(end).map(pos);
      const x = Math.min(...points.map(p => p[0])) - 22, y = Math.min(...points.map(p => p[1])) - 32;
      const width = Math.max(...points.map(p => p[0])) + W + 22 - x;
      const height = Math.max(...points.map(p => p[1])) + H + 22 - y;
      kids.push(svgEl("rect", { x, y, width, height, rx: 12, class: "loop-region" }));
      const label = svgEl("text", { x: x + 8, y: y + 15, class: "loop-label" });
      label.textContent = `${n.inputs.begin} / ${end}: repeat ${n.params?.iterations ?? 3}`;
      kids.push(label);
    }
    for (const id of Object.keys(S.graph.nodes)) {
      inputs(id).forEach(([spec, srcs], i) => {
        srcs.forEach((src, k) => {
          if (!nodeOf(src)) return;
          if (drag && drag.pickup && drag.pickup.to === id && drag.pickup.port === spec.name && drag.pickup.from === src) return;
          const d = wirePath(outPortCenter(src), inPortCenter(id, i, k, srcs.length));
          const sel = S.wireSel && S.wireSel.to === id && S.wireSel.port === spec.name && S.wireSel.from === src;
          const pair = nodeOf(id).type === "loop_end" && spec.name === "begin";
          kids.push(svgEl("path", { d, class: [sel && "sel", pair && "loop-pair"].filter(Boolean).join(" ") }));
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
    renderCrumbs();
  }
  // ---- breadcrumbs: the chain of nested groups (systems) opened from the root graph; a crumb jumps back out
  const TRAIL_KEY = "minihou.trail";
  function updateTrail(path) {
    const nav = S.nav; S.nav = null;
    let trail = S.trail;
    if (!trail.length) { try { trail = JSON.parse(sessionStorage.getItem(TRAIL_KEY) || "[]"); } catch (e) { trail = []; } }
    if (!Array.isArray(trail)) trail = [];
    if (nav && nav.mode === "push") {
      const at = trail.findIndex((c) => c.path === path);                           // opening a graph already on the path just steps back to it
      trail = at >= 0 ? trail.slice(0, at + 1) : trail.concat([{ path, via: nav.via || "" }]);
    } else if (nav && nav.mode === "jump") {
      trail = trail.slice(0, nav.index + 1);
      if (!trail.length || trail[trail.length - 1].path !== path) trail = [{ path, via: "" }];
    } else if (!trail.length || trail[trail.length - 1].path !== path) {
      trail = [{ path, via: "" }];                                                  // a graph opened from the file menu (or elsewhere) starts a new path
    }
    S.trail = trail;
    try { sessionStorage.setItem(TRAIL_KEY, JSON.stringify(trail)); } catch (e) { /* storage unavailable */ }
  }
  const crumbName = (p) => relPath(p).split("/").pop().replace(/\.json$/i, "");
  function renderCrumbs() {
    const box = $("#crumbs");
    if (!S.path) { box.classList.add("hidden"); return; }
    const trail = S.trail.length && S.trail[S.trail.length - 1].path === S.path ? S.trail : [{ path: S.path, via: "" }];
    const kids = [];
    trail.forEach((c, i) => {
      if (i) kids.push(h("span", { class: "sep" }, "›"));
      const last = i === trail.length - 1;
      kids.push(h(last ? "span" : "button", { class: "crumb" + (last ? " cur" : ""), dataset: { index: String(i) },
        title: (c.via ? `opened from node ${c.via}\n` : "") + c.path + (last ? "" : "\nClick to go back to this graph"),
        onclick: last ? null : () => openFile(c.path, false, { mode: "jump", index: i }) }, crumbName(c.path)));
    });
    box.classList.remove("hidden");
    box.replaceChildren(...kids);
  }
  function goUp() {                                                                  // Alt+Up: back out of the current nested graph
    if (S.trail.length > 1) openFile(S.trail[S.trail.length - 2].path, false, { mode: "jump", index: S.trail.length - 2 });
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
    if (st.frame !== undefined && !PB.playing && PB.frame !== st.frame) { PB.frame = st.frame; tlRender(); }          // Blender's timeline moved it
    if (st.fps && !PB.playing && PB.fps !== st.fps && document.activeElement !== $("#tl-fps")) { PB.fps = st.fps; tlRender(); }
    const clients = st.clients || [], blender = clients.filter((n) => /^blender/i.test(n));
    const b = $("#blender-status");
    b.className = "status" + (blender.length ? " ok" : "");
    b.querySelector(".text").textContent = blender.length ? "Blender" : "Blender —";
    b.title = clients.length ? "Connected: " + clients.join(", ") : "No Blender connected. Run tools/launch_blender.py, or Connect in Blender's MiniHou tab.";
    updateBlenderBar(blender.length > 0);
    const bar = $("#errorbar");
    const err = st.ok === false && st.error;
    bar.classList.toggle("hidden", !err);
    document.body.classList.toggle("has-error", !!err);
    if (err) { bar.textContent = (err.node ? err.node + ": " : "") + err.message; bar.dataset.node = err.node || ""; }
  }

  // ---------------------------------------------------------------- Blender bar: install the add-on, open Blender connected
  const BAR_KEY = "minihou.blenderbar";                             // "hidden" once dismissed
  const BB = { info: null, connected: false, forced: false, loading: false };
  const storedHidden = () => { try { return localStorage.getItem(BAR_KEY) === "hidden"; } catch (e) { return false; } };
  async function loadBlenderInfo() {
    if (BB.loading || BB.info) return;
    BB.loading = true;
    try { BB.info = await (await fetch("/api/blender")).json(); } catch (e) { BB.info = null; }
    BB.loading = false;
    renderBlenderBar();
  }
  function updateBlenderBar(connected) {
    if (BB.connected !== connected) { BB.connected = connected; if (connected) BB.forced = false; }
    renderBlenderBar();
  }
  function renderBlenderBar() {
    const bar = $("#blender-bar");
    const show = BB.forced || (!BB.connected && !storedHidden() && S.online !== false);
    bar.classList.toggle("hidden", !show);
    document.body.classList.toggle("has-bar", show);
    if (!show) return;
    if (!BB.info) { loadBlenderInfo(); return; }
    const { blenders, addon } = BB.info, pick = $("#bb-pick");
    pick.classList.toggle("hidden", blenders.length < 2);
    const keep = pick.value;                                           // a re-render must not forget which Blender was chosen
    pick.replaceChildren(...blenders.map((b, i) => h("option", { value: String(i) }, `Blender ${b.version || "?"}${b.supported ? "" : " (too old)"}`)));
    if (keep && Number(keep) < blenders.length) pick.value = keep;
    $("#bb-install").disabled = !blenders.length || !addon.available;
    $("#bb-open").disabled = !blenders.length || !addon.available;
    $("#bb-zip").classList.toggle("hidden", !addon.available);
    $("#bb-text").textContent = BB.connected ? "Blender is connected." :
      blenders.length ? "Blender is not connected. Install the add-on once, then open Blender:" :
        "No Blender found here. Drag the zip onto a Blender window (4.2+), or use Preferences > Add-ons > Install from Disk:";
    if (!BB.msg) $("#bb-msg").textContent = "";
  }
  async function blenderAction(kind) {
    const idx = Number($("#bb-pick").value || 0), msg = $("#bb-msg");
    $("#bb-install").disabled = $("#bb-open").disabled = true;
    msg.textContent = kind === "install" ? "Installing… (Blender starts briefly in the background)" : "Starting Blender…";
    try {
      const r = await api("/api/blender/" + kind, { index: idx });
      msg.textContent = r.message || (r.ok ? "Done." : "Failed.");
      msg.classList.toggle("err", !r.ok);
    } catch (e) { msg.textContent = e.message; }
    BB.msg = true;
    renderBlenderBar();
  }
  $("#bb-install").onclick = () => blenderAction("install");
  $("#bb-open").onclick = () => blenderAction("launch");
  $("#bb-close").onclick = () => { BB.forced = false; try { localStorage.setItem(BAR_KEY, "hidden"); } catch (e) { /* ignore */ } renderBlenderBar(); };
  $("#blender-status").onclick = () => {
    try { localStorage.removeItem(BAR_KEY); } catch (e) { /* ignore */ }
    BB.forced = true; BB.msg = false; renderBlenderBar();
  };
  $("#bb-zip").addEventListener("dragstart", (e) => {                 // Chrome/Edge turn this into a file when dropped on another app
    const url = new URL($("#bb-zip").getAttribute("href"), location.href).href;
    e.dataTransfer.setData("DownloadURL", `application/zip:minihou_link.zip:${url}`);
    e.dataTransfer.setData("text/uri-list", url);
    e.dataTransfer.effectAllowed = "copy";
  });

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
    if (PB.frame !== f) { PB.frame = f; tlRender(); }
    framePending = f;
    if (frameSending) return;
    frameSending = true;
    while (framePending !== null) {
      const v = framePending; framePending = null;
      try { await api("/api/frame", { frame: v }); } catch (e) { /* offline: the status refresh will correct it */ }
    }
    frameSending = false;
  }
  // ---- timeline under the 3-D view: play / pause in real time, step, range, fps, loop
  const TL_KEY = "minihou.timeline";
  const PB = { playing: false, start: 1, end: 96, fps: 24, loop: true, frame: 1, t0: 0, f0: 1, raf: 0 };
  try { Object.assign(PB, JSON.parse(localStorage.getItem(TL_KEY) || "{}"), { playing: false, raf: 0 }); } catch (e) { /* storage unavailable */ }
  const tlSave = () => { try { localStorage.setItem(TL_KEY, JSON.stringify({ start: PB.start, end: PB.end, fps: PB.fps, loop: PB.loop })); } catch (e) { /* ignore */ } };
  function tlRender() {
    const sl = $("#tl-slider");
    sl.min = PB.start; sl.max = Math.max(PB.end, PB.start + 1);
    if (document.activeElement !== sl) sl.value = PB.frame;
    for (const [id, v] of [["#tl-start", PB.start], ["#tl-end", PB.end], ["#tl-fps", PB.fps]]) if (document.activeElement !== $(id)) $(id).value = v;
    if (document.activeElement !== $("#tl-frame")) $("#tl-frame").value = PB.frame;
    const play = $("#tl-play");
    play.textContent = PB.playing ? "⏸" : "▶"; play.classList.toggle("playing", PB.playing);
    play.title = PB.playing ? "Pause (Space over the view)" : "Play (Space over the view)";
    $("#tl-loop").classList.toggle("on", PB.loop);
  }
  function tlGoto(f) { PB.frame = f; tlRender(); setFrame(f); }
  function tlStop() { PB.playing = false; cancelAnimationFrame(PB.raf); PB.raf = 0; tlRender(); scheduleInspect(); }
  function tlTick(now) {
    if (!PB.playing) return;
    PB.raf = requestAnimationFrame(tlTick);
    const span = PB.end - PB.start + 1;
    let target = PB.f0 + Math.floor(((now - PB.t0) / 1000) * PB.fps);          // real time, not frame by frame: a slow cook skips frames
    if (target > PB.end) {
      if (!PB.loop) { tlGoto(PB.end); tlStop(); return; }
      target = PB.start + ((target - PB.start) % span);
      if (target < PB.frame) { PB.t0 = now; PB.f0 = target; }                     // wrapped: restart the clock from the new position
    }
    if (target !== PB.frame && !frameSending) tlGoto(target);
  }
  function tlPlay() {
    if (PB.playing) return;
    if (PB.frame >= PB.end || PB.frame < PB.start) PB.frame = PB.start;
    PB.playing = true; PB.t0 = performance.now(); PB.f0 = PB.frame;
    tlRender(); tlGoto(PB.frame);
    PB.raf = requestAnimationFrame(tlTick);
  }
  const tlToggle = () => (PB.playing ? tlStop() : tlPlay());
  const tlStep = (d) => { tlStop(); tlGoto(Math.min(PB.end, Math.max(PB.start, PB.frame + d))); };
  $("#tl-play").onclick = tlToggle;
  $("#tl-first").onclick = () => { tlStop(); tlGoto(PB.start); };
  $("#tl-last").onclick = () => { tlStop(); tlGoto(PB.end); };
  $("#tl-back").onclick = () => tlStep(-1);
  $("#tl-next").onclick = () => tlStep(1);
  $("#tl-loop").onclick = () => { PB.loop = !PB.loop; tlSave(); tlRender(); };
  $("#tl-slider").addEventListener("input", (e) => { if (PB.playing) { PB.t0 = performance.now(); PB.f0 = Number(e.target.value); } tlGoto(Number(e.target.value)); });
  $("#tl-frame").addEventListener("change", (e) => { const v = Math.round(Number(e.target.value)); if (Number.isFinite(v)) { tlStop(); tlGoto(v); } });
  for (const [id, key, lo] of [["#tl-start", "start", -1e6], ["#tl-end", "end", -1e6]]) {
    $(id).addEventListener("change", (e) => {
      const v = Math.round(Number(e.target.value));
      if (Number.isFinite(v)) { PB[key] = v; if (PB.end <= PB.start) PB.end = PB.start + 1; tlSave(); tlRender(); }
    });
  }
  $("#tl-fps").addEventListener("change", async (e) => {
    const v = Number(e.target.value);
    if (!(v >= 1 && v <= 240)) { tlRender(); return; }
    PB.fps = v; tlSave(); PB.t0 = performance.now(); PB.f0 = PB.frame; tlRender();
    try { await api("/api/frame", { frame: PB.frame, fps: v }); } catch (err) { /* offline */ }              // @Time = frame / fps in the engine too
  });
  tlRender();
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
    if (n.type === "loop_begin" || n.type === "loop_end") {
      const end = n.type === "loop_end" ? id : Object.keys(S.graph.nodes).find(k => nodeOf(k).type === "loop_end" && nodeOf(k).inputs?.begin === id);
      const begin = end && nodeOf(end).inputs?.begin;
      out.push(h("div", { class: "loop-info" }, end && nodeOf(begin)?.type === "loop_begin"
        ? [h("div", null, `Pair: ${begin} / ${end}`), h("button", { onclick: () => select(loopRegion(end)) }, "Select loop block"),
           h("button", { onclick: () => { select([end]); edit([{ op: "display", id: end }]); } }, "Display Loop End")]
        : "Unpaired loop: connect Begin to End's begin port and the body to geo."));
    }
    out.push(h("div", { class: "perr", dataset: { role: "err" }, style: { display: nodeErrorText(id) ? "" : "none" } }, nodeErrorText(id)));
    const isDisp = displayNode() === id;
    out.push(h("div", { class: "pactions" },
      h("button", { disabled: isDisp, onclick: () => edit([{ op: "display", id }]), title: "Show this node's output in Blender" }, isDisp ? "Displayed" : "Display"),
      h("button", { onclick: () => duplicate([id]) }, "Duplicate"),
      h("button", { onclick: () => removeNodes([id]) }, "Delete"),
      t.category === "system" && t.source && h("button", { title: "Open this system's graph to edit it (saves reload it everywhere)",
        onclick: () => openFile(t.source, false, { mode: "push", via: id }) }, "Edit system")));
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
    if (n.type === "rbd_solver") out.push(h("button", {
      onclick: () => queueSet(id, "reset", Number(n.params?.reset || 0) + 1),
      title: "Discard this simulation and replay from its initial geometry"
    }, "Reset simulation"));
    let ogHeader = false;
    for (let i = 0; i < t.params.length; i++) {
      const p = t.params[i];
      if (p.ui === "outgroup") {                                     // Output Groups: a checkbox and the group's name side by side, like Houdini
        if (!ogHeader) { out.push(h("div", { class: "ogheader" }, "Output Groups")); ogHeader = true; }
        const q = t.params[i + 1], named = q && q.ui === "outgroup_name";
        const pair = h("div", { class: "ogpair" + (effective(id, p) ? "" : " off") }, paramRow(id, p), named ? paramRow(id, q) : null);
        pair.addEventListener("change", (e) => { if (e.target.type === "checkbox") pair.classList.toggle("off", !e.target.checked); });
        out.push(pair);
        if (named) i++;
        continue;
      }
      out.push(paramRow(id, p));
    }
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
      if (p.ui === "outgroup") { const pair = row.closest(".ogpair"); if (pair) pair.classList.toggle("off", !v); }
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
    if (!PB.playing) inspectTimer = setTimeout(runInspect, 150);               // no inspector churn while playing
    schedulePreview();
  }

  // ---------------------------------------------------------------- 3-D preview of the selected (or display) node
  const pvMsg = (text) => { const m = $("#pv-msg"); m.textContent = text || ""; m.classList.toggle("hidden", !text); };
  const pv = window.MiniHouPreview ? window.MiniHouPreview.create($("#pv-canvas"), pvMsg) : null;
  let pvTimer = null, pvSeq = 0, pvKey = null, pvNode = null, pvPath = null, pvOver = false;
  function schedulePreview() {
    if (!pv || !pv.ok) return;
    clearTimeout(pvTimer);
    pvTimer = setTimeout(runPreview, PB.playing ? 0 : 200);
  }
  async function runPreview() {
    const ids = [...S.sel].filter((id) => nodeOf(id));
    const node = ids.length === 1 ? ids[0] : displayNode();
    $("#pv-node").textContent = node ? "· " + node : "";
    if (!node) { pv.clear(); pvMsg(""); pvKey = null; return; }
    const key = `${node}:${S.rev}:${S.status ? S.status.cooked_rev : ""}`;
    if (key === pvKey) return;
    pvKey = key;
    const seq = ++pvSeq;
    let buf;
    try {
      const r = await fetch("/api/preview", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ node }) });
      buf = await r.arrayBuffer();
    } catch (e) { pvKey = null; return; }
    if (seq !== pvSeq) return;
    const { head, arrays } = window.MiniHouPreview.parse(buf);
    if (!arrays) {
      pv.clear();
      pvMsg(head.error ? (head.error.node && head.error.node !== node ? head.error.node + ": " : "") + head.error.message : "No preview");
      return;
    }
    const empty = !head.points;
    pvMsg(empty ? "No geometry" : head.truncated ? "Showing part of a very large geometry." : "");
    pv.set(head, arrays, pvNode === node && pvPath === S.path);
    pv.setHandles(head.handles || []);
    refreshSelectionHandles();                                                 // a kept selection moved with the geometry: so does its gizmo
    updateToolButtons();
    pvNode = node; pvPath = S.path;
  }

  // ---- selecting points, edges and faces in the 3-D view and turning the selection into nodes (inserted after the viewed node)
  const rangePattern = (nums) => {                                  // [0,1,2,5,7,8] -> "0-2 5 7-8"
    const out = [];
    for (let i = 0; i < nums.length; i++) {
      let j = i;
      while (j + 1 < nums.length && nums[j + 1] === nums[j] + 1) j++;
      out.push(j > i ? `${nums[i]}-${nums[j]}` : String(nums[i]));
      i = j;
    }
    return out.join(" ");
  };
  const edgePattern = (edges) => edges.map(([a, b]) => `${a}-${b}`).join(" ");
  const SELECTION_ACTIONS = {
    edge: [{ label: "Bevel", key: "b", type: "polybevel", title: "Bevel the selected edges (a loop, or edges that meet or reach a border)",
      params: (s) => ({ edges: edgePattern(s.edges) }) }],
    face: [
      { label: "Extrude", key: "e", type: "polyextrude", params: (s) => ({ group: rangePattern(s.faces), distance: 0.3 }) },
      { label: "Inset", type: "polyextrude", params: (s) => ({ group: rangePattern(s.faces), distance: 0, inset: 0.15 }) },
      { label: "Reverse", type: "reverse", params: (s) => ({ group: rangePattern(s.faces) }) },
      { label: "Group", type: "group_create", params: (s) => ({ name: "faces", domain: "primitives", method: "pattern", pattern: rangePattern(s.faces) }) },
      { label: "Delete", key: "x", type: "blast", params: (s) => ({ group: rangePattern(s.faces), domain: "primitives" }) }],
    point: [
      { label: "Group", type: "group_create", params: (s) => ({ name: "points", domain: "points", method: "pattern", pattern: rangePattern(s.points) }) },
      { label: "Delete", key: "x", type: "blast", params: (s) => ({ group: rangePattern(s.points), domain: "points" }) }],
  };
  const SEL_KEY = { point: "points", edge: "edges", face: "faces", piece: "faces" };
  SELECTION_ACTIONS.piece = SELECTION_ACTIONS.face;                          // a piece is a set of faces: the same actions
  let pvSelection = { mode: "view", points: [], edges: [], faces: [] };
  async function applySelectionAction(a) {
    const s = pvSelection, node = pvNode;
    if (!node || !nodeOf(node)) { toast("Nothing to attach the new node to."); return; }
    const params = a.params(s);
    if ((params.group || params.pattern || params.edges || "").length > 30000) { toast("That selection is too large to write into a node; select fewer elements."); return; }
    const r = await edit([{ op: "insert", after: node, type: a.type, params }]);
    if (r) { pv.clearSelection(); select([r.results[0].id]); }
  }
  function renderSelectionTools(s) {
    pvSelection = s;
    const n = (s[SEL_KEY[s.mode]] || []).length;
    $("#pv-selinfo").textContent = s.mode === "view" ? "" : s.mode === "piece" ? `${n} face${n === 1 ? "" : "s"} in the selected pieces` : `${n} ${SEL_KEY[s.mode].replace(/s$/, "")}${n === 1 ? "" : "s"} selected`;
    document.querySelectorAll("#preview [data-mode]").forEach((b) => b.classList.toggle("on", b.dataset.mode === s.mode));
    const box = $("#pv-actions");
    const acts = n ? SELECTION_ACTIONS[s.mode] || [] : [];
    box.classList.toggle("hidden", !acts.length);
    box.replaceChildren(...acts.map((a) => h("button", { title: a.title || `${a.label} the selection: adds a ${a.type} node after ${pvNode || "the viewed node"}`,
      dataset: { action: a.label }, onclick: () => applySelectionAction(a) }, a.label + (a.key ? ` (${a.key.toUpperCase()})` : ""))));
    refreshSelectionHandles();
  }
  // ---- a selection carries a move / rotate / scale gizmo; the first drag adds a transform node for it (group = the selected points)
  const AXV = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
  function selectionGizmo(info) {
    const c = info.center, hs = [], reach = Math.max(info.radius * 1.15, 0.05);
    for (let i = 0; i < 3; i++) {
      const a = "xyz"[i];
      hs.push({ id: `translate_${a}`, label: `move ${a}`, param: "translate", comp: i, origin: c, dir: AXV[i], scale: 1, kind: "arrow", tool: "move", pending: true });
      hs.push({ id: `rotate_${a}`, label: `rotate ${a}`, param: "rotate", comp: i, origin: c, dir: AXV[i], scale: 180 / Math.PI, kind: "ring", radius: reach, tool: "rotate", pending: true });
      hs.push({ id: `scale_${a}`, label: `scale ${a}`, param: "scale", comp: i, origin: c, dir: AXV[i], scale: 1, kind: "scale", tool: "scale", pending: true });
    }
    hs.push({ id: "uniform_scale", label: "uniform scale", param: "uniform_scale", comp: null, origin: c, dir: AXV[2], scale: 1, kind: "uniform", tool: "scale", pending: true });
    return hs;
  }
  function refreshSelectionHandles() {
    if (!pv || !pv.ok) return;
    const info = pvSelection.mode === "view" ? null : pv.selectionInfo();
    pv.setSelectionHandles(info ? selectionGizmo(info) : []);
    updateToolButtons();
  }
  function updateToolButtons() {
    const has = pv.hasToolHandles();
    $("#pv-xform").classList.toggle("hidden", !has);
    document.querySelectorAll("#pv-xform [data-tool]").forEach((b) => b.classList.toggle("on", b.dataset.tool === pv.tool));
  }
  function setTool(t) { pv.setTool(t); updateToolButtons(); }
  function toggleOrtho() {
    const on = pv.toggleOrtho();
    $("#pv-ortho").textContent = on ? "Ortho" : "Persp";
    $("#pv-ortho").classList.toggle("on", on);
  }
  // ---- the "C" pie menu (pie.js): a tool inserts its node after the viewed node, filled in from the 3-D selection
  let pieFromView = false;
  function pieSelectionParams(item) {
    const s = pvSelection;
    if (!item.sel || !pv || !pv.ok || s.mode === "view" || !(s[SEL_KEY[s.mode]] || []).length) return {};
    const mode = s.mode === "piece" ? "face" : s.mode;
    let spec = item.sel[mode];
    if (!spec) return {};
    spec = typeof spec === "string" ? { [spec]: "$" } : { ...spec };
    const from = spec.from || { face: "faces", point: "points", edge: "edges" }[mode];
    delete spec.from;
    let pattern;
    if (from === "points") { const info = pv.selectionInfo(); pattern = info ? rangePattern(info.points) : ""; }
    else if (from === "edges") pattern = edgePattern(s.edges);
    else pattern = rangePattern(from === "faces" ? s.faces : s.points);
    if (!pattern || pattern.length > 30000) return {};
    for (const k of Object.keys(spec)) if (spec[k] === "$") spec[k] = pattern;
    return spec;
  }
  async function pieTool(item, note) {
    if (!item) { if (note) toast(note); return; }
    const t = S.types[item.type];
    if (!t) { toast(`This engine has no ${item.type} node.`); return; }
    const params = { ...(item.params || {}), ...pieSelectionParams(item) };
    if (item.center && pv && pv.ok && pvSelection.mode !== "view" && (pvSelection[SEL_KEY[pvSelection.mode]] || []).length) {
      const info = pv.selectionInfo();
      if (info) params[item.center] = info.center.map((v) => Math.round(v * 1e5) / 1e5);
    }
    const one = S.sel.size === 1 ? [...S.sel][0] : null;
    const target = pieFromView && pvNode && nodeOf(pvNode) ? pvNode : one && nodeOf(one) ? one : pvNode && nodeOf(pvNode) ? pvNode : displayNode();
    if (item.source || !t.inputs.length || !target) {
      const id = uniqueId(item.type);
      let [x, y] = pieFromView || !S.mouse ? toWorld(canvas.getBoundingClientRect().left + canvas.clientWidth / 2, canvas.getBoundingClientRect().top + canvas.clientHeight / 2) : toWorld(...S.mouse);
      const r = await edit([{ op: "add", type: item.type, id, params, pos: [x - W / 2, y - 12] }]);
      if (r) select([id]);
      return;
    }
    const r = await edit([{ op: "insert", after: target, type: item.type, params }]);
    if (r) { if (pv && pv.ok) pv.clearSelection(); select([r.results[0].id]); }
  }
  if (window.MiniPie) {
    MiniPie.init({ types: () => S.types, onPick: pieTool, onOpen: () => { pieFromView = pvOver; },
      blocked: () => !$("#tabmenu").classList.contains("hidden") || !$("#help").classList.contains("hidden") });
  }
  function wireSelectionTools() {
    pv.onSelect = renderSelectionTools;
    // viewport handles: arrows in the view that scrub a parameter of the viewed node
    let handleBase = null, gizmoNode = null;
    const baseFor = (node, h) => {                                                       // the parameter's current value (or default) on that node
      const n = nodeOf(node), spec = ((S.types[(n || {}).type] || {}).params || []).find((p) => p.name === h.param);
      const cur = n && n.params && n.params[h.param] !== undefined ? n.params[h.param] : spec ? spec.default : undefined;
      handleBase = Array.isArray(cur) ? cur.slice() : null;               // the other components of a vector stay as they were
      return h.comp === null || h.comp === undefined ? Number(cur) : Number(Array.isArray(cur) ? cur[h.comp] : NaN);
    };
    async function startSelectionDrag(h) {                                // the first drag on a selection's gizmo creates the transform node
      const info = pv.selectionInfo();
      if (!info || !pvNode || !nodeOf(pvNode)) return NaN;
      const pattern = rangePattern(info.points);
      if (pattern.length > 30000) { toast("That selection is too large to write into a node; select fewer elements."); return NaN; }
      const cur = nodeOf(pvNode);
      if (cur.type === "transform" && (cur.params || {}).group === pattern) gizmoNode = pvNode;     // the same selection again: edit the node already there
      else {
        const pivot = info.center.map((v) => Math.round(v * 1e5) / 1e5);
        const r = await edit([{ op: "insert", after: pvNode, type: "transform", params: { group: pattern, pivot } }]);
        if (!r) return NaN;
        gizmoNode = r.results[0].id;
        select([gizmoNode]);
      }
      return baseFor(gizmoNode, h);
    }
    pv.onHandleStart = (h) => (h.pending ? startSelectionDrag(h) : baseFor(pvNode, h));
    pv.onHandle = (h, value) => {
      const node = h.pending ? gizmoNode : pvNode;
      if (!node || !nodeOf(node)) return;
      const spec = ((S.types[nodeOf(node).type] || {}).params || []).find((p) => p.name === h.param);
      if (spec && spec.kind === "int") value = Math.round(value);                       // an integer parameter takes whole steps
      if (h.comp === null || h.comp === undefined) queueSet(node, h.param, value);
      else if (handleBase) { const v = handleBase.slice(); v[h.comp] = value; queueSet(node, h.param, v); }
    };
    document.querySelectorAll("#preview [data-mode]").forEach((b) => { b.onclick = () => pv.setMode(b.dataset.mode); });
    const wrap = $("#pv-canvas").parentElement;                         // the canvas and the overlays on it (gizmo, handles) count as one
    wrap.addEventListener("pointerenter", () => { pvOver = true; });
    wrap.addEventListener("pointerleave", () => { pvOver = false; });
    document.addEventListener("keydown", (e) => {
      const tag = (e.target.tagName || "").toLowerCase();
      if (!pvOver || tag === "input" || tag === "textarea" || tag === "select") return;
      const nav = { Numpad1: ["front"], Numpad3: ["right"], Numpad7: ["top"] }[e.code];          // Blender-style view keys; Ctrl = the opposite side
      if (nav && !e.altKey && !e.metaKey) { pv.viewAxis(nav[0], e.ctrlKey); e.preventDefault(); e.stopImmediatePropagation(); return; }
      if (e.code === "Numpad5" || e.code === "NumpadDecimal" || e.key === "Home") {
        if (e.code === "Numpad5") toggleOrtho(); else pv.frameSelection();
        e.preventDefault(); e.stopImmediatePropagation(); return;
      }
      if (e.ctrlKey || e.metaKey || e.altKey) return;
      const k = e.key.toLowerCase();
      if (k === " ") { tlToggle(); e.preventDefault(); e.stopImmediatePropagation(); return; }                        // timeline, with the pointer over the view
      if (k === "arrowright" || k === "arrowleft") {
        if (e.shiftKey) { tlStop(); tlGoto(k === "arrowright" ? PB.end : PB.start); } else tlStep(k === "arrowright" ? 1 : -1);
        e.preventDefault(); e.stopImmediatePropagation(); return;
      }
      const mode = { "1": "point", "2": "edge", "3": "face", "4": "piece" }[k];
      if (mode) { pv.setMode(mode); e.preventDefault(); e.stopImmediatePropagation(); return; }
      if (k === "escape") {
        if (pvSelection.mode !== "view" && !(pvSelection[SEL_KEY[pvSelection.mode]] || []).length) pv.setMode("view"); else pv.clearSelection();
        e.preventDefault(); e.stopImmediatePropagation(); return;
      }
      const act = (SELECTION_ACTIONS[pvSelection.mode] || []).find((a) => a.key === k);
      if (act && (pvSelection[SEL_KEY[pvSelection.mode]] || []).length) { applySelectionAction(act); e.preventDefault(); e.stopImmediatePropagation(); }
      else if ((k === "w" || k === "e" || k === "r") && pv.hasToolHandles()) { setTool({ w: "move", e: "rotate", r: "scale" }[k]); e.preventDefault(); e.stopImmediatePropagation(); }
      else if (k === "a" && pvSelection.mode !== "view") { pv.selectAll(); e.preventDefault(); e.stopImmediatePropagation(); }
    }, true);
  }
  if (pv && pv.ok) {
    for (const [id, key] of [["#pv-shaded", "shaded"], ["#pv-edges", "edges"], ["#pv-points", "points"], ["#pv-normals", "normals"]]) {
      $(id).checked = pv.opt[key];
      $(id).onchange = (e) => { pv.opt[key] = e.target.checked; pv.redraw(); };
    }
    $("#pv-frame").onclick = () => pv.frameSelection();
    document.querySelectorAll("#preview [data-view]").forEach((b) => { b.onclick = () => pv.viewAxis(b.dataset.view, false); });
    $("#pv-ortho").onclick = () => toggleOrtho();
    document.querySelectorAll("#pv-xform [data-tool]").forEach((b) => { b.onclick = () => setTool(b.dataset.tool); });
    wireSelectionTools();
  } else { $("#preview .pv-opts").classList.add("hidden"); }
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
  function flowView(r) {                                   // what this node did to the attributes of its inputs
    const f = r.flow;
    if (!f) return [];
    const row = (sign, cls, a, extra) => h("div", { class: "flow " + cls, title: a.domain + (a.type ? " " + a.type : "") },
      h("span", { class: "sign" }, sign), a.name, h("span", { class: "muted" }, " " + (extra || a.domain)));
    const rows = [
      ...f.added.map((a) => row("+", "add", a)), ...f.modified.map((a) => row("~", "mod", a)),
      ...f.moved.map((a) => row("→", "mov", a, `${a.from} → ${a.domain}`)), ...f.removed.map((a) => row("−", "del", a))];
    const reads = r.wrangle && r.wrangle.reads.length ? [h("div", { class: "muted flow-note" }, "reads " + r.wrangle.reads.map((n) => "@" + n).join(" "))] : [];
    const topo = f.topology === "changed" ? [h("div", { class: "muted flow-note" }, `topology changed (${f.counts.points[0]}→${f.counts.points[1]} points, ${f.counts.prims[0]}→${f.counts.prims[1]} prims)`)] : [];
    if (!rows.length && !reads.length && !topo.length) return [];
    return [h("div", { class: "section" }, "Changes from inputs"), ...rows, ...reads, ...topo];
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
    out.push(...flowView(r));
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
    // Copying a boundary means copying its complete body, including nested pairs.
    const expanded = new Set(ids);
    for (const id of ids) {
      const n = nodeOf(id);
      const end = n.type === "loop_end" ? id : n.type === "loop_begin"
        ? Object.keys(S.graph.nodes).find(k => nodeOf(k).type === "loop_end" && nodeOf(k).inputs?.begin === id) : null;
      if (end) for (const member of loopRegion(end)) expanded.add(member);
    }
    ids = [...expanded];
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
  function loopRegion(end) {
    const begin = nodeOf(end)?.inputs?.begin;
    if (!nodeOf(begin)) return [end];
    const ancestors = new Set(), pending = [end];
    while (pending.length) {
      const id = pending.pop();
      if (ancestors.has(id) || !nodeOf(id)) continue;
      ancestors.add(id);
      for (const [, srcs] of inputs(id)) pending.push(...srcs);
    }
    const members = new Set([begin]);
    let changed = true;
    while (changed) {
      changed = false;
      for (const id of ancestors) if (!members.has(id) && inputs(id).some(([, srcs]) => srcs.some(s => members.has(s)))) {
        members.add(id); changed = true;
      }
    }
    return [...members];
  }
  // ---- copy / cut / paste: nodes travel as JSON text, so they can move between graphs and editor windows
  const CLIP_KEY = "minihou.clip", CLIP_LIMIT = 500;
  function clipboardText(ids) {
    const keep = new Set(ids), nodes = {};
    for (const id of ids) {
      const n = nodeOf(id), ins = {};
      for (const [spec, srcs] of inputs(id)) {
        const inner = srcs.filter((s) => keep.has(s));
        if (inner.length) ins[spec.name] = Array.isArray((n.inputs || {})[spec.name]) ? inner : inner[0];
      }
      nodes[id] = { type: n.type, params: n.params || {}, inputs: ins, pos: pos(id), comment: n.comment || undefined };
    }
    return JSON.stringify({ minihou: "nodes", version: 1, nodes });
  }
  function parseClipboard(text) {
    let d;
    try { d = JSON.parse(text); } catch (e) { return null; }
    if (!d || d.minihou !== "nodes" || !d.nodes || typeof d.nodes !== "object" || Array.isArray(d.nodes)) return null;
    return Object.keys(d.nodes).length <= CLIP_LIMIT ? d : null;
  }
  function copySelection(e) {
    const ids = [...S.sel].filter((id) => nodeOf(id));
    if (!ids.length) return false;
    const text = clipboardText(ids);
    try { localStorage.setItem(CLIP_KEY, text); } catch (err) { /* storage unavailable */ }
    if (e && e.clipboardData) { e.clipboardData.setData("text/plain", text); e.preventDefault(); }
    toast(`Copied ${ids.length} node${ids.length > 1 ? "s" : ""}`, false);
    return true;
  }
  async function pasteNodes(text) {
    const d = parseClipboard(text);
    if (!d) return false;
    const ids = Object.keys(d.nodes).filter((id) => d.nodes[id] && typeof d.nodes[id] === "object");
    const unknown = [...new Set(ids.map((id) => d.nodes[id].type).filter((t) => !S.types[t]))];
    const ok = ids.filter((id) => S.types[d.nodes[id].type]);
    if (unknown.length) toast("Not pasted (unknown node type): " + unknown.join(", "));
    if (!ok.length) return true;
    const map = {}, taken = new Set(), okSet = new Set(ok);
    for (const id of ok) { map[id] = uniqueId(id.replace(/\d+$/, "") || d.nodes[id].type, taken); taken.add(map[id]); }
    const where = canvas.getBoundingClientRect(), inside = S.mouse[0] >= where.left && S.mouse[0] <= where.right &&
      S.mouse[1] >= where.top && S.mouse[1] <= where.bottom;
    const poss = ok.map((id) => d.nodes[id].pos).filter((p) => Array.isArray(p) && p.length === 2);
    const x0 = poss.length ? Math.min(...poss.map((p) => p[0])) : 0, y0 = poss.length ? Math.min(...poss.map((p) => p[1])) : 0;
    const [mx, my] = inside ? toWorld(...S.mouse) : [x0 + 40, y0 + 40];
    const ops = ok.map((id) => {
      const n = d.nodes[id], p = Array.isArray(n.pos) && n.pos.length === 2 ? n.pos : [x0, y0];
      const op = { op: "add", id: map[id], type: n.type, params: n.params || {}, pos: [Math.round(mx + p[0] - x0), Math.round(my + p[1] - y0)] };
      if (n.comment) op.comment = String(n.comment);
      return op;
    });
    for (const id of ok) {
      for (const [port, ref] of Object.entries(d.nodes[id].inputs || {})) {
        for (const src of Array.isArray(ref) ? ref : [ref]) if (okSet.has(src)) ops.push({ op: "connect", from: map[src], to: map[id], port });
      }
    }
    const r = await edit(ops);
    if (r) select(Object.values(map));
    return true;
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
    try { const r = await api(redo ? "/api/redo" : "/api/undo", { origin: "editor", document_id: S.documentId }); applySnapshot(r.graph); }
    catch (e) { toast(e.message); }
  }

  // ---------------------------------------------------------------- network boxes
  const BOX_COLORS = ["#5b7fb5", "#5aa06e", "#c98f3a", "#b5585b", "#8a63b5", "#4aa3a8"];
  const boxesEl = $("#boxes");
  const boxRect = (b) => S.boxDrag && S.boxDrag.id === b.id ? S.boxDrag : { pos: b.pos, size: b.size };
  function boxMembers(b) {                                    // nodes whose centre is inside the box
    const [x, y] = b.pos, [w, hh] = b.size;
    return Object.keys(S.graph.nodes).filter((id) => { const [nx, ny] = pos(id); return nx + W / 2 >= x && nx + W / 2 <= x + w && ny + H / 2 >= y && ny + H / 2 <= y + hh; });
  }
  function renderBoxes() {
    const list = S.graph.boxes || [], keep = new Set();
    for (const b of list) {
      keep.add(b.id);
      let el = boxesEl.querySelector(`.netbox[data-id="${CSS.escape(b.id)}"]`);
      if (!el) {
        el = h("div", { class: "netbox", dataset: { id: b.id } },
          h("div", { class: "hdr" }, h("span", { class: "dot", title: "Change colour" }), h("span", { class: "title" }), h("span", { class: "x", title: "Delete the box (nodes stay)" }, "×")),
          h("div", { class: "grip", title: "Resize" }));
        boxesEl.append(el);
      }
      const r = boxRect(b);
      el.style.left = r.pos[0] + "px"; el.style.top = r.pos[1] + "px";
      el.style.width = r.size[0] + "px"; el.style.height = r.size[1] + "px";
      el.style.setProperty("--bc", b.color || BOX_COLORS[0]);
      const t = el.querySelector(".title");
      if (t.textContent !== (b.title || "")) t.textContent = b.title || "";
      t.title = "Double-click to rename";
    }
    for (const el of [...boxesEl.children]) if (!keep.has(el.dataset.id)) el.remove();
  }
  function boxById(id) { return (S.graph.boxes || []).find((b) => b.id === id); }
  async function addBox() {
    const ids = [...S.sel].filter((id) => nodeOf(id));
    let op;
    if (ids.length) op = { op: "box_add", around: ids, title: "Box" };
    else {
      const r = canvas.getBoundingClientRect(), [x, y] = toWorld(r.left + r.width / 2, r.top + r.height / 2);
      op = { op: "box_add", pos: [Math.round(x - 150), Math.round(y - 100)], size: [300, 200], title: "Box" };
    }
    await edit([op]);
  }
  async function renameBox(id) {
    const b = boxById(id);
    if (!b) return;
    const t = prompt("Box title:", b.title || "");
    if (t !== null) await edit([{ op: "box_set", id, title: t }]);
  }

  // ---------------------------------------------------------------- Tab menu
  let menuCtx = null, menuItems = [], menuActive = 0, menuQuery = "";
  // what the menu remembers between sessions: starred nodes, the last ones used, which groups are open
  const menuPrefs = (() => {
    const load = (k, d) => { try { return JSON.parse(localStorage.getItem("minihou." + k) || "null") || d; } catch (e) { return d; } };
    return { favs: new Set(load("favs", [])), recent: load("recent", []), open: new Set(load("menuopen", ["Create", "Polygon Modelling"])) };
  })();
  function saveMenuPrefs() {
    try {
      localStorage.setItem("minihou.favs", JSON.stringify([...menuPrefs.favs]));
      localStorage.setItem("minihou.recent", JSON.stringify(menuPrefs.recent.slice(0, 12)));
      localStorage.setItem("minihou.menuopen", JSON.stringify([...menuPrefs.open]));
    } catch (e) { /* storage unavailable */ }
  }
  function rememberRecent(name) {
    menuPrefs.recent = [name, ...menuPrefs.recent.filter((n) => n !== name)].slice(0, 12);
    saveMenuPrefs();
  }
  function openMenu(cx, cy, ctx) {
    menuCtx = ctx || {};
    const m = $("#tabmenu");
    m.classList.remove("hidden");
    const w = 380, hgt = 480;
    m.style.left = Math.max(4, Math.min(cx, innerWidth - w - 4)) + "px";
    m.style.top = Math.max(4, Math.min(cy, innerHeight - hgt - 4)) + "px";
    const inp = $("#tab-search");
    inp.value = "";
    fillMenu("");
    inp.focus();
  }
  function closeMenu() { $("#tabmenu").classList.add("hidden"); menuCtx = null; canvas.focus({ preventScroll: true }); }
  function menuTypes() {
    let list = [...(S.menuExtras || []), ...S.typeList];
    if (menuCtx && (menuCtx.from || menuCtx.fromSel)) list = list.filter((t) => t.inputs.length);   // must accept a wire
    return list;
  }
  function fillMenu(q) {
    menuQuery = q.trim();
    const rows = NodeMenu.rows(menuTypes(), menuQuery, menuPrefs);
    // the navigable rows: nodes, and the group headers (Enter or the arrow keys fold them)
    menuItems = rows.filter((r) => r.kind === "item" || !r.special).map((r) => (r.kind === "item" ? r.type : { head: true, name: r.name, open: r.open }));
    menuActive = menuItems.findIndex((t) => !t.head);
    if (menuActive < 0) menuActive = 0;
    const kids = [];
    let index = 0;
    for (const r of rows) {
      if (r.kind === "head") {
        const i = r.special ? -1 : index++;
        kids.push(h("div", { class: "cat head" + (r.special ? " special" : "") + (i === menuActive ? " active" : ""), dataset: { group: r.name, i: i < 0 ? "" : i },
          onmousedown: (e) => { e.preventDefault(); if (!r.special) toggleGroup(r.name); }, onmousemove: () => { if (i >= 0 && menuActive !== i) setMenuActive(i); } },
          h("span", { class: "chev" }, r.special ? (r.name === "Favourites" ? "★" : "⏱") : r.open ? "▾" : "▸"), h("span", null, r.name),
          h("small", null, String(r.count))));
        continue;
      }
      const t = r.type, i = index++;
      const star = h("span", { class: "star" + (menuPrefs.favs.has(t.name) ? " on" : ""), title: "Favourite",
        onmousedown: (e) => { e.preventDefault(); e.stopPropagation(); toggleFav(t.name); } }, menuPrefs.favs.has(t.name) ? "★" : "☆");
      const label = h("span", { class: "lb" }, ...NodeMenu.highlight(t.label || t.name, menuQuery).map(([text, hit]) => (hit ? h("mark", null, text) : text)));
      kids.push(h("div", { class: "it" + (i === menuActive ? " active" : "") + (t.source ? " lib" : ""), dataset: { i, name: t.name }, title: t.name + (t.doc ? " - " + t.doc.split("\n")[0] : ""),
        onmousedown: (e) => { e.preventDefault(); pickMenu(i); }, onmousemove: () => { if (menuActive !== i) setMenuActive(i); } },
        label, h("small", null, r.showGroup ? t.menu || "" : (t.doc || "").split("\n")[0]), star));
    }
    if (!menuItems.length) kids.push(h("div", { class: "cat" }, menuQuery ? "No node matches “" + menuQuery + "”" : "no nodes"));
    $("#tab-list").replaceChildren(...kids);
    showMenuDoc();
  }
  function showMenuDoc() {
    const t = menuItems[menuActive], el = $("#tab-doc");
    if (!t) { el.textContent = ""; return; }
    if (t.head) {
      const g = (S.menuGroups || []).find((x) => x.name === t.name);
      el.replaceChildren(h("b", null, t.name), h("div", { class: "d" }, (g && g.doc) || ""), h("div", { class: "io" }, "Enter or ← → opens and closes the group"));
      return;
    }
    const ins = (t.inputs || []).map((p) => p.name + (p.optional ? "?" : "") + (p.multi ? "*" : "")).join(", ") || "none";
    el.replaceChildren(h("b", null, t.label || t.name), h("span", { class: "grp" }, " · " + (t.menu || "")),
      h("div", { class: "d" }, (t.doc || "").split("\n")[0] || "No description."), h("div", { class: "io" }, "in: " + ins + "  →  " +
      (t.outputs || [{ name: "geo" }]).map((p) => p.name).join(", ") + (t.params && t.params.length ? "  ·  " + t.params.length + " parameters" : "")));
  }
  function toggleGroup(name) {
    if (menuPrefs.open.has(name)) menuPrefs.open.delete(name); else menuPrefs.open.add(name);
    saveMenuPrefs();
    fillMenu($("#tab-search").value);
    const at = menuItems.findIndex((t) => t.head && t.name === name);                // keep the header under the cursor
    if (at >= 0) setMenuActive(at);
  }
  function toggleFav(name) {
    if (menuPrefs.favs.has(name)) menuPrefs.favs.delete(name); else menuPrefs.favs.add(name);
    saveMenuPrefs();
    fillMenu($("#tab-search").value);
  }
  function setMenuActive(i) {
    menuActive = i;
    const items = [...$("#tab-list").querySelectorAll("[data-i]")].filter((el) => el.dataset.i !== "");
    items.forEach((el) => el.classList.toggle("active", Number(el.dataset.i) === i));
    const cur = items.find((el) => Number(el.dataset.i) === i);
    if (cur) cur.scrollIntoView({ block: "nearest" });
    showMenuDoc();
  }
  function moveMenu(d) {
    if (!menuItems.length) return;
    setMenuActive((menuActive + d + menuItems.length) % menuItems.length);
  }
  // left / right fold the group of the highlighted node (when nothing is typed)
  function foldMenu(open) {
    if (menuQuery) return false;
    const t = menuItems[menuActive];
    const g = t ? (t.head ? t.name : t.menu) : null;
    if (!g || menuPrefs.open.has(g) === open) return false;
    toggleGroup(g);
    return true;
  }
  async function pickMenu(i) {
    const t = menuItems[i], ctx = menuCtx || {};
    if (t && t.head) { toggleGroup(t.name); return; }
    closeMenu();
    if (!t) return;
    rememberRecent(t.name);
    const id = uniqueId(t.name);
    let [x, y] = ctx.world || toWorld(...S.mouse);
    x -= W / 2; y -= 12;
    if (t.name === "loop_block") {
      const source = ctx.from || ctx.fromSel;
      if (ctx.fromSel) { const p = pos(ctx.fromSel); x = p[0] + 190; y = p[1] + 110; }
      const r = await edit([{ op: "loop", from: source, to: ctx.feeds?.to, port: ctx.feeds?.port, pos: [x, y] }]);
      if (r) {
        const end = r.results[0].end;
        select([end]);
      }
      return;
    }
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
    const boxEl = t.closest(".netbox");
    if (boxEl && e.button === 0 && !port && !nodeEl) {
      const b = boxById(boxEl.dataset.id);
      if (b && t.closest(".dot")) {
        const i = Math.max(0, BOX_COLORS.indexOf(b.color));
        edit([{ op: "box_set", id: b.id, color: BOX_COLORS[(i + 1) % BOX_COLORS.length] }]);
        return;
      }
      if (b && t.closest(".x")) { edit([{ op: "box_remove", id: b.id }]); return; }
      if (b && (t.closest(".hdr") || t.closest(".grip"))) {
        const [wx, wy] = toWorld(e.clientX, e.clientY);
        const members = t.closest(".grip") ? [] : boxMembers(b);
        drag = { kind: t.closest(".grip") ? "boxresize" : "boxmove", id: b.id, wx, wy, pos: b.pos.slice(), size: b.size.slice(),
          ids: members, start: new Map(members.map((i) => [i, pos(i).slice()])), moved: false };
        e.preventDefault();
        window.addEventListener("pointermove", onDrag); window.addEventListener("pointerup", onDrop);
        return;
      }
    }
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
    } else if (drag.kind === "boxmove" || drag.kind === "boxresize") {
      const [wx, wy] = toWorld(e.clientX, e.clientY), dx = wx - drag.wx, dy = wy - drag.wy;
      if (Math.abs(dx) + Math.abs(dy) > 1) drag.moved = true;
      if (drag.kind === "boxmove") {
        S.boxDrag = { id: drag.id, pos: [Math.round(drag.pos[0] + dx), Math.round(drag.pos[1] + dy)], size: drag.size };
        for (const id of drag.ids) { const [x, y] = drag.start.get(id); S.dragPos.set(id, [Math.round(x + dx), Math.round(y + dy)]); }
        for (const id of drag.ids) { const el = nodesEl.querySelector(`.node[data-id="${CSS.escape(id)}"]`); if (el) { const [x, y] = pos(id); el.style.left = x + "px"; el.style.top = y + "px"; } }
        renderWires();
      } else S.boxDrag = { id: drag.id, pos: drag.pos, size: [Math.max(80, Math.round(drag.size[0] + dx)), Math.max(60, Math.round(drag.size[1] + dy))] };
      renderBoxes();
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
    if (d.kind === "boxmove" || d.kind === "boxresize") {
      const fin = S.boxDrag; S.boxDrag = null;
      if (!d.moved || !fin) { S.dragPos.clear(); renderBoxes(); return; }
      const ops = [];
      if (d.kind === "boxmove") {
        const positions = {};
        for (const id of d.ids) positions[id] = pos(id);
        if (d.ids.length) ops.push({ op: "move", positions });
        ops.push({ op: "box_set", id: d.id, pos: fin.pos });
      } else ops.push({ op: "box_set", id: d.id, size: fin.size });
      const r = await edit(ops);
      S.dragPos.clear();
      if (!r) { renderBoxes(); renderNodes(); }
      return;
    }
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
  canvas.addEventListener("dblclick", (e) => {
    const hdr = e.target.closest(".netbox .title");
    if (hdr) { renameBox(hdr.closest(".netbox").dataset.id); e.stopImmediatePropagation(); }
  });
  canvas.addEventListener("dblclick", (e) => {                                      // double-click a group (system) node: open it, with a breadcrumb back
    const el = e.target.closest(".node");
    if (!el) return;
    const id = el.dataset.id, t = typeOf(id);
    if (t.category === "system" && t.source) { e.stopImmediatePropagation(); openFile(t.source, false, { mode: "push", via: id }); }
  });
  canvas.addEventListener("dblclick", (e) => { if (!e.target.closest(".node") && !e.target.closest(".netbox")) openMenu(e.clientX, e.clientY, { world: toWorld(e.clientX, e.clientY) }); });
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
    if (e.altKey && e.key === "ArrowUp" && !/^(input|textarea|select)$/i.test(e.target.tagName || "")) { e.preventDefault(); goUp(); return; }   // out of a nested group
    const menuOpen = !$("#tabmenu").classList.contains("hidden");
    if (menuOpen) {
      if (e.key === "Escape") { closeMenu(); e.preventDefault(); }
      else if (e.key === "ArrowDown") { moveMenu(1); e.preventDefault(); }
      else if (e.key === "ArrowUp") { moveMenu(-1); e.preventDefault(); }
      else if (e.key === "ArrowLeft" && !$("#tab-search").value) { foldMenu(false); e.preventDefault(); }
      else if (e.key === "ArrowRight" && !$("#tab-search").value) { foldMenu(true); e.preventDefault(); }
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
    else if ((e.key === "o" || e.key === "O") && !mod) addBox();
    else if (e.key === "?") $("#help").classList.toggle("hidden");
  });
  const typing = (el) => ["input", "textarea", "select"].includes((el.tagName || "").toLowerCase()) || el.isContentEditable;
  document.addEventListener("copy", (e) => { if (!typing(e.target)) copySelection(e); });
  document.addEventListener("cut", (e) => {
    if (typing(e.target) || !copySelection(e)) return;
    removeNodes([...S.sel]);
  });
  document.addEventListener("paste", (e) => {
    if (typing(e.target)) return;
    let text = e.clipboardData ? e.clipboardData.getData("text/plain") : "";
    if (!parseClipboard(text)) { try { text = localStorage.getItem(CLIP_KEY) || ""; } catch (err) { text = ""; } }
    if (parseClipboard(text)) { e.preventDefault(); pasteNodes(text); }
  });
  $("#tab-search").addEventListener("input", (e) => fillMenu(e.target.value));
  $("#tab-search").addEventListener("blur", () => setTimeout(() => { if (document.activeElement !== $("#tab-search")) $("#tabmenu").classList.add("hidden"); }, 100));

  // ---------------------------------------------------------------- top bar, file menu, splitter
  $("#undo-btn").onclick = () => undo(false);
  $("#redo-btn").onclick = () => undo(true);
  $("#add-btn").onclick = (e) => { const r = canvas.getBoundingClientRect(); openMenu(e.clientX, e.clientY + 20, { world: toWorld(r.left + r.width / 2, r.top + r.height / 2) }); };
  $("#help-btn").onclick = () => $("#help").classList.toggle("hidden");
  $("#tidy-btn").onclick = () => tidy();
  $("#box-btn").onclick = () => addBox();
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
  async function openFile(path, create, nav) {
    $("#file-menu").classList.add("hidden");
    S.nav = nav || null;
    try { const snap = await api("/api/open", { path, create: !!create }); S.sel.clear(); applySnapshot(snap); }
    catch (e) { toast(e.message); }
    S.nav = null;
  }
  function newFile() {
    let name = prompt("New graph file (inside " + (S.root || "the workspace") + "):", "graphs/my_system.json");
    if (!name) return;
    name = name.trim();
    if (!name.toLowerCase().endsWith(".json")) name += ".json";
    openFile(name, true);
  }
  // ---------------------------------------------------------------- layout: graph, 3-D view and parameter panel are resizable panes
  const layout = (() => {
    const KEY = "minihou.layout";
    const work = $("#work"), graphPane = $("#canvas"), viewPane = $("#preview"), side = $("#side"), params = $("#params"), info = $("#info");
    const L = { viewFrac: 0.5, dir: "row", swap: false, max: null, side: true, sideW: 360, paramsFrac: 0.6 };
    try { Object.assign(L, JSON.parse(localStorage.getItem(KEY) || "{}")); } catch (e) { /* storage unavailable or damaged */ }
    L.viewFrac = Math.min(0.92, Math.max(0.08, +L.viewFrac || 0.5));
    L.paramsFrac = Math.min(0.9, Math.max(0.1, +L.paramsFrac || 0.6));
    L.sideW = Math.max(220, +L.sideW || 360);
    if (L.dir !== "col") L.dir = "row";
    if (L.max !== "graph" && L.max !== "view") L.max = null;
    const save = () => { try { localStorage.setItem(KEY, JSON.stringify(L)); } catch (e) { /* ignore */ } };
    let redrawTimer = 0;
    function apply() {
      work.classList.toggle("col", L.dir === "col");
      work.classList.toggle("max-graph", L.max === "graph");
      work.classList.toggle("max-view", L.max === "view");
      document.body.classList.toggle("side-hidden", !L.side);
      const first = L.swap ? viewPane : graphPane, second = L.swap ? graphPane : viewPane;
      first.style.order = 0; $("#work-split").style.order = 1; second.style.order = 2;
      const f = L.swap ? L.viewFrac : 1 - L.viewFrac;                                 // share of the first pane
      first.style.flex = `0 0 calc(${(f * 100).toFixed(3)}% - 2.5px)`;
      second.style.flex = "1 1 0";
      for (const p of [graphPane, viewPane]) p.style.flex = L.max ? "1 1 0" : p.style.flex;
      side.style.width = innerWidth > 760 ? L.sideW + "px" : "";
      params.style.flex = `0 0 ${(L.paramsFrac * 100).toFixed(2)}%`;
      info.style.flex = "1 1 0";
      document.querySelectorAll("#topbar [data-layout]").forEach((b) => {
        const want = { graph: 0.28, split: 0.5, view: 0.78 }[b.dataset.layout];
        b.classList.toggle("on", !L.max && Math.abs(L.viewFrac - want) < 0.02);
      });
      $("#pv-max").classList.toggle("on", L.max === "view");
      $("#lay-side").classList.toggle("on", L.side);
      if (pv) pv.redraw();
      clearTimeout(redrawTimer); redrawTimer = setTimeout(() => { if (pv) pv.redraw(); }, 60);
    }
    function set(patch) {
      Object.assign(L, patch); apply(); save();
      if (L.max !== "view") setTimeout(() => frame(false), 30);                         // the graph pane changed shape: show all of it again
    }
    function dragSplit(el, onMove, cursorClass) {
      el.addEventListener("pointerdown", (e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        document.body.classList.add("dragging-split"); el.classList.add("active");
        const move = (ev) => { onMove(ev); };
        const up = () => {
          window.removeEventListener("pointermove", move); window.removeEventListener("pointerup", up); window.removeEventListener("pointercancel", up);
          document.body.classList.remove("dragging-split"); el.classList.remove("active"); save();
        };
        window.addEventListener("pointermove", move); window.addEventListener("pointerup", up); window.addEventListener("pointercancel", up);
      });
    }
    dragSplit($("#work-split"), (ev) => {
      const r = work.getBoundingClientRect();
      const p = L.dir === "col" ? (ev.clientY - r.top) / r.height : (ev.clientX - r.left) / r.width;      // where the first pane ends
      const f = Math.min(0.92, Math.max(0.08, p));
      L.viewFrac = L.swap ? f : 1 - f; apply();
    });
    dragSplit($("#splitter"), (ev) => { L.sideW = Math.round(Math.max(220, Math.min(innerWidth * 0.7, innerWidth - ev.clientX))); apply(); });
    dragSplit($("#side-split"), (ev) => {
      const r = side.getBoundingClientRect();
      L.paramsFrac = Math.min(0.9, Math.max(0.1, (ev.clientY - r.top) / r.height)); apply();
    });
    document.querySelectorAll("#topbar [data-layout]").forEach((b) => {
      b.onclick = () => set({ viewFrac: { graph: 0.28, split: 0.5, view: 0.78 }[b.dataset.layout], max: null });
    });
    $("#lay-dir").onclick = () => set({ dir: L.dir === "row" ? "col" : "row" });
    $("#lay-swap").onclick = () => set({ swap: !L.swap });
    $("#lay-side").onclick = () => set({ side: !L.side });
    $("#pv-max").onclick = () => set({ max: L.max === "view" ? null : "view" });
    let hover = null;                                                                  // which pane the pointer is over, for Ctrl+Space
    graphPane.addEventListener("pointerenter", () => { hover = "graph"; });
    viewPane.addEventListener("pointerenter", () => { hover = "view"; });
    document.addEventListener("keydown", (e) => {
      if (e.code !== "Space" || !e.ctrlKey || e.altKey || e.metaKey) return;
      const tag = (e.target.tagName || "").toLowerCase();
      if (tag === "input" || tag === "textarea" || tag === "select") return;
      if (L.max) set({ max: null }); else if (hover) set({ max: hover });
      e.preventDefault(); e.stopImmediatePropagation();
    }, true);
    window.addEventListener("resize", apply);
    apply();
    return { state: L, set, apply };
  })();

  applyView();
  try { recoveryButton(); } catch (e) { /* Browser storage may be disabled. */ }
  connectEvents();
  window.minihou = { S, edit, frame, clipboardText, pasteNodes, preview: pv, layout };                  // handy from the browser console
})();
