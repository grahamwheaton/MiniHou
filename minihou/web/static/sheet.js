/* The geometry spreadsheet: rows of points, vertices, primitives, edges or detail attributes of the viewed node.
 *
 * The engine pages the rows (POST /api/spreadsheet); this keeps only the rows near the scroll position, so a million points cost the same as ten.
 * A row click selects that element in the viewport (and the selected-only box filters to the viewport selection); double-clicking a cell of an
 * attribute edits it through Quick Assign (one paint_assign edit).
 */
window.MiniSheet = (root, host) => {
  const ROW = 22, PAGE = 100;
  const DOMAINS = [["point", "Points"], ["vertex", "Vertices"], ["prim", "Primitives"], ["edge", "Edges"], ["detail", "Detail"]];
  const st = { domain: "point", selectedOnly: false, open: false };
  try { Object.assign(st, JSON.parse(localStorage.getItem("minihou.sheet") || "{}"), { open: false }); } catch (e) { /* no storage */ }
  const save = () => { try { localStorage.setItem("minihou.sheet", JSON.stringify({ domain: st.domain, selectedOnly: st.selectedOnly })); } catch (e) { /* no storage */ } };
  const el = (tag, props, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (k === "class") e.className = v; else if (k === "dataset") Object.assign(e.dataset, v); else if (k.startsWith("on")) e[k] = v; else if (v !== false && v != null) e.setAttribute(k, v === true ? "" : v);
    }
    for (const kid of kids.flat()) if (kid != null) e.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
    return e;
  };
  let data = null, pages = new Map(), pending = new Set(), token = 0, ids = null, lastKey = "", scrollRaf = 0;
  const tabs = el("div", { class: "sh-tabs", role: "tablist" }, DOMAINS.map(([d, label]) => el("button", { type: "button", class: "sh-tab", dataset: { domain: d }, role: "tab", onclick: () => setDomain(d) }, label)));
  const only = el("input", { type: "checkbox", class: "sh-only" });
  only.onchange = () => { st.selectedOnly = only.checked; save(); reload(); };
  const status = el("span", { class: "sh-status muted" });
  const bar = el("div", { class: "sh-bar" }, tabs, el("label", { class: "sh-onlylabel", title: "Only the elements selected in the 3-D view" }, only, "Selected only"), el("span", { class: "spacer" }), status,
    el("button", { type: "button", class: "sh-close", "aria-label": "Close spreadsheet", title: "Close", onclick: () => api.close() }, "×"));
  const head = el("div", { class: "sh-head" });
  const spacer = el("div", { class: "sh-spacer" });
  const rowsBox = el("div", { class: "sh-rows" });
  spacer.append(rowsBox);
  const scroll = el("div", { class: "sh-scroll", tabindex: "0" }, head, spacer);
  const msg = el("div", { class: "sh-msg muted hidden" });
  root.append(bar, msg, scroll);
  root.addEventListener("keydown", (e) => e.stopPropagation());
  scroll.addEventListener("scroll", () => { if (!scrollRaf) scrollRaf = requestAnimationFrame(() => { scrollRaf = 0; paint(); }); });

  const fmt = (v) => (Array.isArray(v) ? v.map(fmt).join(", ") : typeof v === "number" ? (Number.isInteger(v) ? String(v) : String(v)) : String(v));
  const widths = () => (data ? ["56px", ...data.columns.map((c) => (c.comps >= 3 ? "190px" : c.comps === 2 ? "130px" : "96px"))] : []);
  function setDomain(d) { st.domain = d; save(); data = null; pages = new Map(); reload(); syncTabs(); }
  function syncTabs() { tabs.querySelectorAll(".sh-tab").forEach((b) => b.classList.toggle("on", b.dataset.domain === st.domain)); only.checked = st.selectedOnly; only.disabled = !["point", "prim", "edge"].includes(st.domain); }
  function keyNow() { const sel = st.selectedOnly ? (host.selectionFor(st.domain) || []) : null; return [host.key(), st.domain, st.selectedOnly, sel ? sel.length + ":" + (sel[0] ?? "") + ":" + (sel[sel.length - 1] ?? "") : ""].join("|"); }

  async function load(start) {
    if (pages.has(start) || pending.has(start)) return;
    const node = host.viewedNode();
    if (!node) return;
    pending.add(start);
    const mine = token;
    try {
      const r = await host.post("/api/spreadsheet", { node, domain: st.domain, start, count: PAGE, ids: ids || undefined });
      if (mine !== token) return;
      if (!r.ok) { showMessage(r.error ? r.error.message : "No geometry"); return; }
      if (!data) { data = r; buildHead(); }
      data.total = r.total; data.cook_level = r.cook_level;
      pages.set(start, r.rows);
      showMessage("");
      paint();
    } catch (e) { if (mine === token) showMessage(String(e.message || e)); }
    finally { pending.delete(start); }
  }
  function showMessage(t) { msg.textContent = t || ""; msg.classList.toggle("hidden", !t); }
  function buildHead() {
    const w = widths();
    head.style.gridTemplateColumns = w.join(" ");
    head.replaceChildren(el("div", { class: "sh-h sh-idx" }, "#"), ...data.columns.map((c) => el("div", { class: "sh-h", title: `${c.name} (${c.dtype}${c.comps > 1 ? " x" + c.comps : ""})` }, c.name)));
    rowsBox.style.minWidth = head.style.minWidth = w.reduce((a, x) => a + parseInt(x, 10), 0) + "px";
  }
  function reload() {
    token++; pages = new Map(); pending = new Set(); data = null; ids = null;
    if (!st.open) return;
    if (st.selectedOnly && ["point", "prim", "edge"].includes(st.domain)) { ids = host.selectionFor(st.domain) || []; }
    lastKey = keyNow();
    status.textContent = "";
    rowsBox.replaceChildren(); head.replaceChildren();
    spacer.style.height = "0px";
    if (st.selectedOnly && ids && !ids.length) { showMessage("Nothing is selected: select points (1), edges (2) or faces (3) in the view."); status.textContent = "0 rows"; return; }
    load(0).then(paint);
  }
  function paint() {
    if (!data || rowsBox.querySelector(".sh-edit")) return;                // never rebuild the rows under a cell that is being edited
    const total = data.total;
    status.textContent = `${total.toLocaleString()} row${total === 1 ? "" : "s"}` + (data.cook_level < 1 ? " · cook proxy " + Math.round(data.cook_level * 1000) / 10 + "%" : "");
    spacer.style.height = total * ROW + "px";
    const top = scroll.scrollTop, h = scroll.clientHeight, first = Math.max(0, Math.floor(top / ROW) - 4), last = Math.min(total - 1, Math.ceil((top + h) / ROW) + 4);
    for (let p = Math.floor(first / PAGE) * PAGE; p <= last; p += PAGE) load(p);
    const out = [], sel = host.selectedSet(st.domain), w = widths();
    for (let i = first; i <= last; i++) {
      const page = pages.get(Math.floor(i / PAGE) * PAGE), row = page && page[i - Math.floor(i / PAGE) * PAGE];
      if (!row) continue;
      const tr = el("div", { class: "sh-row" + (sel && sel.has(row[0]) ? " sel" : ""), dataset: { id: row[0] }, style: `top:${i * ROW}px;grid-template-columns:${w.join(" ")}` },
        el("div", { class: "sh-c sh-idx" }, row[0]),
        ...data.columns.map((c, k) => el("div", { class: "sh-c" + (editable(c) ? " ed" : ""), dataset: { col: c.name, k }, title: fmt(row[k + 1]) }, fmt(row[k + 1]))));
      out.push(tr);
    }
    rowsBox.replaceChildren(...out);
  }
  const editable = (c) => !c.computed && ["point", "prim", "edge"].includes(st.domain) && c.dtype !== "str";
  rowsBox.addEventListener("click", (e) => {
    const row = e.target.closest(".sh-idx") && e.target.closest(".sh-row");                  // the number selects; a cell is for editing (selecting moves the table)
    if (!row || !["point", "prim", "edge"].includes(st.domain)) return;
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
    open() { st.open = true; root.classList.remove("hidden"); syncTabs(); reload(); host.opened(true); },
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
  };
  syncTabs();
  return api;
};
