/* The geometry spreadsheet: rows of points, vertices, primitives, edges or detail attributes of the viewed node.
 *
 * The engine pages the rows (POST /api/spreadsheet) and also sorts and filters them; this keeps only the rows near the scroll position, so a
 * million points cost the same as ten. A row number selects that element in the viewport (and the selected-only box filters to the viewport
 * selection); double-clicking a cell of an attribute edits it through Quick Assign (one paint_assign edit). Header clicks sort, the filter box
 * narrows the rows, the Columns menu hides columns, cells can be selected, copied (Ctrl+C) and pasted over (Ctrl+V), and each attribute
 * column says which node made it.
 */
window.MiniSheet = (root, host) => {
  const ROW = 22, PAGE = 100, FETCH = 500, MAX_PASTE = 20000;
  const DOMAINS = [["point", "Points"], ["vertex", "Vertices"], ["prim", "Primitives"], ["edge", "Edges"], ["detail", "Detail"]];
  const EDITABLE_DOMAINS = ["point", "prim", "edge"];
  const COMP_LABEL = { 0: "x", 1: "y", 2: "z", "-1": "len" };
  const st = { domain: "point", selectedOnly: false, open: false, sort: {}, hidden: {}, filter: "" };
  try { Object.assign(st, JSON.parse(localStorage.getItem("minihou.sheet") || "{}"), { open: false, filter: "" }); } catch (e) { /* no storage */ }
  const save = () => { try { localStorage.setItem("minihou.sheet", JSON.stringify({ domain: st.domain, selectedOnly: st.selectedOnly, sort: st.sort, hidden: st.hidden })); } catch (e) { /* no storage */ } };
  const el = (tag, props, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (k === "class") e.className = v; else if (k === "dataset") Object.assign(e.dataset, v); else if (k.startsWith("on")) e[k] = v; else if (v !== false && v != null) e.setAttribute(k, v === true ? "" : v);
    }
    for (const kid of kids.flat()) if (kid != null) e.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
    return e;
  };
  let data = null, pages = new Map(), pending = new Set(), token = 0, ids = null, lastKey = "", scrollRaf = 0, filterTimer = 0;
  let cells = null;                                                        // the selected cells: {i0, i1, k0, k1, anchor: [i, k]} (row positions, column indexes into data.columns)
  const tabs = el("div", { class: "sh-tabs", role: "tablist" }, DOMAINS.map(([d, label]) => el("button", { type: "button", class: "sh-tab", dataset: { domain: d }, role: "tab", onclick: () => setDomain(d) }, label)));
  const only = el("input", { type: "checkbox", class: "sh-only" });
  only.onchange = () => { st.selectedOnly = only.checked; save(); reload(); };
  const filterBox = el("input", { type: "text", class: "sh-filter", "aria-label": "Filter rows", spellcheck: "false" });
  filterBox.oninput = () => { clearTimeout(filterTimer); filterTimer = setTimeout(() => { st.filter = filterBox.value.trim(); reload(); }, 300); };
  filterBox.onkeydown = (e) => { if (e.key === "Enter") { clearTimeout(filterTimer); st.filter = filterBox.value.trim(); reload(); } else if (e.key === "Escape") { filterBox.value = ""; clearTimeout(filterTimer); st.filter = ""; reload(); } };
  const status = el("span", { class: "sh-status muted" });
  const colsBtn = el("button", { type: "button", class: "sh-cols", title: "Show or hide columns", onclick: () => toggleColumns() }, "Columns");
  const colsMenu = el("div", { class: "sh-colsmenu hidden" });
  const bar = el("div", { class: "sh-bar" }, tabs, el("label", { class: "sh-onlylabel", title: "Only the elements selected in the 3-D view" }, only, "Selected only"), filterBox, colsBtn, el("span", { class: "spacer" }), status,
    el("button", { type: "button", class: "sh-close", "aria-label": "Close spreadsheet", title: "Close", onclick: () => api.close() }, "×"));
  const head = el("div", { class: "sh-head" });
  const spacer = el("div", { class: "sh-spacer" });
  const rowsBox = el("div", { class: "sh-rows" });
  spacer.append(rowsBox);
  const scroll = el("div", { class: "sh-scroll", tabindex: "0" }, head, spacer);
  const msg = el("div", { class: "sh-msg muted hidden" });
  root.append(bar, colsMenu, msg, scroll);
  root.addEventListener("keydown", (e) => e.stopPropagation());
  root.addEventListener("click", (e) => { if (!e.target.closest(".sh-colsmenu, .sh-cols")) colsMenu.classList.add("hidden"); });     // a click elsewhere closes the Columns menu
  scroll.addEventListener("scroll", () => { if (!scrollRaf) scrollRaf = requestAnimationFrame(() => { scrollRaf = 0; paint(); }); });

  const fmt = (v) => (Array.isArray(v) ? v.map(fmt).join(", ") : typeof v === "number" ? String(v) : String(v));
  const hiddenNow = () => new Set(st.hidden[st.domain] || []);
  const visible = () => (data ? data.columns.map((c, k) => k).filter((k) => !hiddenNow().has(data.columns[k].name)) : []);
  const widthOf = (c) => (c.comps >= 3 ? "190px" : c.comps === 2 ? "130px" : "96px");
  const widths = () => (data ? ["56px", ...visible().map((k) => widthOf(data.columns[k]))] : []);
  const sortable = () => st.domain !== "detail";
  const sortNow = () => (sortable() ? st.sort[st.domain] || null : null);
  const filterable = () => st.domain !== "detail";
  function setDomain(d) { st.domain = d; save(); data = null; pages = new Map(); cells = null; filterBox.value = ""; st.filter = ""; reload(); syncTabs(); }
  function syncTabs() {
    tabs.querySelectorAll(".sh-tab").forEach((b) => b.classList.toggle("on", b.dataset.domain === st.domain));
    only.checked = st.selectedOnly; only.disabled = !EDITABLE_DOMAINS.includes(st.domain);
    filterBox.disabled = !filterable();
    filterBox.placeholder = st.domain === "point" || st.domain === "prim" ? "Filter: @w > 0.5 · group · 0-99" : st.domain === "detail" ? "" : "Filter: faces == 1 and a < 10";
    filterBox.title = st.domain === "point" || st.domain === "prim" ? "Rows to keep: a group name, element numbers (0-99), or an expression such as @P.z > 0 and @w > 0.5" : "Rows to keep: column <op> number, joined by 'and' (<, <=, >, >=, ==, !=)";
  }
  function keyNow() {
    const sel = st.selectedOnly ? (host.selectionFor(st.domain) || []) : null, s = sortNow();
    return [host.key(), st.domain, st.selectedOnly, sel ? sel.length + ":" + (sel[0] ?? "") + ":" + (sel[sel.length - 1] ?? "") : "", s ? `${s.column}/${s.comp}/${s.desc}` : "", st.filter].join("|");
  }
  const request = (start, count, extra) => host.post("/api/spreadsheet", Object.assign({ node: host.viewedNode(), domain: st.domain, start, count, ids: ids || undefined, sort: sortNow() || undefined, filter: st.filter || undefined }, extra));

  async function load(start) {
    if (pages.has(start) || pending.has(start)) return;
    if (!host.viewedNode()) return;
    pending.add(start);
    const mine = token;
    try {
      const r = await request(start, PAGE, start === 0 ? { origins: true } : null);
      if (mine !== token) return;
      if (!r.ok) { data = null; rowsBox.replaceChildren(); head.replaceChildren(); spacer.style.height = "0px"; status.textContent = ""; showMessage(r.error ? r.error.message : "No geometry"); return; }
      if (!data) { data = r; buildHead(); }
      data.total = r.total; data.cook_level = r.cook_level;
      if (r.origins) { data.origins = r.origins; buildHead(); }
      pages.set(start, r.rows);
      showMessage("");
      paint();
    } catch (e) { if (mine === token) { data = null; rowsBox.replaceChildren(); head.replaceChildren(); spacer.style.height = "0px"; status.textContent = ""; showMessage(String(e.message || e)); } }
    finally { pending.delete(start); }
  }
  function showMessage(t) { msg.textContent = t || ""; msg.classList.toggle("hidden", !t); }
  function sortMark(c) {
    const s = sortNow();
    if (!s || s.column !== c.name) return "";
    return (s.desc ? "▼" : "▲") + (c.comps > 1 ? " " + (COMP_LABEL[s.comp] || "x") : "");
  }
  function buildHead() {
    const w = widths(), org = data.origins || {};
    head.style.gridTemplateColumns = w.join(" ");
    head.replaceChildren(el("div", { class: "sh-h sh-idx" }, "#"), ...visible().map((k) => {
      const c = data.columns[k], o = org[c.name], sortOk = sortable() && c.dtype !== "str";
      const kids = [el("span", { class: "sh-hname" }, c.name)];
      const mark = sortMark(c);
      if (mark) kids.push(el("span", { class: "sh-sort" }, mark));
      if (o) kids.push(el("span", { class: "sh-org", dataset: { node: o }, title: `Made by node ${o} (click to select it)`, onclick: (e) => { e.stopPropagation(); host.selectNode(o); } }, "← " + o));
      return el("div", { class: "sh-h" + (sortOk ? " sortable" : ""), dataset: { col: c.name, k },
        title: `${c.name} (${c.dtype}${c.comps > 1 ? " x" + c.comps : ""})` + (sortOk ? "\nClick: sort · " + (c.comps > 1 ? "Alt-click: x/y/z/length" : "click again: descending, again: off") : ""),
        onclick: (e) => { if (sortOk) sortBy(c, e.altKey); } }, kids);
    }));
    rowsBox.style.minWidth = head.style.minWidth = w.reduce((a, x) => a + parseInt(x, 10), 0) + "px";
    if (!colsMenu.classList.contains("hidden")) buildColumnsMenu();                // an open menu follows a reload
  }
  function sortBy(c, alt) {
    const cur = sortNow(), mine = cur && cur.column === c.name ? cur : null;
    let next;
    if (alt && c.comps > 1) { const order = [0, 1, 2, -1].filter((x) => x === -1 || x < c.comps), at = mine ? order.indexOf(mine.comp) : 0; next = { column: c.name, comp: order[(at + 1) % order.length], desc: mine ? mine.desc : false }; }
    else if (!mine) next = { column: c.name, comp: 0, desc: false };
    else if (!mine.desc) next = { column: c.name, comp: mine.comp, desc: true };
    else next = null;
    if (next) st.sort[st.domain] = next; else delete st.sort[st.domain];
    save(); reload();
  }
  function reload() {
    token++; pages = new Map(); pending = new Set(); data = null; ids = null; cells = null;
    if (!st.open) return;
    if (st.selectedOnly && EDITABLE_DOMAINS.includes(st.domain)) { ids = host.selectionFor(st.domain) || []; }
    lastKey = keyNow();
    status.textContent = "";
    rowsBox.replaceChildren(); head.replaceChildren();
    spacer.style.height = "0px";
    if (st.selectedOnly && ids && !ids.length) { showMessage("Nothing is selected: select points (1), edges (2) or faces (3) in the view."); status.textContent = "0 rows"; return; }
    showMessage("");
    load(0).then(paint);
  }
  function paint() {
    if (!data || rowsBox.querySelector(".sh-edit")) return;                // never rebuild the rows under a cell that is being edited
    const total = data.total;
    status.textContent = `${total.toLocaleString()} row${total === 1 ? "" : "s"}` + (st.filter ? " (filtered)" : "") + (data.cook_level < 1 ? " · cook proxy " + Math.round(data.cook_level * 1000) / 10 + "%" : "");
    spacer.style.height = total * ROW + "px";
    const top = scroll.scrollTop, h = scroll.clientHeight, first = Math.max(0, Math.floor(top / ROW) - 4), last = Math.min(total - 1, Math.ceil((top + h) / ROW) + 4);
    for (let p = Math.floor(first / PAGE) * PAGE; p <= last; p += PAGE) load(p);
    const out = [], sel = host.selectedSet(st.domain), w = widths(), vis = visible();
    for (let i = first; i <= last; i++) {
      const page = pages.get(Math.floor(i / PAGE) * PAGE), row = page && page[i - Math.floor(i / PAGE) * PAGE];
      if (!row) continue;
      const tr = el("div", { class: "sh-row" + (sel && sel.has(row[0]) ? " sel" : ""), dataset: { id: row[0], i }, style: `top:${i * ROW}px;grid-template-columns:${w.join(" ")}` },
        el("div", { class: "sh-c sh-idx" }, row[0]),
        ...vis.map((k) => { const c = data.columns[k]; return el("div", { class: "sh-c" + (editable(c) ? " ed" : ""), dataset: { col: c.name, k }, title: fmt(row[k + 1]) }, fmt(row[k + 1])); }));
      out.push(tr);
    }
    rowsBox.replaceChildren(...out);
    markCells();
  }
  const editable = (c) => !c.computed && EDITABLE_DOMAINS.includes(st.domain) && c.dtype !== "str";

  // ---------------------------------------------------------------- columns menu
  function toggleColumns() {
    if (!colsMenu.classList.contains("hidden")) { colsMenu.classList.add("hidden"); return; }
    buildColumnsMenu();
    colsMenu.classList.remove("hidden");
  }
  function buildColumnsMenu() {
    if (!data) { colsMenu.replaceChildren(el("span", { class: "muted" }, "Loading...")); return; }
    const hid = hiddenNow();
    colsMenu.replaceChildren(el("button", { type: "button", class: "sh-colsall", onclick: () => { delete st.hidden[st.domain]; save(); applyColumns(); buildColumnsMenu(); } }, "Show all"),
      ...data.columns.map((c) => {
        const box = el("input", { type: "checkbox", dataset: { col: c.name } }); box.checked = !hid.has(c.name);
        box.onchange = () => { const h = hiddenNow(); if (box.checked) h.delete(c.name); else h.add(c.name); if (h.size) st.hidden[st.domain] = [...h]; else delete st.hidden[st.domain]; save(); applyColumns(); };
        return el("label", { class: "sh-colitem" }, box, c.name);
      }));
  }
  function applyColumns() { if (!data) return; cells = null; buildHead(); paint(); }

  // ---------------------------------------------------------------- cell selection, copy and paste
  function markCells() {
    rowsBox.querySelectorAll(".cs, .cs-active").forEach((n) => n.classList.remove("cs", "cs-active"));
    if (!cells) return;
    const vis = visible(), k0 = Math.min(cells.k0, cells.k1), k1 = Math.max(cells.k0, cells.k1), i0 = Math.min(cells.i0, cells.i1), i1 = Math.max(cells.i0, cells.i1);
    const inRange = new Set(vis.filter((k) => k >= k0 && k <= k1));
    rowsBox.querySelectorAll(".sh-row").forEach((r) => {
      const i = Number(r.dataset.i);
      if (i < i0 || i > i1) return;
      r.querySelectorAll(".sh-c[data-k]").forEach((c) => {
        const k = Number(c.dataset.k);
        if (inRange.has(k)) c.classList.add("cs");
        if (i === cells.anchor[0] && k === cells.anchor[1]) c.classList.add("cs-active");
      });
    });
  }
  rowsBox.addEventListener("click", (e) => {
    const cell = e.target.closest(".sh-c[data-k]");
    if (!cell || cell.querySelector(".sh-edit")) return;
    const i = Number(cell.closest(".sh-row").dataset.i), k = Number(cell.dataset.k);
    cells = e.shiftKey && cells ? { i0: cells.anchor[0], k0: cells.anchor[1], i1: i, k1: k, anchor: cells.anchor } : { i0: i, k0: k, i1: i, k1: k, anchor: [i, k] };
    markCells();
    scroll.focus({ preventScroll: true });
  });
  const rangeBounds = () => {
    const vis = visible(), a = vis.indexOf(cells.k0), b = vis.indexOf(cells.k1);
    return { i0: Math.min(cells.i0, cells.i1), i1: Math.max(cells.i0, cells.i1), cols: vis.slice(Math.min(a, b), Math.max(a, b) + 1) };
  };
  function selectionText() {                                               // the selected cells as tab-separated text (vector components joined by spaces)
    if (!data || !cells) return "";
    const { i0, i1, cols } = rangeBounds(), lines = [];
    for (let i = i0; i <= i1; i++) {
      const page = pages.get(Math.floor(i / PAGE) * PAGE), row = page && page[i - Math.floor(i / PAGE) * PAGE];
      if (!row) continue;
      lines.push(cols.map((k) => { const v = row[k + 1]; return Array.isArray(v) ? v.join(" ") : String(v); }).join("\t"));
    }
    return lines.join("\n");
  }
  async function rowIds(i0, i1) {                                         // the element numbers of display rows i0..i1, from the pages held or fetched in big pages
    const out = new Array(i1 - i0 + 1);
    let need = null;
    for (let i = i0; i <= i1; i++) {
      const page = pages.get(Math.floor(i / PAGE) * PAGE), row = page && page[i - Math.floor(i / PAGE) * PAGE];
      if (row) out[i - i0] = row[0]; else if (need === null) need = i;
    }
    for (let at = need === null ? i1 + 1 : need; at <= i1;) {
      if (out[at - i0] !== undefined) { at++; continue; }
      const r = await request(at, FETCH);
      if (!r.ok) throw new Error(r.error ? r.error.message : "No geometry");
      r.rows.forEach((row, j) => { if (at + j <= i1) out[at + j - i0] = row[0]; });
      at += Math.max(1, r.rows.length);
    }
    return out;
  }
  async function paste(text) {
    if (!data || !cells) { host.toast("Click a cell first: the paste starts there."); return false; }
    const lines = String(text).replace(/\r/g, "").split("\n"); while (lines.length && lines[lines.length - 1] === "") lines.pop();
    if (!lines.length) return false;
    const grid = lines.map((l) => (l.includes("\t") ? l.split("\t") : [l]));
    const vis = visible(), startCol = vis.indexOf(cells.anchor[1]), b = rangeBounds();
    const single = grid.length === 1 && grid[0].length === 1;
    const rows = single ? b.i1 - b.i0 + 1 : grid.length, i0 = single ? b.i0 : cells.anchor[0];
    const colKs = single ? b.cols : vis.slice(startCol, startCol + Math.max(...grid.map((r) => r.length)));
    const n = Math.min(rows, data.total - i0);
    if (n <= 0) return false;
    if (n > MAX_PASTE) { host.toast(`Paste at most ${MAX_PASTE.toLocaleString()} rows at a time.`); return false; }
    const editableCols = colKs.map((k) => data.columns[k]);
    if (!editableCols.some(editable)) { host.toast("Those columns are read-only (built-in values cannot be pasted over)."); return false; }
    const num = (s) => String(s).trim().split(/[\s,;]+/).filter(Boolean).map(Number);
    const jobs = [];
    for (let j = 0; j < colKs.length; j++) {
      const c = data.columns[colKs[j]];
      if (!editable(c)) continue;
      const vals = [];
      for (let r = 0; r < n; r++) {
        const cell = single ? grid[0][0] : (grid[r] || [])[j];
        if (cell === undefined) { vals.push(null); continue; }
        const nums = num(cell);
        if (nums.length !== c.comps || nums.some((x) => !Number.isFinite(x))) { host.toast(`Row ${i0 + r}, "${c.name}" needs ${c.comps} number${c.comps === 1 ? "" : "s"}, not "${String(cell).trim()}".`); return false; }
        vals.push(c.comps === 1 ? nums[0] : nums);
      }
      jobs.push({ col: c, values: vals });
    }
    const ids2 = await rowIds(i0, i0 + n - 1);
    for (const job of jobs) {                                              // one edit (one undo step) per column
      const at = [], vv = [];
      job.values.forEach((v, r) => { if (v !== null && ids2[r] !== undefined) { at.push(ids2[r]); vv.push(v); } });
      if (at.length) await host.assignCells(st.domain, job.col, at, vv);
    }
    return true;
  }
  scroll.addEventListener("copy", (e) => { const t = selectionText(); if (!t || (e.target.closest && e.target.closest("input"))) return; e.clipboardData.setData("text/plain", t); e.preventDefault(); });
  scroll.addEventListener("paste", (e) => { if (e.target.closest && e.target.closest("input")) return; e.preventDefault(); paste(e.clipboardData.getData("text/plain")); });

  rowsBox.addEventListener("click", (e) => {
    const row = e.target.closest(".sh-idx") && e.target.closest(".sh-row");                  // the number selects; a cell is for editing (selecting moves the table)
    if (!row || !EDITABLE_DOMAINS.includes(st.domain)) return;
    host.selectElements(st.domain, [Number(row.dataset.id)], e.shiftKey || e.ctrlKey ? "add" : "replace");
  });
  rowsBox.addEventListener("dblclick", (e) => {
    const cell = e.target.closest(".sh-c.ed");
    if (!cell) return;
    const row = cell.closest(".sh-row"), col = data.columns[Number(cell.dataset.k)], id = Number(row.dataset.id);
    const input = el("input", { type: "text", class: "sh-edit", value: cell.textContent, "aria-label": `${col.name} of element ${id}` });
    cell.replaceChildren(input); input.focus(); input.select();
    let done = false;
    const finish = async (ok) => {
      if (done) return; done = true;
      if (ok) {
        const nums = input.value.split(/[\s,]+/).filter(Boolean).map(Number);
        if (nums.length !== col.comps || nums.some((n) => !Number.isFinite(n))) host.toast(`${col.name} needs ${col.comps} number${col.comps === 1 ? "" : "s"}.`);
        else await host.assignCell(st.domain, col, id, nums);
      }
      paint();
    };
    input.onkeydown = (ev) => { if (ev.key === "Enter") { ev.preventDefault(); finish(true); } else if (ev.key === "Escape") { ev.preventDefault(); finish(false); } };
    input.onblur = () => finish(false);
  });

  const api = {
    open() { st.open = true; root.classList.remove("hidden"); filterBox.value = st.filter; syncTabs(); reload(); host.opened(true); },
    close() { st.open = false; root.classList.add("hidden"); token++; host.opened(false); },
    isOpen: () => st.open,
    toggle() { st.open ? api.close() : api.open(); },
    changed() {                                                                                    // geometry or selection moved on
      if (!st.open) return;
      if (keyNow() !== lastKey) { reload(); return; }
      const sel = host.selectedSet(st.domain);                                                     // only the highlight: rebuilding the rows would eat a double click
      rowsBox.querySelectorAll(".sh-row").forEach((r) => r.classList.toggle("sel", !!(sel && sel.has(Number(r.dataset.id)))));
    },
    state: st,
    rows: () => rowsBox.querySelectorAll(".sh-row").length,
    selectionText, paste, columns: () => (data ? data.columns.map((c) => c.name) : []), origins: () => (data && data.origins) || {},
  };
  syncTabs();
  return api;
};
