/* 3-D viewport chrome, laid out like Blender's: a header with View / Select / Add menus along the top, the tools down the
 * left and the display settings in a panel on the right. This file only handles the menus and the panel toggle; every
 * control keeps its id and is wired in editor.js. Menu items marked data-pv-proxy click the control they name. */
(function () {
  "use strict";
  const $ = (s) => document.querySelector(s);
  const store = {
    get(k) { try { return localStorage.getItem(k); } catch (e) { return null; } },
    set(k, v) { try { localStorage.setItem(k, v); } catch (e) { /* private window */ } },
  };
  const buttons = [...document.querySelectorAll("[data-pv-menu]")];
  let open = null;

  function close(restoreFocus) {
    if (!open) return;
    open.pop.classList.add("hidden");
    open.owners.forEach((b) => b.setAttribute("aria-expanded", "false"));
    if (restoreFocus) open.btn.focus();
    open = null;
  }
  function show(btn, focus) {
    close();
    const pop = document.getElementById(btn.dataset.pvMenu);
    if (!pop) return;
    const owners = buttons.filter((b) => b.dataset.pvMenu === btn.dataset.pvMenu);
    owners.forEach((b) => b.setAttribute("aria-expanded", "true"));
    pop.classList.remove("hidden");
    const r = btn.getBoundingClientRect(), w = pop.offsetWidth;
    pop.style.left = Math.max(4, Math.min(r.left, innerWidth - w - 4)) + "px";
    pop.style.top = r.bottom + 1 + "px";
    open = { btn, pop, owners };
    if (focus) pop.querySelector("button:not(:disabled)")?.focus();
  }
  buttons.forEach((b) => {
    b.addEventListener("click", () => (open && open.btn === b ? close() : show(b, false)));
    b.addEventListener("pointerenter", () => { if (open && open.btn !== b) show(b, false); });
    b.addEventListener("keydown", (e) => { if (e.key === "ArrowDown") { e.preventDefault(); show(b, true); } });
  });
  document.querySelectorAll(".pv-menu").forEach((pop) => {
    pop.addEventListener("click", (e) => {
      const item = e.target.closest("button");
      if (!item) return;
      const target = item.dataset.pvProxy && $(item.dataset.pvProxy);
      close();
      if (target) target.click();
    });
    pop.addEventListener("change", close);                      // picking from a dropdown inside a menu finishes it too
    pop.addEventListener("keydown", (e) => {
      if (!["ArrowDown", "ArrowUp"].includes(e.key)) return;
      const items = [...pop.querySelectorAll("button:not(:disabled)")], at = items.indexOf(document.activeElement);
      items[(at + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
      e.preventDefault();
    });
  });
  document.addEventListener("pointerdown", (e) => { if (open && !open.pop.contains(e.target) && !e.target.closest("[data-pv-menu]")) close(); }, true);
  document.addEventListener("keydown", (e) => { if (open && e.key === "Escape") { close(true); e.stopPropagation(); } }, true);
  window.addEventListener("blur", () => close());

  // the display settings panel on the right (Blender's side panel): shown by default, remembered per browser
  const side = $("#pv-side"), sideBtn = $("#pv-side-btn");
  function setSide(on) {
    side.classList.toggle("hidden", !on);
    sideBtn.classList.toggle("on", on);
    sideBtn.setAttribute("aria-pressed", String(on));
    store.set("minihou.pvSide", on ? "1" : "0");
    window.dispatchEvent(new Event("resize"));                 // the canvas gets the room back
  }
  sideBtn.addEventListener("click", () => setSide(side.classList.contains("hidden")));
  if (store.get("minihou.pvSide") === "0") setSide(false);

  // remember which sections of the panel are folded
  document.querySelectorAll("#pv-side details.pv-sec").forEach((d, i) => {
    const key = "minihou.pvSec." + i, saved = store.get(key);
    if (saved !== null) d.open = saved === "1";
    d.addEventListener("toggle", () => store.set(key, d.open ? "1" : "0"));
  });
})();
