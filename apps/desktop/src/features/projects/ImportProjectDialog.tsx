import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { Dialog } from "../../components/ui/Overlay";

export type ProjectImportSource = "interactive" | "tabular";

const extensions: Record<ProjectImportSource, string[]> = {
  interactive: ["html"],
  tabular: ["csv", "xls", "xlsx"],
};

const sources: { value: ProjectImportSource; label: string; hint: string }[] = [
  { value: "interactive", label: "可交互式 BOM（HTML）", hint: "EasyEDA 导出的 HTML，带 3D 板位与顶/底层信息" },
  { value: "tabular", label: "普通 BOM 表（CSV / XLS / XLSX）", hint: "器件清单表格，按列解析位号与数量；与交互式一起选择时作为它的配套表格" },
];

const fileName = (path: string): string => path.split(/[\\/]/).pop() ?? path;

const matches = (path: string, source: ProjectImportSource): boolean =>
  extensions[source].includes(path.slice(path.lastIndexOf(".") + 1).toLowerCase());

/** A slot is misfiled when its chosen export has an extension it cannot read. */
const isMisfiled = (path: string | undefined, source: ProjectImportSource): boolean =>
  path !== undefined && !matches(path, source);

export const defaultPickFile = async (source: ProjectImportSource): Promise<string | null> => {
  const selected = await open({ multiple: false, filters: [{ name: "BOM", extensions: extensions[source] }] });
  return typeof selected === "string" ? selected : null;
};

/**
 * Both files live in one window: pick the interactive export, the table, or both,
 * then create the project. Whichever half is left out can be added later through
 * 补充导入, so nothing here is either/or. Nothing is analysed or stored until
 * the confirm button is pressed, so a cancelled dialog leaves the workspace alone.
 */
export function ImportProjectDialog({ supplement, pickFile, onCancel, onConfirm }: {
  /** Set when completing an existing project instead of starting a new one. */
  supplement?: { kind: ProjectImportSource; projectName: string };
  pickFile: (source: ProjectImportSource) => Promise<string | null>;
  onCancel: () => void;
  onConfirm: (files: { interactivePath?: string; tablePath?: string }) => void;
}) {
  const [files, setFiles] = useState<Partial<Record<ProjectImportSource, string>>>({});
  const options = supplement ? sources.filter((entry) => entry.value === supplement.kind) : sources;
  const mismatched = options.filter(({ value }) => isMisfiled(files[value], value));
  const chosen = options.filter(({ value }) => Boolean(files[value]));
  const canSubmit = chosen.length > 0 && mismatched.length === 0;

  function pick(source: ProjectImportSource) {
    void pickFile(source).then((selected) => {
      if (selected) setFiles((current) => ({ ...current, [source]: selected }));
    });
  }

  function confirm() {
    const interactivePath = files.interactive;
    const tablePath = files.tabular;
    onConfirm({ ...(interactivePath ? { interactivePath } : {}), ...(tablePath ? { tablePath } : {}) });
  }

  return <Dialog open title={supplement ? "补充导入" : "导入项目"} onRequestClose={onCancel}>
    <div className="project-import-form">
      {supplement && <p className="project-import-note">为项目「{supplement.projectName}」补充缺失的来源；另一种来源保持不变。</p>}
      {options.map((entry) => {
        const path = files[entry.value];
        const wrong = isMisfiled(path, entry.value);
        return <div key={entry.value} className="project-import-source" data-chosen={path ? true : undefined}>
          <div className="project-import-source__text">
            <strong>{entry.label}</strong>
            <small>{entry.hint}</small>
          </div>
          <div className="project-import-file">
            <span className="project-import-file__name" title={path}>{path ? fileName(path) : "尚未选择文件"}</span>
            <button className="pn-button pn-button--secondary" type="button" aria-label={`选择文件：${entry.label}`} onClick={() => pick(entry.value)}>选择文件</button>
            {path && <button className="pn-button pn-button--ghost" type="button" onClick={() => setFiles((current) => ({ ...current, [entry.value]: undefined }))}>清除</button>}
          </div>
          {wrong && <p className="pn-inline-error" role="alert">该文件不是{entry.label}支持的格式，请重新选择。</p>}
        </div>;
      })}
      {!supplement && files.interactive && !files.tabular && <p className="project-import-note">只导交互式也可以，之后能在项目里补充器件表格。</p>}
      {!supplement && files.tabular && !files.interactive && <p className="project-import-note">只导表格也可以，之后能在项目里补充交互式 BOM。</p>}
      <div className="project-import-actions">
        <button className="pn-button pn-button--ghost" type="button" onClick={onCancel}>取消</button>
        <button className="pn-button pn-button--primary" type="button" disabled={!canSubmit} onClick={confirm}>{supplement ? "确定补充导入" : "创建项目"}</button>
      </div>
    </div>
  </Dialog>;
}
