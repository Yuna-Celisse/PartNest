import bridgeSource from "../../../src-tauri/resources/bridge-v1.js?raw";
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * 回归基线：bridge 脚本运行在不可信的 BOM 文档内，它的每一条选中判定都会直接影响
 * 库存取用，因此规则本身必须被测试约束。加载的是仓库内第一方脚本（`?raw` 导入，
 * 非外部输入），用 Function 构造只为把它当作独立文档脚本执行；每个用例拿到自己的
 * document，避免上一个用例注册的监听器和 MutationObserver 继续向同一个 spy 发消息。
 */
type Spy = ReturnType<typeof vi.spyOn>;

type Harness = { spy: Spy; doc: Document; deliver: (data: unknown, source?: unknown) => void };

function loadBridge(markup: string, designators: string[]): Harness {
  const doc = document.implementation.createHTMLDocument("被测 BOM");
  doc.body.innerHTML = markup;
  const listeners: Array<(event: MessageEvent) => void> = [];
  const config = { token: "token-1", designators };
  /* 脚本把它的 window 作为事件 view 传给 MouseEvent，所以替身必须是真实 Window：
     只改写 parent（上报目标）、注入配置，并截获 message 监听器。 */
  const scope = new Proxy(window, {
    get: (target, property) => {
      if (property === "__PARTNEST_BOM_BRIDGE_V1__") return config;
      if (property === "addEventListener") {
        return (type: string, handler: (event: MessageEvent) => void) => {
          if (type === "message") listeners.push(handler);
        };
      }
      return Reflect.get(target, property, target);
    },
  });
  const spy = vi.spyOn(window, "postMessage").mockImplementation(() => undefined);
  new Function("window", "document", bridgeSource)(scope, doc);
  const deliver = (data: unknown, source: unknown = window) => {
    listeners.forEach((listener) => listener({ source, data } as MessageEvent));
  };
  return { spy, doc, deliver };
}

