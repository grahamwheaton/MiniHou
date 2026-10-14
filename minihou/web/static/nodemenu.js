/* The Add Node menu's data logic: group the node types, rank a search. The editor draws the result.
 *
 * Types arrive from the engine with `menu` (group), `menu_rank`, `menu_order`, `label` and `keywords` (see minihou/menu.py). */
(function () {
  "use strict";
  const low = (s) => String(s || "").toLowerCase();
  const words = (s) => low(s).split(/[^a-z0-9]+/).filter(Boolean);
  const tokens = (q) => low(q).split(/[\s,_]+/).filter(Boolean);

  /* How well one token fits a node (0 best); null when it appears nowhere. */
  function tokenScore(t, tok) {
    const label = low(t.label || t.name), name = low(t.name).replace(/_/g, " ");
    if (label === tok || name === tok) return 0;
    if (label.startsWith(tok) || name.startsWith(tok)) return 1;
    if (words(label).some((w) => w.startsWith(tok)) || words(name).some((w) => w.startsWith(tok))) return 2;
    if (label.includes(tok) || name.includes(tok)) return 3;
    if (words(t.keywords).some((w) => w === tok)) return 4;
    if (words(t.keywords).some((w) => w.startsWith(tok))) return 5;
    if (low(t.menu).includes(tok) || low(t.category).includes(tok)) return 6;
    if (low(t.doc).includes(tok)) return 7;
    return null;
  }

  /* Every token must match; the score adds up, a shorter name wins a tie. */
  function score(t, toks) {
    let total = 0;
    for (const tok of toks) {
      const s = tokenScore(t, tok);
      if (s === null) return null;
      total += s;
    }
    // Keep standalone tools first unless the user asks for Blender/native algorithms.
    if (t.menu === "Blender Modelling" && !toks.some(tok => tok === "blender" || tok === "native")) total += 4;
    return total + Math.min(String(t.label || t.name).length, 40) / 100;
  }

  function search(types, query) {
    const toks = tokens(query);
    if (!toks.length) return types.slice();
    return types.map((t) => [score(t, toks), t]).filter(([s]) => s !== null)
      .sort((a, b) => a[0] - b[0] || String(a[1].label || a[1].name).localeCompare(String(b[1].label || b[1].name))).map(([, t]) => t);
  }

  /* Types in menu order: group rank, then the group's own order, then name. */
  function ordered(types) {
    return types.slice().sort((a, b) => (a.menu_rank - b.menu_rank) || (a.menu_order - b.menu_order) ||
      String(a.label || a.name).localeCompare(String(b.label || b.name)));
  }

  /* The groups with their types: [{name, items}] in menu order; empty groups are left out. */
  function grouped(types) {
    const out = [], by = new Map();
    for (const t of ordered(types)) {
      const g = t.menu || "Library";
      if (!by.has(g)) { const e = { name: g, items: [] }; by.set(g, e); out.push(e); }
      by.get(g).items.push(t);
    }
    return out;
  }

  /* The rows the menu shows. With no query: Favourites, Recent, then every group (its items only when open). With a
   * query: the ranked matches, flat. prefs = {favs: Set, recent: [names], open: Set of open group names}. */
  function rows(types, query, prefs) {
    const toks = tokens(query);
    if (toks.length) return search(types, query).map((t) => ({ kind: "item", type: t, showGroup: true }));
    const byName = new Map(types.map((t) => [t.name, t]));
    const out = [];
    const favs = [...prefs.favs].map((n) => byName.get(n)).filter(Boolean);
    if (favs.length) {
      out.push({ kind: "head", name: "Favourites", special: true, count: favs.length, open: true });
      favs.forEach((t) => out.push({ kind: "item", type: t, showGroup: true }));
    }
    const recent = prefs.recent.map((n) => byName.get(n)).filter((t) => t && !prefs.favs.has(t.name)).slice(0, 6);
    if (recent.length) {
      out.push({ kind: "head", name: "Recent", special: true, count: recent.length, open: true });
      recent.forEach((t) => out.push({ kind: "item", type: t, showGroup: true }));
    }
    for (const g of grouped(types)) {
      const open = prefs.open.has(g.name);
      out.push({ kind: "head", name: g.name, count: g.items.length, open });
      if (open) g.items.forEach((t) => out.push({ kind: "item", type: t }));
    }
    return out;
  }

  /* The label split around the first match of any token: [[text, matched?], ...]. */
  function highlight(text, query) {
    const toks = tokens(query).filter((t) => t.length > 0);
    const s = String(text), l = low(s);
    let best = null;
    for (const tok of toks) {
      const i = l.indexOf(tok);
      if (i >= 0 && (best === null || i < best[0])) best = [i, tok.length];
    }
    if (!best) return [[s, false]];
    return [[s.slice(0, best[0]), false], [s.slice(best[0], best[0] + best[1]), true], [s.slice(best[0] + best[1]), false]].filter((p) => p[0]);
  }

  window.NodeMenu = { search, grouped, ordered, rows, highlight, score, tokens };
})();
