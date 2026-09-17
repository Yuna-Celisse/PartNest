import { Dialog } from "../../components/ui/Overlay";
import { StatusBadge } from "../../components/ui/StatusBadge";
import type { ProjectSummary } from "../../app/tauri";

export type ProjectChooserProps = {
  open: boolean;
  projects: ProjectSummary[];
  busy: boolean;
  error: string;
  onRequestClose: () => void;
  onPick: (project: ProjectSummary) => void;
};

/** Open the welding workspace for one of the imported projects. */
export function ProjectChooser({ open, projects, busy, error, onRequestClose, onPick }: ProjectChooserProps) {
  return <Dialog open={open} title="选择项目" onRequestClose={onRequestClose}>
    {error && <p className="pn-inline-error welding-chooser__error" role="alert">{error}</p>}
    {projects.length === 0 ? <p className="welding-chooser__empty">暂无项目，请先在「项目」页导入 BOM。</p> : <ul className="welding-chooser__list">
      {projects.map((project) => <li key={project.id}>
        <button type="button" className="welding-chooser__item" disabled={busy} onClick={() => onPick(project)}>
          <span className="welding-chooser__names"><strong>{project.name}</strong><span>{project.original_name}</span></span>
          {project.active && <StatusBadge tone="success">焊接中</StatusBadge>}
        </button>
      </li>)}
    </ul>}
  </Dialog>;
}
