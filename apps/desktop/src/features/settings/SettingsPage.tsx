import { useState } from "react";
import { open, confirm } from "@tauri-apps/plugin-dialog";
import { desktopApi, errorMessage, type DesktopApi } from "../../app/tauri";
import { StatusBadge } from "../../components/ui/StatusBadge";

export type SettingsApi = Pick<DesktopApi, "createBackup" | "restoreBackup" | "resetData">;

export async function pickBackupFile(): Promise<string | null> {
  const selected = await open({ multiple: false, filters: [{ name: "PartNest 备份", extensions: ["db"] }] });
  return typeof selected === "string" ? selected : null;
}
export function SettingsPage({ api = desktopApi, pickFile = pickBackupFile }: { api?: SettingsApi; pickFile?: () => Promise<string | null> }) {
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");

  async function reset() {
    if (busy) return;
    setBusy(true); setError(""); setNotice("");
    try {
      const accepted = await confirm("将清空全部库存（含已删除器件）、收纳盒、库存流水、BOM 记录、焊接会话和进度，以及 LCSC 查询缓存。执行前自动备份；已有备份和 BOM 缓存文件会保留。需要恢复时可使用备份。确定重置？", {
        title: "重置数据", kind: "warning", okLabel: "确认重置", cancelLabel: "取消",
      });
      if (!accepted) return;
      const path = await api.resetData();
      setNotice(`数据已重置，重置前备份：${path}`);
    } catch (cause) { setError(errorMessage(cause)); }
    finally { setBusy(false); }
  }

  async function create() {
    setBusy(true); setError(""); setNotice("");
    try {
      const path = await api.createBackup();
      setNotice(`备份已创建：${path}`);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally { setBusy(false); }
  }

  async function restore() {
    const path = await pickFile();
    if (!path) return;
    setBusy(true); setError(""); setNotice("");
    try {
      await api.restoreBackup(path);
      setNotice("已恢复备份，请重新打开需要刷新的页面");
    } catch (cause) {
      setError(errorMessage(cause));
    } finally { setBusy(false); }
  }

  return <section className="operations-page settings-page" aria-label="设置">
    <section className="settings-group" aria-labelledby="backup-title">
      <h2 id="backup-title">数据与备份</h2>
      <button className="pn-button pn-button--primary" type="button" disabled={busy} onClick={() => void create()}>立即备份</button>
      <button className="pn-button pn-button--secondary" type="button" disabled={busy} onClick={() => void restore()}>选择备份恢复</button>
      <p className="settings-risk">恢复会覆盖当前数据。</p>
    </section>
    <section className="settings-group" aria-labelledby="interface-title">
      <h2 id="interface-title">界面</h2>
      <p className="settings-value"><span>主题</span><StatusBadge tone="active">深色主题</StatusBadge></p>
    </section>
    <section className="settings-group" aria-labelledby="reset-title">
      <h2 id="reset-title">重置数据</h2>
      <p className="settings-risk">清空库存、收纳盒、流水、BOM 记录与焊接进度，以及 LCSC 查询缓存。执行前自动备份，保留已有备份和 BOM 缓存文件。</p>
      <button className="pn-button pn-button--secondary" type="button" disabled={busy} onClick={() => void reset()}>重置数据</button>
    </section>
    {notice && <p role="status">{notice}</p>}
    {error && <p role="alert">{error}</p>}
  </section>;
}
