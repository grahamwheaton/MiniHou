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

  // In-app text and confirmation dialogs, shared by file and graph actions.
  function appDialog(message, initial, confirmation = false) {
    return new Promise(resolve => {
      const dialog = h("dialog", { class: "app-dialog", "aria-label": confirmation ? "Confirm action" : "Edit value" });
      const input = h("input", { type: "text", value: initial || "", "aria-label": message });
      const form = h("form", { method: "dialog" }, h("h2", null, confirmation ? "Confirm action" : "MiniHou"),
        h("p", null, message), confirmation ? null : input,
        h("div", { class: "dialog-actions" }, h("button", { type: "button", onclick: () => dialog.close("cancel") }, "Cancel"), h("button", { type: "submit", class: "primary" }, confirmation ? "Confirm" : "OK")));
      form.onsubmit = e => { e.preventDefault(); dialog.close("ok"); };
      dialog.append(form); document.body.append(dialog);
      dialog.addEventListener("close", () => { const ok = dialog.returnValue === "ok"; const value = input.value; dialog.remove(); resolve(confirmation ? ok : ok ? value : null); }, { once: true });
      dialog.showModal(); if (!confirmation) { input.focus(); input.select(); }
    });
  }
  const appPrompt = (message, initial) => appDialog(message, initial);
  const appConfirm = message => appDialog(message, "", true);
  document.addEventListener("keydown", e => { if (document.querySelector(".app-dialog[open]")) e.stopImmediatePropagation(); }, true);

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
  let desktopPendingEdits = 0;
  async function edit(ops, context = { path: S.path, documentId: S.documentId }) {
    desktopPendingEdits++;
    try {
      const r = await api("/api/edit", { ops, origin: "editor", document_id: context.documentId });
      applySnapshot(r.graph);
      return r;
    } catch (e) {
      if (context.documentId !== S.documentId || e.message.includes("Editor graph changed")) stashEdits(ops, context);
      else toast(e.message);
      return null;
    } finally { desktopPendingEdits--; }
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
  let fullStateLoaded = false;
  async function loadState() {
    try {
      const st = await api("/api/state");
      S.menuExtras = (st.menu && st.menu.extras) || []; S.menuGroups = (st.menu && st.menu.groups) || [];
      setTypes(st.nodes, st.library);
      S.root = st.root;
      S.status = st.status;
      S.status.clients = st.link.clients;
      S.rev = -1;                                    // the engine may have restarted: accept its revision
      if (!fullStateLoaded) { S.framed = false; fullStateLoaded = true; }
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
    if (window.MiniShelf) MiniShelf.refresh();
    const errs = new Set(((library && library.files) || []).filter((f) => f.error).map((f) => relPath(f.file) + ": " + f.error));
    const fresh = [...errs].filter((e) => !libErrors.has(e));
    if (fresh.length) toast("Node library: " + fresh.join("\n"));
    libErrors = errs;
  }
  let lastRemembered = null;
  function applySnapshot(snap) {
    if (!snap || snap.rev < S.rev) return;
    if (S.documentId !== null && snap.document_id !== S.documentId && pendingSet.size) {
      stashEdits([...pendingSet].map(([id, params]) => ({ op: "set", id, params })), pendingContext);
      pendingSet.clear(); pendingContext = null;
    }
    const fileChanged = snap.path !== S.path;
    let liveBadge = $("#live-modifier");
    if (!liveBadge) { liveBadge = h("span", { id: "live-modifier", class: "status" }); $("#preview").prepend(liveBadge); liveBadge.style.cssText = "flex:none;padding:6px 10px;white-space:normal;color:#8fdbad"; }
    liveBadge.textContent = snap.live_modifier ? `${snap.live_modifier.connected ? "Live" : "Disconnected"}: ${snap.live_modifier.label}` : "";
    liveBadge.style.display = snap.live_modifier ? "block" : "none";
    if (fileChanged && snap.path) updateTrail(snap.path);                           // a snapshot without a file (transient) leaves the path alone
    if (snap.path && snap.path !== lastRemembered && S.root) { lastRemembered = snap.path; rememberFile(relPath(snap.path)); }
    S.rev = snap.rev; S.graph = snap.graph; S.path = snap.path; S.documentId = snap.document_id;
    S.canUndo = snap.can_undo; S.canRedo = snap.can_redo;
    for (const id of [...S.sel]) if (!S.graph.nodes[id]) S.sel.delete(id);
    if (S.wireSel && !wireExists(S.wireSel)) S.wireSel = null;
    render();
    if (fileChanged || !S.framed) { frame(false); S.framed = true; requestAnimationFrame(() => { if (!drag && !S.sel.size) frame(false); }); }
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
  function inputMembers(id) {
    return Object.entries(S.graph.nodes).filter(([, n]) => n.type === "object_input" && n.input_owner === id);
  }
  function inputOwner(id) {
    const owner = nodeOf(id)?.input_owner;
    return nodeOf(id)?.type === "object_input" && nodeOf(owner)?.type === "modifier_input" ? owner : null;
  }
  function nodeHeight(id) { return H + inputMembers(id).length * 22; }
  function pos(id) {
    const owner = inputOwner(id);
    if (owner) return pos(owner);
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
  const outPortCenter = (id) => {
    const owner = inputOwner(id);
    if (owner) {
      const [x, y] = pos(owner), i = inputMembers(owner).findIndex(([key]) => key === id);
      return [x + W, y + H + i * 22];
    }
    const [x, y] = pos(id); return [x + W / 2, y + nodeHeight(id)];
  };
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
    canvas.style.backgroundSize = `${(Number(canvas.dataset.gridSpacing) || 24) * (canvas.dataset.backdrop === "checker" ? 2 : 1) * k}px ${(Number(canvas.dataset.gridSpacing) || 24) * (canvas.dataset.backdrop === "checker" ? 2 : 1) * k}px`;
    canvas.style.backgroundPosition = `${x}px ${y}px`;
  }
  function frame(selectedOnly) {
    const ids = selectedOnly && S.sel.size ? [...S.sel] : Object.keys(S.graph.nodes);
    const r = canvas.getBoundingClientRect();
    if (!ids.length) { S.view = { x: r.width / 2 - W / 2, y: r.height / 3, k: 1 }; applyView(); return; }
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const id of ids) { const [x, y] = pos(id); x0 = Math.min(x0, x); y0 = Math.min(y0, y - 30); x1 = Math.max(x1, x + W); y1 = Math.max(y1, y + nodeHeight(id) + 10); }
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
      if (inputOwner(id) || hiddenInFrame(id)) continue;
      keep.add(id);
      const t = typeOf(id);
      let el = nodesEl.querySelector(`.node[data-id="${CSS.escape(id)}"]`);
      if (!el) { el = h("div", { class: "node", dataset: { id } }); nodesEl.append(el); }
      el.style.background = /^#[0-9a-f]{6}$/i.test(n.color || "") ? n.color : "";
      const [x, y] = pos(id);
      el.style.left = x + "px"; el.style.top = y + "px"; el.style.height = nodeHeight(id) + "px";
      el.className = ["node", "cat-" + (t.category || "other"), t.category === "system" && t.source && "group", S.sel.has(id) && "selected", id === disp && "display",
        id === errNode && "error", !live.has(id) && "unused"].filter(Boolean).join(" ");
      const ex = exposedOf(id);
      // Rebuild a node's DOM only when its structure changes (wiring, comment, flags); status updates touch just the
      // status line. Stable elements keep hover state and never vanish under a click or a drag.
      const subSig = JSON.stringify([id === errNode && st.error.message, nodeMs[id], cached.has(id), n.type]);
      const structSig = JSON.stringify([n.flags,!!n.locked_geometry,S.graph.render,inputMembers(id).map(([key, child]) => [key, child.input_label]), n.type, n.inputs, n.comment, ex.map((e) => e.name), t.inputs.length, id === disp, t.category === "system" && !!t.source]);
      if (el._struct === structSig) {
        if (el._sub !== subSig) { el._sub = subSig; el.querySelector(".sub").replaceChildren(...subLine(id, n, st, errNode, nodeMs, cached)); }
        continue;
      }
      el._struct = structSig; el._sub = subSig;
      const kids = [h("div", { class: "band" }), h("div", { class: "title", title: id }, id),
        h("div", { class: "sub" }, subLine(id, n, st, errNode, nodeMs, cached)),
        ...['bypass','lock','template','display'].map(name=>h('button',{class:'flag flag-'+name+(flagState(id,name)?' on':''),dataset:{flag:name},title:flagTitle(name), 'aria-label':flagTitle(name),'aria-pressed':String(flagState(id,name))}, ({bypass:'B',lock:'L',template:'T',display:'D'})[name]))];
      el.classList.toggle('bypassed',!!n.flags?.bypass);
      el.classList.toggle('templated',!!(n.flags?.template||n.flags?.selectable_template));
      el.classList.toggle('selectable-template',!!n.flags?.selectable_template);
      if(n.flags?.unload)kids.push(h('span',{class:'unload-badge',title:'Unload: do not retain intermediate geometry'},'U'));
      inputs(id).forEach(([spec, srcs], i) => {
        const cx = (W * (i + 1)) / (t.inputs.length + 1);
        const missing = !srcs.length && !spec.optional;
        kids.push(h("div", { class: ["port", "in", spec.multi && "multi", missing && "missing"].filter(Boolean).join(" "),
          dataset: { port: spec.name, node: id }, title: spec.name + (spec.multi ? " (any number)" : ""),
          style: { left: cx - (spec.multi ? 17 : 7) + "px" } }));
        kids.push(h("div", { class: "port-label", style: { left: cx + "px" } }, spec.name));
      });
      kids.push(h("div", { class: "port out", dataset: { node: id }, title: "output: drag to an input" }));
      inputMembers(id).forEach(([key, child], i) => {
        kids.push(h("div", { class: "input-output-label", style: { position: "absolute", right: "14px", top: (H + i * 22 - 8) + "px", fontSize: "11px" } }, child.input_label || key));
        kids.push(h("div", { class: "port out", dataset: { node: key }, title: child.input_label || key,
          style: { left: (W - 7) + "px", top: (H + i * 22 - 7) + "px", bottom: "auto" } }));
      });
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
          if (!nodeOf(src) || hiddenInFrame(id) || hiddenInFrame(src)) return;
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
    $("#app-document").textContent = S.path ? relPath(S.path) : "Untitled graph";
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
      const rf = st.proxy && st.proxy.refining;
      c.className = "status busy"; txt.textContent = rf ? "refining to full…" : "cooking…";
    } else if (st.ok) {
      const px = st.proxy && st.proxy.cooked < 1 ? st.proxy.cooked : 0;
      c.className = "status " + (px ? "proxy" : "ok");
      txt.textContent = `${fmt(st.points)} pts · ${fmt(st.prims)} prims` + (st.cook ? ` · ${st.cook.ms.toFixed(1)} ms` : "") + (px ? ` · proxy ${pct(px)}` : "");
    } else { c.className = "status"; txt.textContent = "idle"; }
    renderProxy();
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
    const show = BB.forced;
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
        "No Blender found here. Download the ZIP, then use Blender Preferences > Add-ons > Install from Disk:";
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
  $("#bb-zip").onclick = () => {
    BB.msg = true;
    $("#bb-msg").textContent = "In Blender: Preferences > Add-ons > menu > Install from Disk. Choose the downloaded ZIP, then enable MiniHou Link.";
  };

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
    if(curveTool){const id=S.sel.size===1?[...S.sel][0]:null,n=id&&nodeOf(id),spec=n?.type==='bezier'?typeOf(id).params.find(p=>p.name==='controls'):null;curveTool.update(spec?id:null,spec?effective(id,spec):null,n?.params?.closed,spec?.default);}
    updateViewportHints();
    updateViewportContext();
    const panel = $("#params");
    const ids = [...S.sel].filter((id) => nodeOf(id));
    document.body.classList.toggle("no-selection", ids.length === 0);
    const bodyKey = ids.length === 1 && nodeOf(ids[0]).type === "for_each" ? effective(ids[0], typeOf(ids[0]).params.find((p) => p.name === "body")) : "";
    const key = ids.length === 1 ? ids[0] + ":" + bodyKey + JSON.stringify({ ...nodeOf(ids[0]), params: undefined }) + JSON.stringify(inputMembers(ids[0])) : ids.length ? "multi:" + ids.join(",") : "graph";
    const busy = scrubbing || panel.contains(document.activeElement);
    if (key === panelKey && (ids.length === 1 || busy || statusOnly)) { syncParams(ids[0]); return; }
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
    if (n.type === "modifier_input") {
      out.push(h("div", { class: "section" }, "Modifier inputs"));
      out.push(h("div", { class: "pdoc" }, "Add object inputs here. Pick their Blender objects on the modifier, then wire the named outputs into your graph."));
      for (const [key, child] of inputMembers(id)) {
        out.push(h("div", { class: "pw" },
          h("input", { type: "text", value: child.input_label || key, "aria-label": "Input name", onchange: async (e) => {
            const label = e.target.value.trim();
            if (label) await edit([{ op: "meta", id: key, set: { input_label: label } }]);
          } }),
          h("button", { title: "Remove this input and its wires", onclick: async () => {
            await edit([{ op: "remove", id: key }]); panelKey = null; renderParams();
          } }, "Remove")));
      }
      out.push(h("button", { id: "add-modifier-input", onclick: async () => {
        const label = await appPrompt("Object input name", "Object");
        if (!label?.trim()) return;
        const key = uniqueId(id + "_object_");
        const r = await edit([{ op: "add", id: key, type: "object_input", pos: pos(id) },
          { op: "meta", id: key, set: { input_owner: id, input_label: label.trim() } }]);
        if (r) { panelKey = null; render(); }
      } }, "+ Add Object Input"));
    }
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
    if(n.type==='bezier'&&curveTool)out.push(curveTool.panel());
    if (n.type === "rbd_solver") out.push(h("button", {
      onclick: () => queueSet(id, "reset", Number(n.params?.reset || 0) + 1),
      title: "Discard this simulation and replay from its initial geometry"
    }, "Reset simulation"));
    let ogHeader = false;
    for (let i = 0; i < t.params.length; i++) {
      const p = t.params[i];
      if(n.type==='bezier'&&p.name==='controls')continue;
      if (p.ui === "outgroup") {                                     // Output Groups: a checkbox and the group's name side by side, like Houdini
        if (!ogHeader) { out.push(h("div", { class: "ogheader" }, "Output Groups")); ogHeader = true; }
        const q = t.params[i + 1], named = q && q.ui === "outgroup_name";
        const pair = h("div", { class: "ogpair" + (effective(id, p) ? "" : " off") }, paramRow(id, p), named ? paramRow(id, q) : null);
        pair.addEventListener("change", (e) => { if (e.target.type === "checkbox") pair.classList.toggle("off", !e.target.checked); });
        out.push(pair);
        if (named) i++;
        continue;
      }
      if (n.type === "for_each" && p.name === "params") {
        const advanced = h("details", null, h("summary", null, "Advanced parameter JSON"), paramRow(id, p));
        out.push(advanced);
      } else out.push(paramRow(id, p));
    }
    if (n.type === "for_each") out.push(...foreachControls(id));
    if (ins.length) out.push(streamAttributes(id, ins));
    out.push(h("div", { class: "section" }, "Note"));
    out.push(h("div", { class: "pw" }, h("input", { type: "text", value: n.comment || "", placeholder: "comment shown above the node",
      onchange: (e) => edit([{ op: "meta", id, set: { comment: e.target.value.trim() || null } }]) })));
    return out;
  }
  function foreachControls(id) {
    const spec = typeOf(id).params;
    const body = effective(id, spec.find((p) => p.name === "body"));
    const t = S.types[body];
    if (!t) return [h("div", { class: "hint" }, "Choose a body node from the suggestions. These are operations, not geometry attributes.")];
    let values;
    try { values = JSON.parse(effective(id, spec.find((p) => p.name === "params")) || "{}"); }
    catch { return [h("div", { class: "err" }, "Fix the advanced parameter JSON to show the body controls.")]; }
    if (!values || Array.isArray(values) || typeof values !== "object") return [];
    const out = [h("div", { class: "section" }, (t.label || t.name) + " controls"), h("div", { class: "hint" }, t.doc || "")];
    const set = (name, value) => {
      const current = JSON.parse(effective(id, spec.find((p) => p.name === "params")) || "{}");
      current[name] = value;
      queueSet(id, "params", JSON.stringify(current));
    };
    for (const p of t.params) {
      const v = values[p.name] ?? p.default;
      let input;
      if (p.kind === "bool") input = h("input", { type: "checkbox", checked: v, onchange: (e) => set(p.name, e.target.checked) });
      else if (p.kind === "enum") input = h("select", { onchange: (e) => set(p.name, e.target.value) }, ...p.choices.map((x) => h("option", { value: x, selected: x === v }, x)));
      else if (p.kind === "vec3") input = h("div", { class: "pw vec" }, ...v.map((x, i) => h("input", { type: "number", value: x, step: "any", onchange: (e) => { const next = [...v]; next[i] = Number(e.target.value); set(p.name, next); } })));
      else input = h("input", { type: ["int", "float"].includes(p.kind) ? "number" : "text", value: v, step: p.kind === "int" ? 1 : "any", min: p.min, max: p.max,
        onchange: (e) => set(p.name, ["int", "float"].includes(p.kind) ? Number(e.target.value) : e.target.value) });
      out.push(h("div", { class: "prow body-control", dataset: { bodyParam: p.name } }, h("span", { class: "pl", title: p.doc }, p.label || p.name), h("div", { class: "pw" }, input)));
    }
    if (!t.params.length) out.push(h("div", { class: "hint" }, "This operation has no parameters."));
    return out;
  }

  function streamAttributes(id, ins) {
    const content = h("div", { class: "stream-tags" });
    const details = h("details", { class: "stream-attributes" }, h("summary", null, "Incoming geometry attributes"), content);
    let loaded = false;
    details.addEventListener("toggle", async () => {
      if (!details.open || loaded) return;
      loaded = true;
      content.replaceChildren(h("div", { class: "hint" }, "Inspecting inputs…"));
      const documentId = S.documentId;
      const blocks = [];
      for (const [input, sources] of ins) for (const source of sources) {
        try {
          const r = await api("/api/inspect", { node: source });
          if (documentId !== S.documentId || !details.isConnected) return;
          blocks.push(h("div", { class: "hint" }, input.name + " ← " + source));
          if (!r.ok) { blocks.push(h("div", { class: "err" }, r.error?.message || "Input could not cook")); continue; }
          for (const a of r.attributes || []) {
            const target = typeOf(id).params.find((p) => ["attribute", "piece_attrib"].includes(p.name));
            const usable = a.domain === "prim" && a.components === 1 && !!target;
            blocks.push(h(usable ? "button" : "span", { class: "stream-tag", title: a.domain + " · " + (a.type || "detail value"),
              onclick: usable ? () => { queueSet(id, target.name, a.name); if (nodeOf(id).type === "for_each") queueSet(id, "piece", "attribute"); renderParams(); } : null }, a.name + " · " + a.domain));
          }
          if (!(r.attributes || []).length) blocks.push(h("div", { class: "hint" }, "No attributes on this input."));
        } catch (e) { blocks.push(h("div", { class: "err" }, e.message)); }
      }
      content.replaceChildren(...blocks);
    });
    return details;
  }

  function paramRow(id, p) {
    const v = effective(id, p);
    const label = h("span", { class: "pl" + (isSet(id, p) ? " changed" : ""), title: (p.doc ? p.doc + "\n" : "") + p.name + " (" + p.kind + ")" }, p.label || p.name);
    const row = h("div", { class: "prow " + p.kind + (p.ui === "code" ? " code" : ""), dataset: { param: p.name } });
    let widget;
    if (S.graph.nodes[id].type === "for_each" && p.name === "body") {
      const listId = "foreach-body-" + id;
      const choices = S.typeList.filter((t) => t.inputs.length);
      const input = h("input", { type: "search", value: v, placeholder: "Search nodes…", "aria-label": "Body node",
        dataset: { role: "str" }, onchange: (e) => { queueSet(id, p.name, e.target.value); renderParams(); } });
      input.setAttribute("list", listId);
      widget = h("div", { class: "pw" }, input, h("datalist", { id: listId }, ...choices.map((t) => h("option", { value: t.name, label: (t.label || t.name) + " · " + (t.menu || t.category || "") }))));
    } else if (p.kind === "float" || p.kind === "int") {
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
  function syncParams(id, panel = $("#params")) {
    // Update values in place, keeping focus and in-progress drags.
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
  const curveTool = pv?.ok && window.MiniCurveTool ? MiniCurveTool(pv,$('#pv-canvas'),(id,value)=>queueSet(id,'controls',value)) : null;
  let pvTimer = null, pvSeq = 0, pvKey = null, pvNode = null, pvPath = null, pvOver = false, pvFull = false, pvReduced = null;
  const pct = (x) => (x >= 0.1 ? Math.round(x * 100) : Math.round(x * 1000) / 10) + "%";
  // A big geometry is drawn reduced (clustered to a budget): say so, offer full detail, and keep selection off (the drawn points are not the real ones)
  function setReducedView(p) {
    pvReduced = p;
    $("#pv-proxy").classList.toggle("hidden", !p && !pvFull);
    $("#pv-proxy-text").textContent = p ? `Reduced view: ${fmt(p.shown_tris)} of ${fmt(p.tris)} triangles` + (p.points > p.shown_points ? `, ${fmt(p.shown_points)} of ${fmt(p.points)} points` : "") : "";
    $("#pv-full").classList.toggle("on", pvFull);
    $("#pv-full").textContent = pvFull ? "Full res ✓" : "Full res";
    document.querySelectorAll("#preview [data-mode]").forEach((b) => {
      const lock = !!p && b.dataset.mode !== "view";
      b.disabled = lock; b.title = lock ? "Selecting works on the real geometry: this view is reduced. Use Full res, or lower the proxy level." : b.title;
    });
    if (p && pvSelection.mode !== "view") pv.setMode("view");
  }
  function blockedByReduced() {
    if (!pvReduced) return false;
    pvMsg("Selecting needs the real geometry: this view is reduced. Click Full res, or lower the proxy level.");
    clearTimeout(blockedByReduced.t); blockedByReduced.t = setTimeout(() => pvMsg(""), 3500);
    return true;
  }
  $("#pv-full").onclick = () => { pvFull = !pvFull; pvKey = null; schedulePreview(); setReducedView(pvReduced); };
  let pvInFlight = false, pvDirty = false;
  function schedulePreview() {
    if (!pv || !pv.ok) return;
    // A trailing debounce starves the viewport throughout a continuous gizmo drag.
    // Keep the first deadline, and at most one geometry request in flight. Changes
    // during a slow cook collapse into one request for the latest state afterwards.
    pvDirty = true;
    if (pvTimer === null && !pvInFlight) pvTimer = setTimeout(runPreview, PB.playing ? 0 : 33);
  }
  function flagTitle(name){return {display:'Display (R)',render:'Render (T; toggle off to follow Display)',template:'Template (E; Shift: add; Ctrl: selectable)',selectable_template:'Selectable Template (W)',bypass:'Bypass / Ignore (Q or B)',lock:'Lock / Freeze geometry',unload:'Unload cached geometry'}[name];}
  function flagState(id,name){const n=nodeOf(id);return name==='display'?id===displayNode():name==='render'?id===(S.graph.render||displayNode()):name==='lock'?!!n?.locked_geometry:!!n?.flags?.[name];}
  async function setNodeFlag(id,name,additive=false){
    if(!name)return;
    const ids=['display','render','template','selectable_template'].includes(name)?[id]:(S.sel.has(id)?[...S.sel]:[id]);
    const value=name==='display'?true:name==='render'?S.graph.render!==id:!flagState(id,name);
    const result=await edit(ids.map(id=>({op:'flag',id,flag:name,value,additive})));
    if(result&&name==='display'){$('#pv-follow').value='display';pvKey=null;schedulePreview();}
  }
  $('#pv-follow').onchange=()=>{pvKey=null;schedulePreview();};
  function previewTarget() {
    if($('#pv-follow')?.value==='display')return displayNode();
    const ids = [...S.sel].filter((id) => nodeOf(id));
    return ids.length === 1 ? ids[0] : displayNode();
  }
  async function runPreview() {
    pvTimer = null; pvDirty = false; pvInFlight = true;
    try { await loadPreview(); }
    finally {
      pvInFlight = false;
      if (pvDirty) schedulePreview();
    }
  }
  async function loadPreview() {
    const node = previewTarget(), documentId = S.documentId;
    $("#pv-node").textContent = node ? "· " + node : "";
    if (!node) { pv.clear(); pvMsg(""); pvKey = null; return; }
    const key = `${documentId}:${node}:${S.rev}:${S.status ? S.status.cooked_rev : ""}:${pvFull}`;
    if (key === pvKey) return;
    pvKey = key;
    let buf;
    try {
      const r = await fetch("/api/preview", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ node, full: pvFull, templates:true }) });
      buf = await r.arrayBuffer();
    } catch (e) { pvKey = null; return; }
    // A completed frame of the same node is useful even if a newer edit arrived.
    // Never install geometry from the previous selection or document, though.
    if (documentId !== S.documentId || node !== previewTarget()) { pvKey = null; return; }
    const { head, arrays } = window.MiniHouPreview.parse(buf);
    if (!arrays) {
      pv.clear();
      pvMsg(head.error ? (head.error.node && head.error.node !== node ? head.error.node + ": " : "") + head.error.message : "No preview");
      return;
    }
    const empty = !head.points;
    pvMsg(empty ? "No geometry" : head.truncated ? "Showing part of a very large geometry." : "");
    setReducedView(head.proxy && head.proxy.active ? head.proxy : null);
    pv.set(head, arrays, pvNode === node && pvPath === S.path);
    pv.selectableTemplates=head.selectable_templates||[];
    if(head.template_warnings?.length)pvMsg(head.template_warnings.join('\n'));
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
    const s = pvSelection;
    let node = pvNode;
    if (!node || !nodeOf(node)) { toast("Nothing to attach the new node to."); return; }
    const params = a.params(s);
    if ((params.group || params.pattern || params.edges || "").length > 30000) { toast("That selection is too large to write into a node; select fewer elements."); return; }
    if(pv.selectableTemplates?.length){const merged=await edit([{op:'merge_templates',id:node}]);if(!merged)return;node=merged.results[0].id;pv.selectableTemplates=[];}
    const r = await edit([{ op: "insert", after: node, type: a.type, params }]);
    if (r) { pv.clearSelection(); select([r.results[0].id]); }
  }
  function renderSelectionTools(s) {
    pvSelection = s;
    updateViewportHints();
    const n = (s[SEL_KEY[s.mode]] || []).length;
    $("#pv-selinfo").textContent = s.mode === "view" ? "" : s.mode === "piece" ? `${n} face${n === 1 ? "" : "s"} in the selected objects` : `${n} ${SEL_KEY[s.mode].replace(/s$/, "")}${n === 1 ? "" : "s"} selected`;
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
    const c = info.center, hs = [], reach = Math.max(info.radius * 1.15, 0.05), axes=pv.orientationFrame(), axis_rotation=MiniInteraction.euler(axes);
    for (let i = 0; i < 3; i++) {
      const a = "xyz"[i];
      hs.push({ id: `translate_${a}`, label: `move ${a}`, param: "translate", comp: i, origin: c, dir: axes[i], axis_rotation, scale: 1, kind: "arrow", tool: "move", pending: true });
      hs.push({ id: `rotate_${a}`, label: `rotate ${a}`, param: "rotate", comp: i, origin: c, dir: axes[i], axis_rotation, scale: 180 / Math.PI, kind: "ring", radius: reach, tool: "rotate", pending: true });
      hs.push({ id: `scale_${a}`, label: `scale ${a}`, param: "scale", comp: i, origin: c, dir: axes[i], axis_rotation, scale: 1, kind: "scale", tool: "scale", pending: true });
    }
    hs.push({ id: "uniform_scale", label: "uniform scale", param: "uniform_scale", comp: null, origin: c, dir: axes[2], axis_rotation, scale: 1, kind: "uniform", tool: "scale", pending: true });
    hs.push({id:'translate_plane',label:'move in view plane',param:'translate',comp:null,origin:c,dir:axes[2],axis_rotation,scale:1,kind:'plane',tool:'move',pending:true});
    return hs;
  }
  function refreshSelectionHandles() {
    if (!pv || !pv.ok) return;
    const n=nodeOf(pvNode),p=n?.params||{},M=MiniInteraction;
    const local=n?.type==='transform'?M.frame(p.rotate).map(v=>M.world(M.frame(p.axis_rotation),v)):M.identity();
    pv.interaction.local=M.euler(local);
    const info = pvSelection.mode === "view" ? null : pv.selectionInfo();
    pv.setSelectionHandles(info ? selectionGizmo(info) : []);
    updateToolButtons();
  }
  function updateToolButtons() {
    const has = pv.hasToolHandles();
    $("#pv-xform").classList.toggle("hidden", !has);
    document.querySelectorAll("#pv-xform [data-tool]").forEach((b) => b.classList.toggle("on", b.dataset.tool === pv.tool));
  }
  function updateViewportContext() {
    clearTimeout(updateViewportContext.timer);
    // Inserting a transform during a gizmo drag must not reveal a toolbar
    // that resizes the canvas and shifts the pointer's snap targets.
    if (document.body.classList.contains('pv-dragging')) {
      updateViewportContext.timer = setTimeout(updateViewportContext, 50);
      return;
    }
    const panel = $("#pv-context");
    const ids = [...S.sel].filter((id) => nodeOf(id));
    const id = ids.length === 1 ? ids[0] : "";
    const key = id ? id + ":" + nodeOf(id).type : "";
    if (panel.dataset.node !== key) {
      panel.dataset.node = key;
      const params = id ? typeOf(id).params.filter((p) => ["float", "int", "enum", "bool"].includes(p.kind) && p.ui !== "outgroup").slice(0, 4) : [];
      panel.replaceChildren(...params.map((p) => paramRow(id, p)));
      panel.classList.toggle("hidden", !params.length);
    }
    syncParams(id, panel);
  }
  function updateViewportHints() {
    const state = $("#pv-state");
    if (!state) return;
    const id = [...S.sel].find((id) => nodeOf(id)), node = id && nodeOf(id);
    const mode = pvSelection.mode;
    const name = mode === "view" ? (node ? node.type + " · " + id : "View") : "Select " + ({ point: "points", edge: "edges", face: "faces", piece: "objects" }[mode] || mode);
    state.textContent = name;
    $("#pv-hint-title").textContent = name;
    $("#pv-hint-text").textContent = node?.type==='bezier'
      ? "Drag anchors / tangents: edit curve · X/Y/Z: constrain\nShift: precision · Ctrl: snap · Esc: cancel\nShift-double-click: append · Handle modes in inspector"
      : mode === "view"
      ? "Drag: orbit · Middle/right drag: pan\nWheel: dolly · Double-click: frame all\nC: modeling tools · 1–4: component selection"
      : "Click / drag box: select · Shift: add · Ctrl: remove\nAlt-drag / middle drag: orbit · Right drag: pan\nW / E / R: move / rotate / scale · C: modeling tools";
  }
  function setTool(t) { pv.setTool(t); updateToolButtons(); updateViewportHints(); }
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
  async function pieTool(item, note, opts) {
    if (!item) { if (note) toast(note); return; }
    const t = S.types[item.type];
    if (!t) { toast(`This engine has no ${item.type} node.`); return; }
    const params = { ...(item.params || {}), ...pieSelectionParams(item) };
    if (item.center && pv && pv.ok && pvSelection.mode !== "view" && (pvSelection[SEL_KEY[pvSelection.mode]] || []).length) {
      const info = pv.selectionInfo();
      if (info) params[item.center] = info.center.map((v) => Math.round(v * 1e5) / 1e5);
    }
    const one = S.sel.size === 1 ? [...S.sel][0] : null;
    const target = opts && opts.floating ? (one && nodeOf(one) ? one : null)                  // the shelf: a tool with nothing selected floats
      : pieFromView && pvNode && nodeOf(pvNode) ? pvNode : one && nodeOf(one) ? one : pvNode && nodeOf(pvNode) ? pvNode : displayNode();
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
  // ---- the tool shelf (shelf.js): the same tools as the pie, one click or drag away
  async function addToolAt(tool, clientX, clientY) {
    const t = S.types[tool.type];
    if (!t) { toast(`This engine has no ${tool.type} node.`); return; }
    const id = uniqueId(tool.type), [x, y] = toWorld(clientX, clientY);
    const r = await edit([{ op: "add", type: tool.type, id, pos: [Math.round(x - W / 2), Math.round(y - 12)] }]);
    if (r) select([id]);
  }
  if (window.MiniShelf) {
    MiniShelf.init({ types: () => S.types, typeList: () => S.typeList,
      onPick: (tool, item) => { pieFromView = false; pieTool(item || { label: tool.label, type: tool.type }, null, { floating: true }); } });
    canvas.addEventListener("dragover", (e) => { if ([...(e.dataTransfer.types || [])].includes("application/x-minihou-tool")) { e.preventDefault(); e.dataTransfer.dropEffect = "copy"; } });
    canvas.addEventListener("drop", (e) => {
      const raw = e.dataTransfer.getData("application/x-minihou-tool");
      if (!raw) return;
      e.preventDefault();
      try { addToolAt(JSON.parse(raw), e.clientX, e.clientY); } catch (err) { /* not ours */ }
    });
  }
  if (window.MiniPie) {
    MiniPie.init({ types: () => S.types, onPick: pieTool, onOpen: () => { pieFromView = pvOver; },
      blocked: () => !$("#tabmenu").classList.contains("hidden") || !$("#help").classList.contains("hidden") });
  }
  function wireSelectionTools() {
    const controls=[['#pv-orientation','orientation'],['#pv-snap','snap'],['#pv-snap-element','element'],['#pv-snap-target','target'],['#pv-move-step','moveStep'],['#pv-angle-step','angleStep'],['#pv-scale-step','scaleStep'],['#pv-snap-backface','backface'],['#pv-snap-selected','includeSelected']];
    try {const saved=JSON.parse(localStorage.getItem('minihou.interaction')||'{}');for(const [,key]of controls)if(typeof saved[key]===typeof pv.interaction[key])pv.interaction[key]=saved[key];if(saved.cursor?.length===3&&saved.cursor.every(Number.isFinite))pv.interaction.cursor=saved.cursor;}catch{}
    try {const saved=JSON.parse(localStorage.getItem('minihou.interaction')||'{}');if(saved.custom?.length===3&&saved.custom.every(v=>v.length===3&&v.every(Number.isFinite)))pv.interaction.custom=MiniInteraction.normalFrame(saved.custom[2],saved.custom[0]);}catch{}
    for(const key of ['moveStep','angleStep','scaleStep'])if(!Number.isFinite(pv.interaction[key])||pv.interaction[key]<=0)pv.interaction[key]=key==='angleStep'?15:0.1;
    const store=()=>{localStorage.setItem('minihou.interaction',JSON.stringify(pv.interaction));refreshSelectionHandles();pv.redraw();};
    for(const [selector,key] of controls){const e=$(selector);if(e.type==='checkbox')e.checked=pv.interaction[key];else e.value=pv.interaction[key];e.onchange=()=>{let v=e.type==='checkbox'?e.checked:e.type==='number'?Number(e.value):e.value;if(e.type==='number'&&(!Number.isFinite(v)||v<=0)){e.value=pv.interaction[key];return;}pv.interaction[key]=v;store();};}
    document.querySelectorAll('.pv-cursor-angle').forEach(e=>{const i=Number(e.dataset.axis);e.value=pv.interaction.cursor[i];e.onchange=()=>{if(Number.isFinite(Number(e.value))){pv.interaction.cursor[i]=Number(e.value);store();}};});
    $('#pv-custom-frame').onclick=()=>{pv.interaction.orientation='normal';pv.interaction.custom=pv.orientationFrame();pv.interaction.orientation='custom';$('#pv-orientation').value='custom';store();};
    pv.onSelect = renderSelectionTools;
    // viewport handles: arrows in the view that scrub a parameter of the viewed node
    let handleBase = null, gizmoNode = null, createdNode=null, originalGraph=null, originalNode=null, createdTemplateMerge=null;
    const baseFor = (node, h) => {                                                       // the parameter's current value (or default) on that node
      const n = nodeOf(node), spec = ((S.types[(n || {}).type] || {}).params || []).find((p) => p.name === h.param);
      const cur = n && n.params && n.params[h.param] !== undefined ? n.params[h.param] : spec ? spec.default : undefined;
      handleBase = Array.isArray(cur) ? cur.slice() : null;               // the other components of a vector stay as they were
      return h.kind==='plane'?handleBase:h.comp === null || h.comp === undefined ? Number(cur) : Number(Array.isArray(cur) ? cur[h.comp] : NaN);
    };
    async function startSelectionDrag(h) {                                // the first drag on a selection's gizmo creates the transform node
      const info = pv.selectionInfo();
      if (!info || !pvNode || !nodeOf(pvNode)) return NaN;
      const pattern = rangePattern(info.points);
      if(pv.selectableTemplates?.length){const merged=await edit([{op:'merge_templates',id:pvNode}]);if(!merged)return NaN;pvNode=merged.results[0].id;createdTemplateMerge=pvNode;pv.selectableTemplates=[];}
      if (pattern.length > 30000) { toast("That selection is too large to write into a node; select fewer elements."); return NaN; }
      const cur = nodeOf(pvNode);
      if (h.tool==='move'&&cur.type === "transform" && (cur.params || {}).group === pattern && JSON.stringify(cur.params.axis_rotation||[0,0,0])===JSON.stringify(h.axis_rotation||[0,0,0])) gizmoNode = pvNode;
      else {
        const pivot = info.center.map((v) => Math.round(v * 1e5) / 1e5);
        const r = await edit([{ op: "insert", after: pvNode, type: "transform", params: { group: pattern, pivot, axis_rotation:h.axis_rotation||[0,0,0] } }]);
        if (!r) return NaN;
        gizmoNode = r.results[0].id;
        createdNode=gizmoNode;
        select([gizmoNode]);
      }
      return baseFor(gizmoNode, h);
    }
    pv.onHandleStart = (h) => {createdTemplateMerge=null;createdNode=null;originalGraph=structuredClone(S.graph);originalNode=pvNode;return h.pending ? startSelectionDrag(h) : baseFor(pvNode, h);};
    pv.onHandleAxis = h => {
      const node=h.pending?gizmoNode:originalNode,n=originalGraph.nodes[node],spec=typeOf(h.pending?gizmoNode:originalNode).params.find(p=>p.name===h.param),cur=n?.params?.[h.param]??spec?.default;
      handleBase=Array.isArray(cur)?cur.slice():null;return Number(handleBase?handleBase[h.comp]:cur);
    };
    pv.onHandleCancel = async (h,base) => {
      const created=createdNode,graph=originalGraph,previous=originalNode;
      if(Number.isFinite(base)||Array.isArray(base))pv.onHandle(h,base,true);
      while(setInFlight||pendingSet.size)await new Promise(resolve=>setTimeout(resolve,20));
      if(created&&nodeOf(created)){
        const ops=[{op:'remove',id:created}];
        if(createdTemplateMerge){ops.push({op:'remove',id:createdTemplateMerge});for(const [id,n]of Object.entries(graph.nodes))if(n.flags)ops.push({op:'meta',id,set:{flags:n.flags}});}
        for(const [id,n] of Object.entries(graph.nodes))for(const [port,ref] of Object.entries(n.inputs||{})){
          const refs=Array.isArray(ref)?ref:[ref];if(!refs.includes(previous))continue;
          ops.push({op:'disconnect',to:id,port});for(const from of refs)ops.push({op:'connect',from,to:id,port});
        }
        if(graph.display)ops.push({op:'display',id:graph.display});
        await edit(ops);select([previous]);
      }
    };
    pv.onHandle = (h, value) => {
      const node = h.pending ? gizmoNode : pvNode;
      if (!node || !nodeOf(node)) return;
      const spec = ((S.types[nodeOf(node).type] || {}).params || []).find((p) => p.name === h.param);
      if (spec && spec.kind === "int") value = Math.round(value);                       // an integer parameter takes whole steps
      if (h.comp === null || h.comp === undefined) queueSet(node, h.param, value);
      else if (handleBase) { const v = handleBase.slice(); v[h.comp] = value; queueSet(node, h.param, v); }
    };
    document.querySelectorAll("#preview [data-mode]").forEach((b) => { b.onclick = () => { if (b.dataset.mode !== "view" && blockedByReduced()) return; pv.setMode(b.dataset.mode); }; });
    const wrap = $("#pv-canvas").parentElement;                         // the canvas and the overlays on it (gizmo, handles) count as one
    wrap.addEventListener("pointerenter", () => { pvOver = true; });
    wrap.addEventListener("pointerleave", () => { pvOver = false; });
    document.addEventListener("keydown", (e) => {
      const tag = (e.target.tagName || "").toLowerCase();
      if(pv.isDragging()||curveTool?.active())return;
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
      if (mode) { if (!blockedByReduced()) pv.setMode(mode); e.preventDefault(); e.stopImmediatePropagation(); return; }
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
  // Browser-local viewport preferences: only validated, known fields are restored.
  const preferenceDefaults = { background: "#212429", gridOpacity: 1, grid: true, axes: true,
    shaded: true, edges: true, points: false, normals: false,
    nodeBackdrop: "grid", nodeBackground: "#26292d", nodeGrid: "#2d3035", nodeColor: "#3a3f46",
    wireColor: "#a9b1ba", selectionColor: "#e8a33d", nodeSnap: false, nodeSnapStep: 24, frameColor: "#5b7fb5", lighting: 1, pointSize: 4, gridSpacing: 0 };
  function validPreference(key, value) {
    const ranges = { lighting: [0, 2], pointSize: [1, 20], gridSpacing: [0, 10000], gridOpacity: [0, 2], nodeSnapStep: [1, 200] };
    if (ranges[key]) return Number.isFinite(value) && value >= ranges[key][0] && value <= ranges[key][1] && (key !== "nodeSnapStep" || Number.isInteger(value));
    if (key === "nodeBackdrop") return ["grid", "checker", "solid"].includes(value);
    if (/background|color|nodeGrid/i.test(key)) return typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value);
    return typeof value === "boolean";
  }
  const preferencesKey = "minihou.preferences.v1";
  const preferences = { ...preferenceDefaults };
  try {
    const saved = JSON.parse(localStorage.getItem(preferencesKey) || "{}");
    for (const key of Object.keys(preferences)) {
      const value = saved?.[key];
      if (validPreference(key, value)) preferences[key] = value;
    }
  } catch (e) { /* Keep defaults when storage is unavailable or malformed. */ }
  function applyPreferences(save = false) {
    if (pv && pv.ok) { Object.assign(pv.opt, preferences); pv.redraw(); }
    for (const [key, css] of [["nodeBackground", "--canvas"], ["nodeGrid", "--grid"], ["nodeColor", "--node"], ["wireColor", "--wire"], ["selectionColor", "--node-hi"]]) canvas.style.setProperty(css, preferences[key]);
    canvas.dataset.backdrop = preferences.nodeBackdrop;
    canvas.dataset.gridSpacing = preferences.nodeSnapStep;
    applyView();
    for (const key of Object.keys(preferences)) {
      const input = $("#pref-" + key);
      if (input.type === "checkbox") input.checked = preferences[key]; else input.value = preferences[key];
      const quick = $("#pv-" + key); if (quick) quick.checked = preferences[key];
    }
    $("#pref-display-mode").value = !preferences.shaded ? (preferences.points && !preferences.edges ? "points" : "wire") : preferences.edges ? "edges" : "shaded";
    $("#network-snap").checked = preferences.nodeSnap;
    $("#pv-display-mode").value = $("#pref-display-mode").value;
    if (save) {
      try { localStorage.setItem(preferencesKey, JSON.stringify(preferences)); $("#preferences-storage").textContent = "Saved in this browser."; }
      catch (e) { $("#preferences-storage").textContent = "Browser storage is unavailable. Changes apply for this session."; }
    }
  }
  applyPreferences();
  for (const key of Object.keys(preferences)) $("#pref-" + key).oninput = e => {
    const value = e.target.type === "checkbox" ? e.target.checked : ["gridOpacity", "nodeSnapStep", "lighting", "pointSize", "gridSpacing"].includes(key) ? Number(e.target.value) : e.target.value;
    if (!validPreference(key, value)) return;
    preferences[key] = value;
    applyPreferences(true);
  };
  $("#network-snap").onchange = e => { preferences.nodeSnap = e.target.checked; applyPreferences(true); };
  $("#network-home").onclick = () => frame(false);
  const preferencesDialog = $("#preferences");
  function preferencePage(page) {
    document.querySelectorAll("[data-preference-tab]").forEach(b => { const active = b.dataset.preferenceTab === page; b.setAttribute("aria-selected", active); b.tabIndex = active ? 0 : -1; });
    document.querySelectorAll("[data-preference-page]").forEach(p => p.hidden = p.dataset.preferencePage !== page);
  }
  document.querySelectorAll("[data-preference-tab]").forEach(b => {
    b.onclick = () => preferencePage(b.dataset.preferenceTab);
    b.onkeydown = e => { if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) { const page = e.key === "Home" ? "viewport" : e.key === "End" ? "network" : b.dataset.preferenceTab === "viewport" ? "network" : "viewport"; preferencePage(page); document.querySelector(`[data-preference-tab="${page}"]`).focus(); e.preventDefault(); } };
  });
  $("#preferences-btn").onclick = () => { applyPreferences(); preferencesDialog.showModal(); preferencesDialog.scrollTop = 0; $("#preferences-preset").focus({ preventScroll: true }); };
  $("#preferences-reset").onclick = () => { Object.assign(preferences, preferenceDefaults); applyPreferences(true); };
  preferencesDialog.addEventListener("keydown", e => e.stopPropagation());
  document.addEventListener("keydown", e => { if (preferencesDialog.open && e.target.dataset.preferenceTab) e.target.onkeydown(e); if (preferencesDialog.open) e.stopImmediatePropagation(); }, true);
  if (pv && pv.ok) {
    for (const [id, key] of [["#pv-grid", "grid"], ["#pv-shaded", "shaded"], ["#pv-edges", "edges"], ["#pv-points", "points"], ["#pv-normals", "normals"]]) {
      $(id).checked = pv.opt[key];
      $(id).onchange = (e) => { preferences[key] = e.target.checked; applyPreferences(true); };
    }
    $("#pv-frame-all").onclick = () => pv.frameAll();
    $("#pv-view-extra").onchange = (e) => { if (e.target.value) pv.viewAxis(e.target.value, false); e.target.value = ""; };
    $("#pv-hints-toggle").onclick = (e) => {
      const hidden = $("#pv-hints").classList.toggle("hidden");
      e.currentTarget.setAttribute("aria-pressed", String(!hidden));
    };
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
      for (const [member] of inputMembers(id)) expanded.add(member);
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
      const n = nodeOf(id);
      if (n.color) ops.push({ op: "meta", id: map[id], set: { color: n.color } });
      if(n.flags||n.locked_geometry)ops.push({op:'meta',id:map[id],set:{flags:n.flags||{},locked_geometry:n.locked_geometry||null}});
      if (inputOwner(id) && map[n.input_owner]) ops.push({ op: "meta", id: map[id], set: { input_owner: map[n.input_owner], input_label: n.input_label || id } });
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
    ids = [...new Set(ids.concat(ids.flatMap(id => inputMembers(id).map(([key]) => key))))];
    const keep = new Set(ids), nodes = {};
    for (const id of ids) {
      const n = nodeOf(id), ins = {};
      for (const [spec, srcs] of inputs(id)) {
        const inner = srcs.filter((s) => keep.has(s));
        if (inner.length) ins[spec.name] = Array.isArray((n.inputs || {})[spec.name]) ? inner : inner[0];
      }
      nodes[id] = { type: n.type, params: n.params || {}, inputs: ins, pos: pos(id), flags:n.flags, locked_geometry:n.locked_geometry, comment: n.comment || undefined, input_owner: keep.has(n.input_owner) ? n.input_owner : undefined, input_label: n.input_label };
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
      const n = d.nodes[id];
      if(n.flags||n.locked_geometry)ops.push({op:'meta',id:map[id],set:{flags:n.flags||{},locked_geometry:n.locked_geometry||null}});
      if (n.type === "object_input" && map[n.input_owner]) ops.push({ op: "meta", id: map[id], set: { input_owner: map[n.input_owner], input_label: n.input_label || id } });
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
    if (b.note) return [];
    const [x, y] = b.pos, [w, hh] = b.size;
    return Object.keys(S.graph.nodes).filter((id) => { const [nx, ny] = pos(id); return nx + W / 2 >= x && nx + W / 2 <= x + w && ny + H / 2 >= y && ny + H / 2 <= y + hh; });
  }
  function hiddenInFrame(id) { return (S.graph.boxes || []).some(b => b.collapsed && !b.note && boxMembers(b).includes(id)); }
  function lockedInFrame(id) { return (S.graph.boxes || []).some(b => b.locked && !b.note && boxMembers(b).includes(id)); }
  function renderBoxes() {
    const list = S.graph.boxes || [], keep = new Set();
    for (const b of list) {
      keep.add(b.id);
      let el = boxesEl.querySelector(`.netbox[data-id="${CSS.escape(b.id)}"]`);
      if (!el) {
        el = h("div", { class: "netbox", dataset: { id: b.id } },
          h("div", { class: "hdr" }, h("input", { class: "dot", type: "color", title: "Frame colour", "aria-label": "Frame colour", oninput: e => edit([{ op: "box_set", id: b.id, color: e.target.value }]) }), h("span", { class: "title" }), h("span", { class: "x", title: "Delete the box (nodes stay)" }, "×")),
          h("textarea", { class: "note-text", "aria-label": "Sticky note text", maxlength: 10000, onchange: e => edit([{ op: "box_set", id: b.id, text: e.target.value }]) }),
          h("div", { class: "grip", title: "Resize" }));
        boxesEl.append(el);
      }
      const r = boxRect(b);
      el.style.left = r.pos[0] + "px"; el.style.top = r.pos[1] + "px";
      el.style.width = r.size[0] + "px"; el.style.height = (b.collapsed ? 28 : r.size[1]) + "px";
      el.classList.toggle("collapsed", !!b.collapsed); el.classList.toggle("sticky-note", !!b.note); el.classList.toggle("locked", !!b.locked);
      const note = el.querySelector(".note-text"); note.hidden = !b.note || !!b.collapsed; note.readOnly = !!b.locked;
      if (document.activeElement !== note) note.value = b.text || "";
      el.style.setProperty("--bc", b.color || BOX_COLORS[0]);
      el.querySelector(".dot").value = b.color || BOX_COLORS[0];
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
    if (ids.length) op = { op: "box_add", around: ids, title: "Frame", color: preferences.frameColor };
    else {
      const r = canvas.getBoundingClientRect(), [x, y] = toWorld(r.left + r.width / 2, r.top + r.height / 2);
      op = { op: "box_add", pos: [Math.round(x - 150), Math.round(y - 100)], size: [300, 200], title: "Frame", color: preferences.frameColor };
    }
    await edit([op]);
  }
  async function renameBox(id) {
    const b = boxById(id);
    if (!b) return;
    const t = await appPrompt("Box title:", b.title || "");
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
  let drag = null, suppressContextUntil=0;
  function segmentsCross(a,b,c,d){
    const cross=(p,q,r)=>(q[0]-p[0])*(r[1]-p[1])-(q[1]-p[1])*(r[0]-p[0]);
    return Math.max(a[0],b[0])>=Math.min(c[0],d[0]) && Math.max(c[0],d[0])>=Math.min(a[0],b[0]) && Math.max(a[1],b[1])>=Math.min(c[1],d[1]) && Math.max(c[1],d[1])>=Math.min(a[1],b[1]) && cross(a,b,c)*cross(a,b,d)<=0 && cross(c,d,a)*cross(c,d,b)<=0;
  }
  function cancelNetworkDrag(){if(!drag)return;drag=null;S.dragPos.clear();S.boxDrag=null;overlay.replaceChildren();$('#boxsel').classList.add('hidden');canvas.classList.remove('panning','wiring','detaching');window.removeEventListener('pointermove',onDrag);window.removeEventListener('pointerup',onDrop);renderNodes();renderBoxes();}
  window.addEventListener('pointercancel',cancelNetworkDrag);
  window.addEventListener('blur',cancelNetworkDrag);

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
      if (t.closest(".note-text") || b?.locked) return;
      if (b && t.closest(".dot")) return;
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
    if (e.button === 2 && e.shiftKey) {
      drag={kind:"cut",points:[[e.clientX,e.clientY]],cuts:new Map()};suppressContextUntil=performance.now()+1000;
    } else if (e.button === 1 || (e.button === 0 && e.altKey && !nodeEl)) {
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
      const name=e.ctrlKey&&t.dataset.flag==='template'?'selectable_template':t.dataset.flag;
      setNodeFlag(nodeEl.dataset.id,name,e.shiftKey);
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
      if (lockedInFrame(id)) return;
      drag = { kind: "move", ids: [...S.sel].filter(i => !lockedInFrame(i)), wx, wy, start: new Map([...S.sel].map((i) => [i, pos(i).slice()])), moved: false };
    } else if (t.classList.contains("hit")) {
      S.wireSel = { to: t.dataset.to, port: t.dataset.port, from: t.dataset.from };
      S.sel.clear(); renderNodes(); renderParams();
      return;
    } else {
      drag = { kind: "box", x0: e.clientX, y0: e.clientY, additive:e.shiftKey };
    }
    if (drag) {
      e.preventDefault();
      canvas.classList.toggle("wiring", drag.kind === "wire" || drag.kind === "wire-rev");
      window.addEventListener("pointermove", onDrag); window.addEventListener("pointerup", onDrop);
    }
  });
  function nodeDragDelta(dx, dy, anchor) {
    if (!preferences.nodeSnap) return [dx, dy];
    const step = preferences.nodeSnapStep;
    return [Math.round((anchor[0] + dx) / step) * step - anchor[0], Math.round((anchor[1] + dy) / step) * step - anchor[1]];
  }
  function onDrag(e) {
    S.mouse = [e.clientX, e.clientY];
    if (!drag) return;
    if (drag.kind === "cut") {
      const next=[e.clientX,e.clientY], prev=drag.points.at(-1);
      for(const path of canvas.querySelectorAll("path.hit")){
        const matrix=path.getScreenCTM(), len=path.getTotalLength();let a=path.getPointAtLength(0).matrixTransform(matrix);
        for(let t=4;t<len+4;t+=4){const b=path.getPointAtLength(Math.min(t,len)).matrixTransform(matrix);
          if(segmentsCross(prev,next,[a.x,a.y],[b.x,b.y])){const w={to:path.dataset.to,port:path.dataset.port,from:path.dataset.from};drag.cuts.set(JSON.stringify(w),w);path.classList.add("cut-mark");break;}a=b;}}
      drag.points.push(next);const r=canvas.getBoundingClientRect();overlay.replaceChildren(svgEl("path",{d:drag.points.map((p,i)=>`${i?"L":"M"}${p[0]-r.left},${p[1]-r.top}`).join(" ")}));
    } else if (drag.kind === "pan") {
      const dx = e.clientX - drag.x0, dy = e.clientY - drag.y0;
      if (Math.abs(dx) + Math.abs(dy) > 2) drag.moved = true;
      S.view.x = drag.vx + dx; S.view.y = drag.vy + dy; applyView();
    } else if (drag.kind === "move") {
      const [wx, wy] = toWorld(e.clientX, e.clientY), [dx, dy] = nodeDragDelta(wx - drag.wx, wy - drag.wy, drag.kind === "move" ? drag.start.get(drag.ids[0]) : drag.kind === "boxresize" ? drag.size : drag.pos);
      if (Math.abs(dx) + Math.abs(dy) > 1) drag.moved = true;
      for (const id of drag.ids) { const [x, y] = drag.start.get(id); S.dragPos.set(id, [Math.round(x + dx), Math.round(y + dy)]); }
      for (const id of drag.ids) { const el = nodesEl.querySelector(`.node[data-id="${CSS.escape(id)}"]`); if (el) { const [x, y] = pos(id); el.style.left = x + "px"; el.style.top = y + "px"; } }
      const now=performance.now(), q=drag.shake ||= {x:e.clientX,dir:0,turns:0,time:now};
      const delta=e.clientX-q.x;
      if(Math.abs(delta)>=18){const dir=Math.sign(delta);if(now-q.time>650)q.turns=0;
        if(q.dir && dir!==q.dir){q.turns++;q.time=now;}q.dir=dir;q.x=e.clientX;
        if(q.turns>=3 && !drag.detached){drag.detached=true;canvas.classList.add("detaching");toast("Disconnect on release; Esc cancels");}}
      renderWires();
    } else if (drag.kind === "boxmove" || drag.kind === "boxresize") {
      const [wx, wy] = toWorld(e.clientX, e.clientY), [dx, dy] = nodeDragDelta(wx - drag.wx, wy - drag.wy, drag.kind === "move" ? drag.start.get(drag.ids[0]) : drag.kind === "boxresize" ? drag.size : drag.pos);
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
    canvas.classList.remove("panning", "wiring", "detaching");
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
    if (d.kind === "cut") {
      suppressContextUntil=performance.now()+500;
      if(d.cuts.size)await edit([...d.cuts.values()].map(w=>({op:"disconnect",...w})));
      renderWires();
    } else if (d.kind === "pan") {
      if (!d.moved && e.button === 0) { S.sel.clear(); S.wireSel = null; renderNodes(); renderParams(); scheduleInspect(); }
    } else if (d.kind === "move") {
      if (d.moved) {
        const positions = {};
        for (const id of d.ids) positions[id] = pos(id);
        const ops=[{op:"move",positions}];
        if(d.detached){const ids=new Set(d.ids);for(const [to,n] of Object.entries(S.graph.nodes))for(const [port,ref] of Object.entries(n.inputs||{}))for(const from of [ref].flat())if(ids.has(to)!==ids.has(from))ops.push({op:"disconnect",to,port,from});}
        const r = await edit(ops);
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
      const hits = Object.keys(S.graph.nodes).filter((id) => { const [x, y] = pos(id); return x + W > ax && x < bx && y + nodeHeight(id) > ay && y < by; });
      select(hits, d.additive);
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
  canvas.addEventListener("contextmenu", graphContextMenu);
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
    if (e.key === "Escape") {
      cancelNetworkDrag();
      api("/api/cancel", {}).catch((err) => toast(err.message));
    }
  }, true);
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
    if(!pvOver&&!e.ctrlKey&&!e.metaKey&&!e.altKey&&S.sel.size){const flag={q:'bypass',b:'bypass',e:'template',w:'selectable_template',r:'display'}[e.key.toLowerCase()];if(flag){e.preventDefault();setNodeFlag([...S.sel][0],flag,e.shiftKey);return;}}
    const mod = e.ctrlKey || e.metaKey;
    if (e.key === "Escape") { $("#help").classList.add("hidden"); $("#file-menu").classList.add("hidden"); }
    else if (e.key === "Tab") {
      e.preventDefault();
      const one = S.sel.size === 1 ? [...S.sel][0] : null;
      openMenu(S.mouse[0], S.mouse[1], one ? { fromSel: one } : { world: toWorld(...S.mouse) });
    } else if (mod && e.key.toLowerCase() === "z") { e.preventDefault(); undo(e.shiftKey); }
    else if (mod && e.key.toLowerCase() === "y") { e.preventDefault(); undo(true); }
    else if (mod && e.key.toLowerCase() === "d") { e.preventDefault(); if (S.sel.size) duplicate([...S.sel]); }
    else if (mod && e.key.toLowerCase() === "g") { e.preventDefault(); saveSelectionAsNode(); }                // Houdini's collapse to subnet
    else if (mod && e.key.toLowerCase() === "a") { e.preventDefault(); select(Object.keys(S.graph.nodes)); }
    else if (e.key === "Delete" || e.key === "Backspace" || (!pvOver && !mod && e.key.toLowerCase() === "x")) { e.preventDefault(); deleteSelection(); }
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
  // ---- the graph & library browser: search, recent, folders, library nodes, Save As, insert, duplicate, rename, delete
  const FM = { files: [], q: "", active: 0, rows: [], libDir: "" };
  const recentFiles = () => { try { return JSON.parse(localStorage.getItem("minihou.recentfiles") || "[]").map(relPath); } catch (e) { return []; } };
  function rememberFile(path) {
    if (!path || !S.root) return;
    try { localStorage.setItem("minihou.recentfiles", JSON.stringify([path, ...recentFiles().filter((p) => p !== path)].slice(0, 8))); } catch (e) { /* storage unavailable */ }
  }
  function ago(sec) {
    const d = Date.now() / 1000 - sec;
    if (d < 60) return "just now";
    if (d < 3600) return Math.floor(d / 60) + " min ago";
    if (d < 86400) return Math.floor(d / 3600) + " h ago";
    if (d < 86400 * 60) return Math.floor(d / 86400) + " d ago";
    return new Date(sec * 1000).toLocaleDateString();
  }
  const fileMatches = (f, toks) => toks.every((t) => (f.path + " " + (f.type || "") + " " + (f.doc || "")).toLowerCase().includes(t));
  async function openFileMenu() {
    const menu = $("#file-menu");
    try { const d = await api("/api/library"); FM.files = d.files; FM.libDir = d.library_dir; } catch (err) { toast(err.message); return; }
    FM.q = ""; FM.active = 0;
    menu.classList.remove("hidden");
    renderFileMenu();
    const inp = $("#fm-search");
    if (inp) inp.focus();
  }
  function fileRows() {
    const toks = NodeMenu.tokens(FM.q.replace(/[/.]/g, " "));
    const cur = S.path ? relPath(S.path) : null;
    const files = FM.files.filter((f) => !toks.length || fileMatches(f, toks));
    const rows = [];
    const section = (title, list, opts = {}) => { if (list.length) { rows.push({ head: title }); list.forEach((f) => rows.push({ file: f, ...opts })); } };
    if (!toks.length) {
      const recent = recentFiles().map((p) => FM.files.find((f) => f.path === p)).filter((f) => f && f.path !== cur && !f.system).slice(0, 5);
      section("Recent", recent);
    }
    section("Library nodes ◆", files.filter((f) => f.system), { lib: true });
    const shown = new Set(rows.filter((r) => r.file && !r.lib).map((r) => r.file.path));                 // a recent file is not listed twice
    const graphs = files.filter((f) => !f.system && !shown.has(f.path)).sort((a, b) => a.path.localeCompare(b.path));
    const folders = new Map();
    for (const f of graphs) { const dir = f.path.includes("/") ? f.path.slice(0, f.path.lastIndexOf("/")) : "(root)"; if (!folders.has(dir)) folders.set(dir, []); folders.get(dir).push(f); }
    for (const [dir, list] of folders) section(toks.length ? "Graphs" : dir === "(root)" ? "Graphs" : dir + "/", list, { tree: true });
    if (toks.length) {                                                         // one heading when searching
      const seen = new Set();
      return rows.filter((r) => !r.head || !seen.has(r.head) && seen.add(r.head));
    }
    return rows;
  }
  function renderFileMenu() {
    const menu = $("#file-menu");
    const cur = S.path ? relPath(S.path) : null;
    const curFile = FM.files.find((f) => f.path === cur);
    const rows = fileRows();
    FM.rows = rows.filter((r) => r.file);
    if (FM.active >= FM.rows.length) FM.active = Math.max(0, FM.rows.length - 1);
    let n = 0;
    const list = rows.map((r) => {
      if (r.head) return h("div", { class: "cat" }, r.head);
      const f = r.file, i = n++, name = f.path.split("/").pop().replace(/\.json$/, "");
      const actions = h("span", { class: "acts" },
        r.lib && h("button", { title: "Add this node to the graph", onclick: (e) => { e.stopPropagation(); addLibraryNode(f); } }, "+"),
        h("button", { title: "Insert this graph's nodes into the open graph", onclick: (e) => { e.stopPropagation(); insertFromFile(f.path); } }, "⤓"),
        h("button", { title: "Duplicate", onclick: (e) => { e.stopPropagation(); fileOp("duplicate", f.path); } }, "⧉"),
        !f.current && h("button", { title: "Rename", onclick: (e) => { e.stopPropagation(); renameFile(f); } }, "✎"),
        !f.current && h("button", { title: "Delete", onclick: (e) => { e.stopPropagation(); deleteFile(f); } }, "🗑"));
      return h("div", { class: "item frow" + (f.path === cur ? " cur" : "") + (i === FM.active ? " active" : "") + (f.error ? " bad" : ""), dataset: { i, path: f.path },
        title: f.path + (f.doc ? "\n" + f.doc : "") + (f.error ? "\n" + f.error : ""), onclick: () => openFile(f.path), onmousemove: () => { if (FM.active !== i) { FM.active = i; markActive(); } } },
        h("span", { class: "nm" }, ...NodeMenu.highlight(name, FM.q).map(([t, hit]) => (hit ? h("mark", null, t) : t)), f.system && f.type ? h("small", { class: "ty" }, " " + f.type) : ""),
        h("span", { class: "meta" }, `${f.nodes} node${f.nodes === 1 ? "" : "s"} · ${ago(f.mtime)}`), actions);
    });
    menu.replaceChildren(
      h("input", { id: "fm-search", placeholder: "Search graphs and library nodes…", value: FM.q, autocomplete: "off", spellcheck: "false",
        oninput: (e) => { FM.q = e.target.value; FM.active = 0; renderFileMenu(); $("#fm-search").focus(); } }),
      h("div", { class: "fm-cur" }, h("div", null, h("b", null, cur || "(unsaved graph)"), h("small", null, curFile ? ` · ${curFile.nodes} nodes · edits save as you make them` : "")),
        h("span", { class: "acts on" },
          h("button", { onclick: saveAs, title: "Write this graph to a new file and switch to it" }, "Save As…"),
          h("button", { onclick: exportUsd, title: "Write the cooked geometry (full size) to a USD file for Houdini, Blender, Omniverse or usdview" }, "Export USD…"),
          h("button", { onclick: saveSelectionAsNode, disabled: !S.sel.size, title: "Turn the selected nodes into a reusable library node (Ctrl+G)" }, "Save selection as node…"))),
      h("div", { class: "fm-list" }, ...(list.length ? list : [h("div", { class: "cat" }, FM.q ? "No graph or library node matches “" + FM.q + "”" : "No graph files here yet")])),
      h("div", { class: "fm-foot" }, h("button", { onclick: newFile }, "New graph…"),
        h("small", null, "Library nodes live in " + (FM.libDir ? FM.libDir.split(/[\\/]/).slice(-2).join("/") : "nodes") + " and appear in the Add Node menu under Library.")));
  }
  function markActive() {
    document.querySelectorAll("#file-menu .frow").forEach((el) => el.classList.toggle("active", Number(el.dataset.i) === FM.active));
    const el = document.querySelector(`#file-menu .frow[data-i="${FM.active}"]`);
    if (el) el.scrollIntoView({ block: "nearest" });
  }
  // ---------------------------------------------------------------- proxy: a lighter version of a very large scene
  const PROXY_ROWS = [
    { id: "full", label: "Full", level: 1, auto: false, hint: "the real thing" },
    { id: "auto", label: "Auto", level: 0.1, auto: true, hint: "slow scenes: small first, full when you pause" },
    { id: "0.5", label: "50%", level: 0.5, auto: false, hint: "about half the points" },
    { id: "0.25", label: "25%", level: 0.25, auto: false, hint: "" },
    { id: "0.1", label: "10%", level: 0.1, auto: false, hint: "" },
    { id: "0.05", label: "5%", level: 0.05, auto: false, hint: "" },
    { id: "0.01", label: "1%", level: 0.01, auto: false, hint: "a sketch" },
  ];
  const LINK_CAPS = [[0, "everything"], [100000, "100 000 triangles"], [500000, "500 000 triangles"], [2000000, "2 million triangles"]];
  const proxyRow = () => {
    const p = (S.status && S.status.proxy) || { level: 1, auto: false };
    if (p.auto) return PROXY_ROWS[1];
    return PROXY_ROWS.find((r) => !r.auto && Math.abs(r.level - p.level) < 1e-9) || { id: "custom", label: pct(p.level), level: p.level, auto: false };
  };
  function renderProxy() {
    const p = (S.status && S.status.proxy) || { level: 1, auto: false, link_tris: 0 };
    const row = proxyRow();
    $("#proxy-label").textContent = row.label;
    $("#proxy-btn").classList.toggle("on", row.id !== "full" || p.link_tris > 0 || !!(S.status && S.status.disk_cache));
    $("#proxy-btn").title = row.id === "full" ? "Proxy: work on a lighter version of a very large scene" : `Proxy ${row.label}: nodes cook a smaller version of the scene (counts, resolutions, voxel sizes scale)`;
    if (!$("#proxy-menu").classList.contains("hidden")) fillProxyMenu();
  }
  function fillProxyMenu() {
    const p = (S.status && S.status.proxy) || { level: 1, auto: false, link_tris: 0 }, cur = proxyRow().id, m = $("#proxy-menu");
    m.innerHTML = "";
    const head = document.createElement("div"); head.className = "head";
    head.textContent = "Cook a lighter version of the scene while you work: grid and scatter counts, sphere bands, voxel sizes and spacings scale down. The viewport always draws within a budget of its own.";
    m.appendChild(head);
    for (const r of PROXY_ROWS) {
      const it = document.createElement("div"); it.className = "item" + (r.id === cur ? " cur" : ""); it.dataset.proxy = r.id;
      it.innerHTML = `<span class="dot">${r.id === cur ? "●" : ""}</span><span>${r.label}</span><span class="hint">${r.hint}</span>`;
      it.onclick = async () => { try { S.status = Object.assign(S.status || {}, { proxy: await api("/api/proxy", { level: r.level, auto: r.auto }) }); } catch (e) { toast(e.message); } renderProxy(); $("#proxy-menu").classList.add("hidden"); };
      m.appendChild(it);
    }
    const sep = document.createElement("div"); sep.className = "sep"; m.appendChild(sep);
    const dc = (S.status && S.status.disk_cache) || null;
    const hd = document.createElement("div"); hd.className = "head"; hd.textContent = "Disk cache: expensive results survive a restart, so reopening a big scene is quick."; m.appendChild(hd);
    const dcRow = (label, hint, id, fn) => {
      const it = document.createElement("div"); it.className = "item"; it.dataset.diskcache = id;
      it.innerHTML = `<span class="dot"></span><span>${label}</span><span class="hint">${hint}</span>`;
      it.onclick = async () => { try { const r = await api("/api/disk_cache", fn); S.status = Object.assign(S.status || {}, { disk_cache: r.stats }); } catch (e) { toast(e.message); } renderProxy(); $("#proxy-menu").classList.add("hidden"); };
      m.appendChild(it);
    };
    if (dc) {
      dcRow("Disk cache on", `${(dc.bytes / 1e6).toFixed(0)} MB · ${dc.hits} hits`, "off", { enabled: false });
      dcRow("Clear the cache", "", "clear", { clear: true });
    } else dcRow("Turn disk cache on", "off", "on", { enabled: true });
    const sep2 = document.createElement("div"); sep2.className = "sep"; m.appendChild(sep2);
    const h2 = document.createElement("div"); h2.className = "head"; h2.textContent = "Send Blender at most:"; m.appendChild(h2);
    for (const [n, label] of LINK_CAPS) {
      const it = document.createElement("div"); it.className = "item" + ((p.link_tris || 0) === n ? " cur" : ""); it.dataset.cap = String(n);
      it.innerHTML = `<span class="dot">${(p.link_tris || 0) === n ? "●" : ""}</span><span>${label}</span>`;
      it.onclick = async () => { try { S.status = Object.assign(S.status || {}, { proxy: await api("/api/proxy", { link_tris: n }) }); } catch (e) { toast(e.message); } renderProxy(); $("#proxy-menu").classList.add("hidden"); };
      m.appendChild(it);
    }
  }
  $("#proxy-btn").onclick = (e) => {
    e.stopPropagation();
    const m = $("#proxy-menu");
    if (!m.classList.contains("hidden")) { m.classList.add("hidden"); return; }
    fillProxyMenu(); m.classList.remove("hidden");
  };
  document.addEventListener("click", (e) => { if (!e.target.closest(".proxy")) $("#proxy-menu").classList.add("hidden"); });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") $("#proxy-menu").classList.add("hidden"); }, true);
  $("#file-btn").onclick = async (e) => {
    e.stopPropagation();
    if (!$("#file-menu").classList.contains("hidden")) { $("#file-menu").classList.add("hidden"); return; }
    await openFileMenu();
  };
  $("#file-menu").addEventListener("keydown", (e) => {
    if (e.key === "Escape") { $("#file-menu").classList.add("hidden"); e.stopPropagation(); }
    else if (e.key === "ArrowDown" || e.key === "ArrowUp") { if (FM.rows.length) { FM.active = (FM.active + (e.key === "ArrowDown" ? 1 : -1) + FM.rows.length) % FM.rows.length; markActive(); } e.preventDefault(); }
    else if (e.key === "Enter" && FM.rows[FM.active]) { openFile(FM.rows[FM.active].file.path); e.preventDefault(); }
  });
  document.addEventListener("click", (e) => { if (!e.target.closest(".file")) $("#file-menu").classList.add("hidden"); });
  async function saveAs() {
    const cur = S.path ? relPath(S.path) : "graphs/my_graph.json";
    let name = await appPrompt("Save a copy as (inside " + (S.root || "the workspace") + ") and switch to it:", cur.replace(/\.json$/, "") + "_v2.json");
    if (!name) return;
    name = name.trim();
    if (!name.toLowerCase().endsWith(".json")) name += ".json";
    try { const snap = await api("/api/save_as", { path: name }); $("#file-menu").classList.add("hidden"); S.sel.clear(); applySnapshot(snap); toast("Saved as " + name, false); }
    catch (err) {
      if (/already exists/.test(err.message) && await appConfirm(name + " already exists. Overwrite it?")) {
        try { const snap = await api("/api/save_as", { path: name, overwrite: true }); $("#file-menu").classList.add("hidden"); S.sel.clear(); applySnapshot(snap); } catch (e2) { toast(e2.message); }
      } else toast(err.message);
    }
  }
  async function exportUsd() {
    const cur = S.path ? relPath(S.path) : "exports/scene.json";
    const picked = [...S.sel].filter((id) => nodeOf(id)), single = picked.length === 1 ? picked[0] : null;
    let name = await appPrompt("Export the " + (single ? "node " + single : "display output") + " as USD (inside " + (S.root || "the workspace") + ", full size, .usd .usda or .usdc):",
      cur.replace(/\.json$/i, "") + ".usdc");
    if (!name) return;
    name = name.trim();
    if (!/\.(usd|usda|usdc)$/i.test(name)) name += ".usdc";
    const send = (overwrite) => api("/api/export_usd", { path: name, node: single || undefined, overwrite });
    try {
      let r;
      try { r = await send(false); }
      catch (err) {
        if (/already exists/.test(err.message) && await appConfirm(name + " already exists. Overwrite it?")) r = await send(true); else throw err;
      }
      $("#file-menu").classList.add("hidden");
      const o = Object.values(r.outputs), n = (k) => o.reduce((a, x) => a + (x[k] || 0), 0);
      toast(`Exported ${fmt(n("faces"))} faces, ${fmt(n("curves"))} curves, ${fmt(n("points"))} loose points` + (n("instances") ? `, ${fmt(n("instances"))} instances` : "") + ` to ${name}`, false);
    } catch (err) { toast(err.message); }
  }
  async function fileOp(op, path, to) {
    try { await api("/api/file_op", { op, path, to }); await openFileMenu(); } catch (err) { toast(err.message); }
  }
  async function renameFile(f) {
    let to = await appPrompt("Rename to:", f.path);
    if (!to || to === f.path) return;
    if (!to.toLowerCase().endsWith(".json")) to += ".json";
    fileOp("rename", f.path, to.trim());
  }
  async function deleteFile(f) {
    const warn = f.system ? "\n\nIt is a library node: graphs that use it will show an error." : "";
    if (await appConfirm("Delete " + f.path + "? This cannot be undone." + warn)) fileOp("delete", f.path);
  }
  async function insertFromFile(path) {
    try {
      const d = await api("/api/file?path=" + encodeURIComponent(path));
      const nodes = (d.graph && d.graph.nodes) || {};
      if (!Object.keys(nodes).length) { toast("That graph has no nodes."); return; }
      $("#file-menu").classList.add("hidden");
      await pasteNodes(JSON.stringify({ minihou: "nodes", nodes }));
    } catch (err) { toast(err.message); }
  }
  async function addLibraryNode(f) {
    const t = S.types[f.type];
    if (!t) { toast(f.error || "That library node is not loaded."); return; }
    $("#file-menu").classList.add("hidden");
    const one = S.sel.size === 1 ? [...S.sel][0] : null, id = uniqueId(t.name);
    const r = canvas.getBoundingClientRect(), [x, y] = toWorld(r.left + r.width / 2, r.top + r.height / 2);
    const op = { op: "add", type: t.name, id, pos: [Math.round(x - W / 2), Math.round(y - 12)] };
    if (one && nodeOf(one) && t.inputs[0]) op.inputs = { [t.inputs[0].name]: one };
    const res = await edit([op]);
    if (res) select([id]);
  }
  async function saveSelectionAsNode() {
    const ids = [...S.sel].filter((id) => nodeOf(id));
    if (!ids.length) { toast("Select the nodes to turn into a node first."); return; }
    const first = nodeOf(ids[0]);
    let name = await appPrompt("Name the new library node (letters, digits and _). It appears in the Add Node menu under Library:", ids.length === 1 ? first.type + "_group" : "my_node");
    if (!name) return;
    name = name.trim();
    try {
      const d = await api("/api/make_system", { name, nodes: ids, replace: true });
      $("#file-menu").classList.add("hidden");
      toast(`Made the library node ${name} (${d.path}) and put it where the selection was. Undo (Ctrl+Z) takes the nodes back.`, false);
      for (let i = 0; i < 40 && d.node && !nodeOf(d.node); i++) await new Promise((r) => setTimeout(r, 50));      // the new graph arrives over the event stream
      if (d.node && nodeOf(d.node)) select([d.node]);
    } catch (err) {
      if (/already exists/.test(err.message) && /\.json already exists/.test(err.message) && await appConfirm(name + " exists in the library. Replace it?")) {
        try { await api("/api/make_system", { name, nodes: ids, replace: true, overwrite: true }); $("#file-menu").classList.add("hidden"); } catch (e2) { toast(e2.message); }
      } else toast(err.message);
    }
  }
  async function openFile(path, create, nav) {
    $("#file-menu").classList.add("hidden");
    S.nav = nav || null;
    try { const snap = await api("/api/open", { path, create: !!create }); S.sel.clear(); applySnapshot(snap); }
    catch (e) { toast(e.message); }
    S.nav = null;
  }
  async function newFile() {
    let name = await appPrompt("New graph file (inside " + (S.root || "the workspace") + "):", "graphs/my_system.json");
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

  // Graph editing menus and browser-local appearance presets.
  function colourControl(value, change) {
    return h("input", { type: "color", value: /^#[0-9a-f]{6}$/i.test(value || "") ? value : preferences.nodeColor,
      "aria-label": "Colour", onchange: e => change(e.target.value) });
  }
  const contextMenu = h("div", { id: "graph-context", class: "hidden", role: "menu", "aria-label": "Graph actions" });
  document.body.append(contextMenu);
  function closeContext() { contextMenu.classList.add("hidden"); }
  document.addEventListener("pointerdown", e => { if (!contextMenu.contains(e.target)) closeContext(); }, true);
  contextMenu.onkeydown = e => {
    if (e.key === "Escape") { closeContext(); canvas.focus(); }
    if (["ArrowDown", "ArrowUp"].includes(e.key)) {
      const buttons = [...contextMenu.querySelectorAll("button,input")], at = buttons.indexOf(document.activeElement);
      buttons[(at + (e.key === "ArrowDown" ? 1 : buttons.length - 1)) % buttons.length]?.focus(); e.preventDefault();
    }
    e.stopPropagation();
  };
  function graphContextMenu(e) {
    if(drag?.kind==="cut" || performance.now()<suppressContextUntil){e.preventDefault();return;}
    e.preventDefault(); closeMenu();
    const rows = [], action = (label, fn) => rows.push(h("button", { role: "menuitem", onclick: () => { closeContext(); Promise.resolve(fn()).catch(err => toast(err.message)); } }, label));
    const node = e.target.closest(".node"), box = e.target.closest(".netbox"), wire = e.target.closest("path.hit");
    if (node) {
      const id = node.dataset.id; if (!S.sel.has(id)) select([id]);
      for(const name of ['display','template','selectable_template','bypass','lock','unload'])action((flagState(id,name)?'✓ ':'')+flagTitle(name),()=>setNodeFlag(id,name,true));
      action("Rename…", async () => { const name = await appPrompt("Node name", id); if (name && name !== id) return edit([{ op: "rename", id, to: name }]); });
      action("Duplicate", () => duplicate([...S.sel]));
      action("Frame selection", () => frame(true));
      action("Group in frame", addBox);
      rows.push(h("label", null, "Node colour ", colourControl(nodeOf(id).color, color => edit([...S.sel].map(id => ({ op: "meta", id, set: { color } }))))));
      action("Use default colour", () => edit([...S.sel].map(id => ({ op: "meta", id, set: { color: null } }))));
      action("Delete", () => removeNodes([...S.sel]));
    } else if (wire) {
      const w = wire.dataset;
      action("Focus source", () => { select([w.from]); frame(true); });
      action("Focus destination", () => { select([w.to]); frame(true); });
      action("Disconnect", () => edit([{ op: "disconnect", from: w.from, to: w.to, port: w.port }]));
    } else if (box) {
      const b = boxById(box.dataset.id);
      action("Rename…", () => renameBox(b.id));
      rows.push(h("label", null, "Frame colour ", colourControl(b.color, color => edit([{ op: "box_set", id: b.id, color }]))));
      action(b.collapsed ? "Expand" : "Collapse", () => edit([{ op: "box_set", id: b.id, collapsed: !b.collapsed }]));
      action(b.locked ? "Unlock" : "Lock", () => edit([{ op: "box_set", id: b.id, locked: !b.locked }]));
      if (!b.note) action("Fit around contents", () => {
        const ids = boxMembers(b); if (!ids.length) { toast("This frame contains no nodes.", false); return; }
        const pts = ids.map(pos), x = Math.min(...pts.map(p => p[0])) - 30, y = Math.min(...pts.map(p => p[1])) - 58;
        return edit([{ op: "box_set", id: b.id, pos: [x, y], size: [Math.max(...pts.map(p => p[0])) + W + 30 - x, Math.max(...ids.map(id => pos(id)[1] + nodeHeight(id))) + 30 - y] }]);
      });
      action("Duplicate", () => edit([{ op: "box_add", title: b.title, color: b.color, pos: [b.pos[0] + 40, b.pos[1] + 40], size: b.size, note: !!b.note, text: b.text || "" }]));
      action("Delete frame (keep nodes)", () => edit([{ op: "box_remove", id: b.id }]));
    } else {
      const point = toWorld(e.clientX, e.clientY);
      action("Add node…", () => openMenu(e.clientX, e.clientY, { world: point }));
      action("Add sticky note", () => addStickyNote(point));
      action("Add frame", addBox); action("Find node…", openNodeSearch); action("Frame all", () => frame(false));
    }
    contextMenu.replaceChildren(...rows); contextMenu.classList.remove("hidden");
    contextMenu.style.left = Math.max(8, Math.min(e.clientX, innerWidth - contextMenu.offsetWidth - 8)) + "px";
    contextMenu.style.top = Math.max(8, Math.min(e.clientY, innerHeight - contextMenu.offsetHeight - 8)) + "px";
    contextMenu.querySelector("button")?.focus();
  }
  function addStickyNote(point) {
    if (!point) { const r = canvas.getBoundingClientRect(); point = toWorld(r.left + r.width / 2, r.top + r.height / 2); }
    return edit([{ op: "box_add", title: "Note", color: "#c9a94d", pos: point, size: [260, 180], note: true, text: "Write your note here…" }]);
  }
  $("#note-btn").onclick = () => addStickyNote();
  const searchDialog = h("dialog", { id: "node-search", "aria-label": "Find node" });
  const searchInput = h("input", { type: "search", placeholder: "Name, type or comment…", "aria-label": "Find node" });
  const searchResults = h("div", { class: "search-results" });
  searchDialog.append(h("h2", null, "Find node"), searchInput, searchResults, h("button", { onclick: () => searchDialog.close() }, "Close")); document.body.append(searchDialog);
  function updateNodeSearch() {
    const query = searchInput.value.toLowerCase().trim();
    const matches = Object.entries(S.graph.nodes).filter(([id, n]) => !inputOwner(id) && `${id} ${n.type} ${n.comment || ""}`.toLowerCase().includes(query));
    searchResults.replaceChildren(...matches.slice(0, 100).map(([id, n]) => h("button", { onclick: async () => {
      const hidden = (S.graph.boxes || []).filter(b => b.collapsed && boxMembers(b).includes(id));
      if (hidden.length) await edit(hidden.map(b => ({ op: "box_set", id: b.id, collapsed: false })));
      select([id]); frame(true); searchDialog.close();
    } }, id, h("span", { class: "muted" }, " · " + n.type))));
    if (!matches.length) searchResults.append(h("p", { class: "muted" }, "No matching nodes."));
  }
  function openNodeSearch() { searchInput.value = ""; updateNodeSearch(); searchDialog.showModal(); searchInput.focus(); }
  searchInput.oninput = updateNodeSearch;
  searchInput.onkeydown = e => { if (e.key === "Enter") { searchResults.querySelector("button")?.click(); e.preventDefault(); } if (e.key === "ArrowDown") searchResults.querySelector("button")?.focus(); };
  searchDialog.onkeydown = e => e.stopPropagation();
  $("#find-btn").onclick = openNodeSearch;
  document.addEventListener("keydown", e => {
    if (searchDialog.open) {
      if (e.key === "Enter" && document.activeElement === searchInput) { searchResults.querySelector("button")?.click(); e.preventDefault(); }
      if (["ArrowDown", "ArrowUp"].includes(e.key)) {
        const buttons = [...searchResults.querySelectorAll("button")], at = buttons.indexOf(document.activeElement);
        buttons[(at + (e.key === "ArrowDown" ? 1 : buttons.length - 1)) % buttons.length]?.focus(); e.preventDefault();
      }
      e.stopImmediatePropagation(); return;
    }
    if (!contextMenu.classList.contains("hidden")) { contextMenu.onkeydown(e); e.stopImmediatePropagation(); return; }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f" && !/input|textarea|select/i.test(e.target.tagName)) { e.preventDefault(); e.stopImmediatePropagation(); openNodeSearch(); }
  }, true);
  $("#pv-display-mode").onchange = $("#pref-display-mode").onchange = e => {
    const mode = e.target.value; preferences.shaded = ["shaded", "edges"].includes(mode); preferences.edges = ["edges", "wire"].includes(mode); preferences.points = mode === "points"; applyPreferences(true);
  };
  $("#preferences-save-custom").onclick = () => {
    try { localStorage.setItem("minihou.preferences.custom", JSON.stringify(preferences)); toast("Custom preset saved.", false); }
    catch (err) { toast("Browser storage is unavailable; custom preset was not saved."); }
  };
  $("#preferences-preset").onchange = e => {
    if (!e.target.value) return;
    if (e.target.value === "custom") {
      try {
        const custom = JSON.parse(localStorage.getItem("minihou.preferences.custom") || "null");
        if (!custom) throw new Error("Save a custom preset first.");
        const next = { ...preferenceDefaults };
        for (const key of Object.keys(next)) if (key in custom && validPreference(key, custom[key])) next[key] = custom[key];
        Object.assign(preferences, next); applyPreferences(true);
      } catch (err) { toast(err.message); }
      e.target.value = ""; return;
    }
    Object.assign(preferences, preferenceDefaults);
    if (e.target.value === "houdini") Object.assign(preferences, { background: "#3d3d3d", nodeBackground: "#303030", nodeGrid: "#393939", nodeColor: "#505050", selectionColor: "#ffb540", nodeSnap: true });
    if (e.target.value === "contrast") Object.assign(preferences, { background: "#101010", nodeBackground: "#101010", nodeGrid: "#444444", nodeColor: "#424242", wireColor: "#ffffff", selectionColor: "#ffff00", gridOpacity: 1.6 });
    applyPreferences(true); e.target.value = "";
  };
  $("#preferences-export").onclick = () => {
    const url = URL.createObjectURL(new Blob([JSON.stringify({ version: 1, preferences }, null, 2)], { type: "application/json" }));
    const a = h("a", { href: url, download: "minihou-preferences.json" }); a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  $("#preferences-import").onclick = () => $("#preferences-file").click();
  $("#preferences-file").onchange = async e => {
    const file = e.target.files[0]; if (!file) return;
    try {
      if (file.size > 100000) throw new Error("Preferences file is too large.");
      const data = JSON.parse(await file.text());
      if (data.version !== 1 || !data.preferences || typeof data.preferences !== "object") throw new Error("Not a supported preferences file.");
      const next = { ...preferenceDefaults };
      for (const key of Object.keys(next)) if (key in data.preferences) {
        if (!validPreference(key, data.preferences[key])) throw new Error("Invalid preference: " + key);
        next[key] = data.preferences[key];
      }
      Object.assign(preferences, next); applyPreferences(true); toast("Preferences imported.", false);
    } catch (err) { toast(err.message); } finally { e.target.value = ""; }
  };
  // Desktop-style shell: commands stay wired to the same editor actions.
  const appPopup = h("div", { id: "app-menu-popup", role: "menu", class: "hidden" });
  document.body.append(appPopup);
  let appMenuOwner = null;
  function closeAppMenu(restoreFocus = false) {
    appPopup.classList.add("hidden");
    if (appMenuOwner) { appMenuOwner.setAttribute("aria-expanded", "false"); if (restoreFocus) appMenuOwner.focus(); }
    appMenuOwner = null;
  }
  const clickCommand = id => () => $(id).click();
  const appCommands = {
    File: [["New graph…", newFile], ["Open graph…", clickCommand("#file-btn")], ["Save as…", saveAs], ["Preferences…", clickCommand("#preferences-btn")]],
    Edit: [["Undo", clickCommand("#undo-btn"), "Ctrl+Z", () => !S.canUndo], ["Redo", clickCommand("#redo-btn"), "Ctrl+Shift+Z", () => !S.canRedo],
      ["Add node…", clickCommand("#add-btn"), "Tab"], ["Find node…", openNodeSearch, "Ctrl+F"], ["Duplicate selection", () => duplicate([...S.sel]), "Ctrl+D", () => !S.sel.size],
      ["Group in frame", addBox, "O"], ["Sticky note", () => addStickyNote()], ["Tidy layout", clickCommand("#tidy-btn"), "L"], ["Delete selection", () => removeNodes([...S.sel]), "Delete", () => !S.sel.size]],
    View: [["Node network", () => layout.set({ max: "graph" })], ["Split workspace", () => layout.set({ max: null })], ["3D viewport", () => layout.set({ max: "view" })],
      ["Frame all nodes", () => frame(false), "Home"], ["Swap panes", clickCommand("#lay-swap")], ["Stack panes", clickCommand("#lay-dir")], ["Toggle parameters", clickCommand("#lay-side")],
      ["Blender connection…", clickCommand("#blender-status")], ["Preferences…", clickCommand("#preferences-btn")]],
    Help: [["Keyboard and mouse", clickCommand("#help-btn"), "?"], ["Blender connection…", clickCommand("#blender-status")]]
  };
  function openAppMenu(button, focus = true) {
    closeAppMenu(); closeContext();
    appMenuOwner = button; button.setAttribute("aria-expanded", "true");
    appPopup.replaceChildren(...appCommands[button.dataset.appMenu].map(([label, action, shortcut, disabled]) => h("button", {
      role: "menuitem", disabled: disabled?.(), onclick: () => { closeAppMenu(); Promise.resolve(action()).catch(err => toast(err.message)); }
    }, h("span", null, label), h("span", { class: "menu-shortcut" }, shortcut || ""))));
    appPopup.classList.remove("hidden"); const r = button.getBoundingClientRect();
    appPopup.style.left = r.left + "px"; appPopup.style.top = r.bottom + "px";
    if (focus) appPopup.querySelector("button:not(:disabled)")?.focus();
  }
  const appMenuButtons = [...document.querySelectorAll("[data-app-menu]")];
  appMenuButtons.forEach(button => {
    button.onclick = () => appMenuOwner === button ? closeAppMenu() : openAppMenu(button);
    button.onpointerenter = () => { if (appMenuOwner && appMenuOwner !== button) openAppMenu(button, false); };
  });
  document.addEventListener("pointerdown", e => { if (!appPopup.contains(e.target) && !e.target.closest("[data-app-menu]")) closeAppMenu(); }, true);
  document.addEventListener("keydown", e => {
    if (e.key === "F10" && !preferencesDialog.open && !searchDialog.open) { e.preventDefault(); e.stopImmediatePropagation(); appMenuOwner ? closeAppMenu(true) : openAppMenu(appMenuButtons[0]); return; }
    const menuButton = e.target.closest("[data-app-menu]");
    if (!appMenuOwner && menuButton && ["ArrowDown", "ArrowUp"].includes(e.key)) { openAppMenu(menuButton); e.preventDefault(); e.stopImmediatePropagation(); return; }
    if (!appMenuOwner) return;
    if (e.key === "Escape") closeAppMenu(true);
    if (e.key === "Tab") closeAppMenu();
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) {
      const items = [...appPopup.querySelectorAll("button:not(:disabled)")], at = items.indexOf(document.activeElement);
      const index = e.key === "Home" ? 0 : e.key === "End" ? items.length - 1 : (at + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length;
      items[index]?.focus(); e.preventDefault();
    }
    if (["ArrowLeft", "ArrowRight"].includes(e.key)) { const at = appMenuButtons.indexOf(appMenuOwner); openAppMenu(appMenuButtons[(at + (e.key === "ArrowRight" ? 1 : appMenuButtons.length - 1)) % appMenuButtons.length]); e.preventDefault(); }
    e.stopImmediatePropagation();
  }, true);
  [$("#cook-status"), $("#blender-status"), $("#topbar .proxy")].forEach(el => $("#app-status-items").append(el));
  $("#network-pane-header").onpointerdown = e => e.stopPropagation();
  $("#canvas").classList.add("active-pane");
  $("#main").addEventListener("pointerdown", e => {
    const active = e.target.closest("#canvas,#preview,#side"); if (!active) return;
    document.querySelectorAll(".active-pane").forEach(p => p.classList.remove("active-pane")); active.classList.add("active-pane");
  }, true);
  applyView();
  try { recoveryButton(); } catch (e) { /* Browser storage may be disabled. */ }
  connectEvents();
  window.minihou = { desktopCanClose: () => { document.activeElement?.blur(); return !desktopPendingEdits && !setInFlight && !pendingSet.size; }, S, edit, frame, clipboardText, pasteNodes, preview: pv, layout };                  // handy from the browser console
})();
