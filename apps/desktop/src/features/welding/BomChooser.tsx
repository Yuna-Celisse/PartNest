import { Dialog } from "../../components/ui/Overlay";
import { StatusBadge } from "../../components/ui/StatusBadge";
import type { BomFileSummary } from "../../app/tauri";

export type BomChooserProps = {
  open: boolean;
  files: BomFileSummary[];
  busy: boolean;
  error: string;
  onRequestClose: () => void;
  onPick: (file: BomFileSummary) => void;
};

/** Pick one of the imported BOMs as the welding session. */
export function BomChooser({ open, files, busy, error, onRequestClose, onPick }: BomChooserProps) {
  return <Dialog open={open} title="选择 BOM" onRequestClose={onRequestClose}>
    {error && <p className="pn-inline-error welding-chooser__error" role="alert">{error}</p>}
    {files.length === 0 ? <p className="welding-chooser__empty">暂无已导入 BOM，请先在「BOM 分析」页导入。</p> : <ul className="welding-chooser__list">
      {files.map((file) => <li key={file.id}>
        <button type="button" className="welding-chooser__item" disabled={busy} onClick={() => onPick(file)}>
          <span className="welding-chooser__names"><strong>{file.display_name}</strong><span>{file.original_name}</span></span>
          {file.active && <StatusBadge tone="success">活动</StatusBadge>}
        </button>
      </li>)}
    </ul>}
  </Dialog>;
}
