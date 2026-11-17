import { useEffect, useState } from "react";
import type { ProjectPackage } from "../../app/contracts";

export function ProjectPackagePanel({ projectId, load }: { projectId: string; load?: (id: string) => Promise<ProjectPackage | null> }) {
  const [data, setData] = useState<ProjectPackage | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    let live = true;
    setData(null); setError("");
    if (load) void load(projectId).then((result) => { if (live) setData(result); }).catch(() => { if (live) setError("读取工程包资料失败，请重新打开项目"); });
    return () => { live = false; };
  }, [projectId, load]);
  if (error) return <p role="alert">{error}</p>;
  if (!data) return null;
  return <section className="project-package" aria-label="KiCad 工程资料">
    <h3>KiCad 工程资料 · {data.files.length} 个文件</h3>
    <div className="project-package__content">
      <p>BOM 来源：{data.bom_source}</p>
      <p>PCB 3D 预览与器件位置请在焊接工作台中查看。</p>
      {data.warnings.map((warning) => <p key={warning}>{warning}</p>)}
      <details open><summary>工程文件清单</summary><ul>{data.files.map((file) => <li key={file.path}>{file.kind} · {file.path} · {(file.size / 1024).toFixed(1)} KiB</li>)}</ul></details>
    </div>
  </section>;
}
