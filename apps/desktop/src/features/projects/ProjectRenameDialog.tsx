import { useState } from "react";
import { Dialog } from "../../components/ui/Overlay";
import { FormField } from "../../components/ui/FormField";

/** Rename a project. The imported file name is kept separately as its origin. */
export function ProjectRenameDialog({ project, busy, onCancel, onSubmit }: {
  project: { id: string; name: string };
  busy: boolean;
  onCancel: () => void;
  onSubmit: (name: string) => void;
}) {
  const [name, setName] = useState(project.name);
  const trimmed = name.trim();
  return <Dialog open title="重命名项目" onRequestClose={onCancel}>
    <div className="project-rename-form">
      <FormField label="项目名称" htmlFor="project-name">
        <input
          id="project-name"
          className="pn-control"
          aria-label="项目名称"
          value={name}
          autoFocus
          onChange={(event) => setName(event.target.value)}
          onKeyDown={(event) => { if (event.key === "Enter" && trimmed && !busy) onSubmit(trimmed); }}
        />
      </FormField>
      <div className="project-rename-actions">
        <button className="pn-button pn-button--ghost" type="button" onClick={onCancel}>取消</button>
        <button className="pn-button pn-button--primary" type="button" disabled={!trimmed || busy} onClick={() => onSubmit(trimmed)}>保存</button>
      </div>
    </div>
  </Dialog>;
}
