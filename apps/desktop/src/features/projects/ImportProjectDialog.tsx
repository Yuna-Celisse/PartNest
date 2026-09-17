import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Dialog } from "../../components/ui/Overlay";

export type ProjectImportSource = "interactive" | "tabular";

const extensions: Record<ProjectImportSource, string[]> = {
  interactive: ["html"],
  tabular: ["csv", "xls", "xlsx"],
};

const sources: { value: ProjectImportSource; label: string; short: string; hint: string }[] = [
  { value: "interactive", label: "从可交互式 BOM 导入", short: "可交互式 BOM（HTML）", hint: "EasyEDA 导出的 HTML，带 3D 板位与顶/底层信息" },
  { value: "tabular", label: "从普通 BOM 表导入", short: "普通 BOM 表（CSV / XLS / XLSX）", hint: "器件清单表格，按列解析位号与数量" },
];

const matches = (path: string, source: ProjectImportSource): boolean =>
  extensions[source].includes(path.slice(path.lastIndexOf(".") + 1).toLowerCase());

export const defaultPickFile = async (source: ProjectImportSource): Promise<string | null> => {
  const selected = await open({ multiple: false, filters: [{ name: "BOM", extensions: extensions[source] }] });
  return typeof selected === "string" ? selected : null;
};

export const defaultPickCompanionFile = async (): Promise<string | null> => {
  const selected = await open({ multiple: false, filters: [{ name: "CSV", extensions: ["csv"] }] });
  return typeof selected === "string" ? selected : null;
};

/**
 * One import means one source: the operator picks either the interactive HTML
 * export or a plain table, then confirms. Nothing is analysed or stored until
 * 确定导入 is pressed, so a cancelled dialog leaves the workspace untouched.
 */
export function ImportProjectDialog({ supplement, pickFile, pickCompanionFile, onCancel, onConfirm }: {
  /** Set when completing an existing project instead of starting a new one. */
  supplement?: { kind: ProjectImportSource; projectName: string };
  pickFile: (source: ProjectImportSource) => Promise<string | null>;
  pickCompanionFile: () => Promise<string | null>;
  onCancel: () => void;
  onConfirm: (input: { path: string; companionPath?: string }) => void;
}) {
  const [source, setSource] = useState<ProjectImportSource>(supplement?.kind ?? "interactive");
  const [path, setPath] = useState("");
  const [companionPath, setCompanionPath] = useState("");
  const mismatch = Boolean(path) && !matches(path, source);
  const option = sources.find((entry) => entry.value === source)!;

  function chooseSource(next: ProjectImportSource) {
    if (next === source) return;
    // A different source means a different file kind, so the old pick is gone.
    setSource(next); setPath(""); setCompanionPath("");
  }

  return <Dialog open title={supplement ? "补充导入" : "导入项目"} onRequestClose={onCancel}>
    <div className="project-import-form">
      {supplement
        ? <p className="project-import-note">为项目「{supplement.projectName}」补充{option.short}：{option.hint}</p>
        : <fieldset className="project-import-source">
          <legend>导入来源（二选一）</legend>
          {sources.map((entry) => <label key={entry.value} className="project-import-source__option" data-selected={source === entry.value || undefined}>
            <input type="radio" name="project-import-source" value={entry.value} checked={source === entry.value} onChange={() => chooseSource(entry.value)} />
            <span><strong>{entry.label}</strong><small>{entry.hint}</small></span>
          </label>)}
        </fieldset>}
      <div className="project-import-file">
        <span className="project-import-file__name" title={path}>{path ? path.split(/[\\/]/).pop() : "尚未选择文件"}</span>
        <button className="pn-button pn-button--secondary" type="button" onClick={() => void pickFile(source).then((selected) => { if (selected) { setPath(selected); setCompanionPath(""); } })}>选择文件</button>
        {!supplement && source === "interactive" && <button className="pn-button pn-button--ghost" type="button" disabled={!path} onClick={() => void pickCompanionFile().then((selected) => selected && setCompanionPath(selected))}>配套表格</button>}
      </div>
      {!supplement && source === "interactive" && companionPath && <p className="project-import-file__companion" title={companionPath}>配套：{companionPath.split(/[\\/]/).pop()}</p>}
      {mismatch && <p className="pn-inline-error" role="alert">所选文件与导入来源不符，请重新选择。</p>}
      <div className="project-import-actions">
        <button className="pn-button pn-button--ghost" type="button" onClick={onCancel}>取消</button>
        <button className="pn-button pn-button--primary" type="button" disabled={!path || mismatch} onClick={() => onConfirm({ path, ...(companionPath ? { companionPath } : {}) })}>{supplement ? "确定补充导入" : "确定导入"}</button>
      </div>
    </div>
  </Dialog>;
}
