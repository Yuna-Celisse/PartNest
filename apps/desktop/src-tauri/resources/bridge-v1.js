/* partnest bridge-v1: report only known designators from the untrusted BOM. */
/* The host receiver must also require event.source === iframe.contentWindow. */
(function (window, document) {
  "use strict";
  const config = window.__PARTNEST_BOM_BRIDGE_V1__;
  if (!config || typeof config.token !== "string" || !Array.isArray(config.designators)) return;

  const known = new Set(config.designators.filter((value) => typeof value === "string" && value));
  if (!known.size) return;
  const targetOrigin = typeof config.targetOrigin === "string" && /^https?:\/\/[^\/\s]+$/.test(config.targetOrigin)
    ? config.targetOrigin
    : "*";
  let lastSelection = "";

  const addTokens = (value, output) => {
    if (typeof value !== "string" || !value) return;
    value.split(/[^A-Za-z0-9_+.-]+/).forEach((token) => {
      if (known.has(token) && !output.includes(token)) output.push(token);
    });
  };

  const attributes = [
    "data-designator", "data-designators", "data-refdes", "data-refdeses",
    "data-reference", "data-name", "id", "title", "aria-label", "name",
  ];

  /* Only whole class tokens count as "selected": a substring match would turn
     `deselected`, `unselected`, or `highlighted` into a selection. */
  const SELECTED_SELECTOR = [
    ".selected", ".is-selected", ".active", ".is-active",
    "[aria-selected='true']", "[data-selected='true']",
    "[data-state='selected']", "[data-active='true']", "[data-highlight='true']",
  ].join(",");

  /* The host caps one message at 512 designators; never emit a truncated set. */
  const MAX_DESIGNATORS = 512;

  const collectNode = (node, output, includeText) => {
    if (!node || node.nodeType !== 1) return;
    attributes.forEach((name) => addTokens(node.getAttribute(name), output));
    const text = node.textContent || "";
    if (includeText && text.length <= 512) addTokens(text, output);
  };

  const collectAncestors = (target, output) => {
    let node = target && target.nodeType === 1 ? target : target && target.parentElement;
    let depth = 0;
    while (node && depth < 6) {
      const marked = node.matches(SELECTED_SELECTOR);
      collectNode(node, output, marked || node.matches("tr, [role='row'], g"));
      if (marked) {
        node.querySelectorAll("[data-designator], [data-refdes], [data-reference], [data-name]").forEach((child) => collectNode(child, output, false));
      }
      node = node.parentElement;
      depth += 1;
    }
  };

  const collectMarked = (output) => {
    document.querySelectorAll(SELECTED_SELECTOR).forEach((node) => collectNode(node, output, true));
  };

  const emit = (target) => {
    const selected = [];
    collectMarked(selected);
    if (!selected.length && target) collectAncestors(target, selected);
    if (!selected.length || selected.length > MAX_DESIGNATORS) return;
    const signature = selected.join("\u0000");
    if (signature === lastSelection) return;
    lastSelection = signature;
    window.parent.postMessage({ type: "partnest:bom-selection", token: config.token, designators: selected }, targetOrigin);
  };

  let frame = 0;
  let lastTarget = null;
  /* Selection only ever follows a real user interaction. Page load and DOM
     mutations alone must not push a selection the operator never made. */
  let interacted = false;
  /* Our own programmatic row click must not come back as a selection report. */
  let synthetic = false;
  const schedule = (target) => {
    if (!interacted) return;
    lastTarget = target || lastTarget;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      const currentTarget = lastTarget;
      lastTarget = null;
      emit(currentTarget);
    });
  };

  const onInteract = (event) => {
    if (synthetic) return;
    interacted = true;
    schedule(event.target);
  };

  /* 宿主选中器件后，驱动查看器自己的那一行，让 3D 高亮对应元件。 */
  const HIGHLIGHT_CLASS = "partnest-highlight";

  /* 位号可能直接挂在单元格文本上，也可能在首个子块里（<td><p><span>C1</span></p><p>1个器件…</p></td>）。
     只认这两种，并且排除把整棵应用套进一个格子的布局单元格。 */
  const cellTokens = (cell) => {
    if (cell.querySelector("canvas")) return [];
    const first = cell.firstElementChild;
    const block = first ? String(first.textContent || "") : "";
    if (block.length > 64) return [];
    const own = Array.prototype.filter.call(cell.childNodes, (child) => child.nodeType === 3)
      .map((child) => child.textContent).join("").trim();
    return (own || block).split(/[^A-Za-z0-9_+.-]+/);
  };

  /* 逐个单元格匹配，命中第一个含该位号的单元格。 */
  const designatorCell = (wanted) => {
    const set = new Set(wanted);
    let found = null;
    document.querySelectorAll("tr").forEach((row) => {
      if (found) return;
      Array.prototype.forEach.call(row.children, (cell) => {
        if (found) return;
        if (cellTokens(cell).some((token) => set.has(token))) found = { row, cell };
      });
    });
    return found;
  };

  const clickSynthetic = (node) => {
    const point = { bubbles: true, cancelable: true };
    synthetic = true;
    try {
      ["pointerdown", "pointerup", "pointerover", "mousedown", "mouseup", "click"].forEach((type) => {
        const Ctor = type.indexOf("pointer") === 0 ? window.PointerEvent : window.MouseEvent;
        if (typeof Ctor !== "function") return;
        node.dispatchEvent(new Ctor(type, point));
      });
    } finally {
      synthetic = false;
    }
  };

  const leafText = (node) => Array.prototype.filter.call(node.childNodes, (child) => child.nodeType === 3)
    .map((child) => child.textContent.trim()).join("");

  const LAYER_LABEL = { top: "顶层", bottom: "底层" };

  const layerItems = (label) => {
    const items = [];
    document.querySelectorAll("div,span,li,a,button").forEach((node) => {
      if (leafText(node) !== label) return;
      const parent = node.parentElement;
      if (/layer-switch|nav-bar/.test(String(node.className)) || /layer-switch|nav-bar/.test(String(parent && parent.className))) {
        items.push(node);
      }
    });
    return items;
  };

  /* 查看器用 is-select 标记当前面；已经是目标面时不再点击，避免无谓重渲染。 */
  const needsLayerSwitch = (side) => {
    const label = LAYER_LABEL[side];
    if (!label) return false;
    const items = layerItems(label);
    return items.length > 0 && !items.some((node) => /is-select/.test(String(node.className)));
  };

  const switchLayer = (side) => {
    const label = LAYER_LABEL[side];
    if (!label) return;
    const hit = layerItems(label)[0] || null;
    if (hit) clickSynthetic(hit);
  };

  const whenSettled = (run, delay) => {
    if (typeof window.setTimeout === "function") window.setTimeout(run, delay);
    else run();
  };

  /* 缩放居中/复位都走查看器自己的工具栏；自动缩放会在退化包围盒上冲到极端，所以交给操作者点。 */
  const VIEW_CONTROLS = { fit: "适应选中", reset: "适应全部(K)" };

  const applyView = (action) => {
    const title = VIEW_CONTROLS[action];
    if (!title) return;
    const button = document.querySelector(`[title="${title}"]`);
    if (button) clickSynthetic(button);
  };

  const applyHighlight = (wanted, side) => {
    document.querySelectorAll("." + HIGHLIGHT_CLASS).forEach((node) => node.classList.remove(HIGHLIGHT_CLASS));
    const selectRow = () => {
      const hit = designatorCell(wanted);
      if (!hit) return;
      hit.row.classList.add(HIGHLIGHT_CLASS);
      clickSynthetic(hit.cell);
    };
    if (side && needsLayerSwitch(side)) {
      // 翻面会重建表格，所以等它稳定后再点行，否则刚选中就被重渲染冲掉。
      switchLayer(side);
      whenSettled(selectRow, 120);
      return;
    }
    selectRow();
  };

  const fromHost = (event) => Boolean(event && event.source === window.parent)
    && Boolean(event.data && typeof event.data === "object")
    && typeof event.data.token === "string" && event.data.token === config.token;

  window.addEventListener("message", (event) => {
    if (!fromHost(event)) return;
    const data = event.data;
    if (data.type === "partnest:bom-view") {
      applyView(data.action);
      return;
    }
    if (data.type !== "partnest:bom-highlight") return;
    if (!Array.isArray(data.designators)) return;
    const wanted = data.designators.filter((value) => typeof value === "string" && known.has(value));
    const side = data.side === "top" || data.side === "bottom" ? data.side : null;
    if (wanted.length && wanted.length <= MAX_DESIGNATORS) applyHighlight(wanted, side);
  });

  /* PartNest 只把这块区域当作 3D 板子看：隐藏查看器自带的表头与元件表，让它
     自己的弹性布局把画布撑满。用隐藏而非移除，搜索、行点击等原生行为仍可用。 */
  const HIDDEN_FLAG = "partnestHiddenChrome";
  const boardCanvas = () => {
    const direct = document.getElementById("smt-engine-canvas");
    if (direct) return direct;
    let largest = null;
    document.querySelectorAll("canvas").forEach((node) => {
      const area = node.clientWidth * node.clientHeight;
      if (area > 0 && (!largest || area > largest.clientWidth * largest.clientHeight)) largest = node;
    });
    return largest;
  };

  const hideChrome = () => {
    const canvas = boardCanvas();
    if (!canvas) return false;
    const keep = new Set();
    for (let node = canvas; node; node = node.parentElement) keep.add(node);
    keep.add(document.body);
    let changed = false;
    keep.forEach((node) => {
      const parent = node.parentElement;
      if (!parent) return;
      Array.prototype.forEach.call(parent.children, (sibling) => {
        if (keep.has(sibling) || sibling.tagName === "SCRIPT" || sibling.tagName === "STYLE") return;
        if (sibling.dataset[HIDDEN_FLAG] === "1") return;
        sibling.style.setProperty("display", "none", "important");
        sibling.dataset[HIDDEN_FLAG] = "1";
        changed = true;
      });
    });
    return changed;
  };

  let pruneTimer = 0;
  const prune = () => {
    if (!hideChrome()) return;
    /* The engine sizes the WebGL viewport from its container on resize. */
    if (typeof window.Event === "function") window.dispatchEvent(new window.Event("resize"));
  };

  prune();
  if (typeof MutationObserver === "function" && typeof window.setTimeout === "function") {
    const chromeObserver = new MutationObserver(() => {
      window.clearTimeout(pruneTimer);
      pruneTimer = window.setTimeout(prune, 120);
    });
    chromeObserver.observe(document.documentElement || document, { childList: true, subtree: true });
  }

  document.addEventListener("click", onInteract, true);
  document.addEventListener("change", onInteract, true);
  if (typeof MutationObserver === "function") {
    const observer = new MutationObserver(() => schedule(null));
    observer.observe(document.documentElement || document, { subtree: true, attributes: true, attributeFilter: ["class", "aria-selected", "data-selected", "data-state", "data-active", "data-designator"] });
  }
}(window, document));