async function reportedDesignators(spy: Spy): Promise<string[][]> {
  await new Promise((resolve) => window.requestAnimationFrame(() => resolve(null)));
  await new Promise((resolve) => window.requestAnimationFrame(() => resolve(null)));
  return spy.mock.calls
    .map(([message]) => (message as { designators?: string[] }).designators ?? [])
    .filter((designators) => designators.length > 0);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("交互式 BOM bridge", () => {
  it("在操作者交互之前保持沉默", async () => {
    const { spy, doc } = loadBridge(`<div class="selected"><span>R1</span></div>`, ["R1", "R2"]);

    window.dispatchEvent(new Event("load"));
    doc.querySelectorAll("span")[0].setAttribute("class", "selected active");

    expect(await reportedDesignators(spy)).toEqual([]);
  });

  it("忽略只是包含 selected 字样的类名", async () => {
    const { spy, doc } = loadBridge(`
      <span class="deselected">R1</span>
      <table><tbody><tr><td>R2</td></tr></tbody></table>`, ["R1", "R2"]);

    doc.querySelectorAll("tr")[0].click();

    expect(await reportedDesignators(spy)).toEqual([["R2"]]);
  });

  it("点击任意位置后上报被标记的那一行", async () => {
    const { spy, doc } = loadBridge(`
      <table><tbody>
        <tr class="selected"><td data-designator="R1">R1</td></tr>
        <tr><td data-designator="R2">R2</td></tr>
      </tbody></table>`, ["R1", "R2"]);

    doc.querySelectorAll("tr")[1].click();

    expect(await reportedDesignators(spy)).toEqual([["R1"]]);
  });

  it("超过上限的选择一律不发送", async () => {
    const designators = Array.from({ length: 513 }, (_, index) => `R${index}`);
    const { spy, doc } = loadBridge(designators
      .map((value) => `<span class="selected" data-designator="${value}">${value}</span>`)
      .join(""), designators);

    doc.querySelectorAll("span")[0].click();

    expect(await reportedDesignators(spy)).toEqual([]);
  });

  it("只上报属于当前 BOM 的位号", async () => {
    const { spy, doc } = loadBridge(`<table><tbody><tr class="selected"><td data-designator="R9">R9</td><td data-designator="R1">R1</td></tr></tbody></table>`, ["R1"]);

    doc.querySelector("td")?.click();

    expect(await reportedDesignators(spy)).toEqual([["R1"]]);
  });

  it("只保留画布：隐藏查看器自带的表格与表头", () => {
    const { doc } = loadBridge(`
      <div class="header">视图 工具 导出</div>
      <table><tbody><tr>
        <td class="left"><div class="selected"><span>R1</span></div></td>
        <td class="stage"><div class="wrap"><div class="inner"><canvas id="smt-engine-canvas"></canvas></div></div></td>
      </tr></tbody></table>`, ["R1"]);
    const hidden = (selector: string) => (doc.querySelector(selector) as HTMLElement | null)?.style.display === "none";

    expect(hidden(".header")).toBe(true);
    expect(hidden(".left")).toBe(true);

    const canvas = doc.getElementById("smt-engine-canvas");
    expect(canvas).not.toBeNull();
    for (let node = canvas as Element | null; node && node !== doc.body; node = node.parentElement) {
      expect((node as HTMLElement).style.display).not.toBe("none");
    }
  });

  it("宿主选中后点选查看器自己的行以持续高亮", () => {
    const { doc, deliver } = loadBridge(`<table><tbody>
      <tr data-row="a"><td></td><td>CN3、CN1</td><td>2个器件</td></tr>
      <tr data-row="b"><td></td><td>R1</td><td>1个器件</td></tr>
    </tbody></table>`, ["R1", "CN1", "CN3"]);
    const clicks: string[] = [];
    doc.querySelectorAll("tr").forEach((row) => row.addEventListener("click", () => clicks.push(row.dataset.row || "")));

    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["CN1", "CN3"] });

    expect(clicks).toEqual(["a"]);
    expect(doc.querySelector('[data-row="a"]')?.classList.contains("partnest-highlight")).toBe(true);

    clicks.length = 0;
    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["R1"] });
    expect(clicks).toEqual(["b"]);
    expect(doc.querySelector('[data-row="a"]')?.classList.contains("partnest-highlight")).toBe(false);
  });

  it("令牌不符、来源不符或位号不属于本 BOM 时不驱动画布", () => {
    const { doc, deliver } = loadBridge(`<table><tbody><tr class="row-a"><td>R1</td></tr></tbody></table>`, ["R1"]);
    const clicks: string[] = [];
    doc.querySelector("tr")?.addEventListener("click", () => clicks.push("clicked"));

    deliver({ type: "partnest:bom-highlight", token: "forged", designators: ["R1"] });
    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["R1"] }, { });
    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["ZZ9"] });
    deliver({ type: "other:message", token: "token-1", designators: ["R1"] });

    expect(clicks).toEqual([]);
  });

  it("驱动行高亮的合成点击不会被当成操作者的选择上报", async () => {
    const { spy, doc, deliver } = loadBridge(`<table><tbody>
      <tr class="selected"><td>R1</td><td>已选中行</td></tr>
    </tbody></table>`, ["R1"]);

    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["R1"] });

    expect(await reportedDesignators(spy)).toEqual([]);
  });

  it("宿主选择带板面时同时翻转查看器的顶层/底层视角", () => {
    const { doc, deliver } = loadBridge(`<div class="nav-bar layer-switch">
        <div class="nav-bar-item is-select">顶层</div>
        <div class="nav-bar-item">底层</div>
      </div>
      <table><tbody><tr><td>顶层</td><td>R1<span>1个器件</span><span>详情</span></td></tr></tbody></table>`, ["R1"]);
    const clicks: string[] = [];
    doc.querySelectorAll(".nav-bar-item, td").forEach((node) => node.addEventListener("click", () => {
      clicks.push(`${node.textContent?.trim()}@${node.tagName}`);
    }));

    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["R1"], side: "bottom" });

    expect(clicks).toContain("底层@DIV");
    expect(clicks).not.toContain("顶层@DIV");
    // 翻面会重建表格，行点击延后到它稳定之后
    expect(clicks.filter((entry) => entry.endsWith("@TD"))).toEqual([]);
  });

  it("已经在目标板面时不重复翻面，直接选中行", () => {
    const { doc, deliver } = loadBridge(`<div class="nav-bar layer-switch">
        <div class="nav-bar-item">顶层</div>
        <div class="nav-bar-item is-select">底层</div>
      </div>
      <table><tbody><tr>
        <td><input type="checkbox"><span></span></td>
        <td><p><span>C1</span></p><p><span>1个器件</span><span>详情</span></p></td>
        <td><p>100nF<span>详情</span></p></td>
      </tr></tbody></table>`, ["C1"]);
    const clicks: string[] = [];
    doc.querySelectorAll(".nav-bar-item, td").forEach((node) => node.addEventListener("click", () => {
      const first = node.firstElementChild ? node.firstElementChild.textContent?.trim() ?? "" : "";
      clicks.push(`${node.tagName}:${first.slice(0, 8)}`);
    }));

    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["C1"], side: "bottom" });

    // 位号在首块里，后面还跟着 "1个器件""详情"，不能被拼成 C11
    expect(clicks).toEqual(["TD:C1"]);
    expect(doc.querySelector("tr")?.classList.contains("partnest-highlight")).toBe(true);
  });

  it("跳过把整棵应用套进一格里的布局单元格", () => {
    const { doc, deliver } = loadBridge(`<table>
      <tbody><tr><td><div><canvas></canvas><p>C1</p><p>R1</p></div></td></tr></tbody>
      <table><tbody><tr><td><p><span>R1</span></p><p>1个器件</p></td></tr></tbody></table>
    </table>`, ["C1", "R1"]);
    const clicks: string[] = [];
    doc.querySelectorAll("td").forEach((node) => node.addEventListener("click", () => {
      clicks.push(node.querySelector("canvas") ? "layout" : "data");
    }));

    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["R1"] });

    expect(clicks).toEqual(["data"]);
  });

  it("选中不会自动缩放，缩放与复位由显式指令触发", async () => {
    const { doc, deliver } = loadBridge(`<ul class="nav-bar">
        <li title="适应选中" class="nav-bar-item">适应选中</li>
        <li title="适应全部(K)" class="nav-bar-item">适应全部</li>
      </ul>
      <table><tbody><tr><td><p><span>R1</span></p><p>1个器件</p></td></tr></tbody></table>`, ["R1"]);
    const clicks: string[] = [];
    doc.querySelectorAll("li, td").forEach((node) => node.addEventListener("click", () => {
      clicks.push(node.tagName === "LI" ? `fit:${node.getAttribute("title")}` : "row");
    }));

    deliver({ type: "partnest:bom-highlight", token: "token-1", designators: ["R1"] });
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(clicks).toEqual(["row"]);

    deliver({ type: "partnest:bom-view", token: "token-1", action: "fit" }, window);
    expect(clicks).toEqual(["row", "fit:适应选中"]);

    deliver({ type: "partnest:bom-view", token: "token-1", action: "reset" }, window);
    expect(clicks).toEqual(["row", "fit:适应选中", "fit:适应全部(K)"]);

    deliver({ type: "partnest:bom-view", token: "token-1", action: "drop-table" }, window);
    deliver({ type: "partnest:bom-view", token: "forged", action: "fit" }, window);
    expect(clicks.length).toBe(3);
  });

  it("没有画布的文档不做任何隐藏", () => {
    const { doc } = loadBridge(`<div class="header">视图</div><span class="row">R1</span>`, ["R1"]);

    expect((doc.querySelector(".header") as HTMLElement).style.display).not.toBe("none");
    expect((doc.querySelector(".row") as HTMLElement).style.display).not.toBe("none");
  });
});
