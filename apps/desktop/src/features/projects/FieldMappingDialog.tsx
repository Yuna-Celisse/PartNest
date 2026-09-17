import type { FieldMapping, FieldName } from "./useProjectImport";
import { Dialog } from "../../components/ui/Overlay";
import { FormField } from "../../components/ui/FormField";

const labels: Record<FieldName, string> = {
  quantity: "BOM 数量", designators: "位号", name: "器件", value: "值", package: "封装",
  manufacturer: "制造商", mpn: "MPN", lcsc_code: "LCSC", side: "板面",
};

export function FieldMappingDialog({
  headers, mapping, onChange, onConfirm, onCancel,
}: {
  headers: string[];
  mapping: FieldMapping;
  onChange: (mapping: FieldMapping) => void;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const fields = Object.keys(labels) as FieldName[];
  function setField(field: FieldName, source: string) {
    if (source && fields.some((other) => other !== field && mapping[other] === source)) return;
    onChange({ ...mapping, [field]: source || undefined });
  }
  return <Dialog open title="字段映射" onRequestClose={onCancel}>
    <div className="bom-mapping-form">
      {fields.map((field) => {
        const inputId = `bom-map-${field}`;
        return <FormField key={field} label={labels[field]} htmlFor={inputId}>
          <select id={inputId} className="pn-control" aria-label={labels[field]} value={mapping[field] ?? ""} onChange={(event) => setField(field, event.target.value)}>
            <option value="">不映射</option>
            {headers.map((header) => <option key={header} value={header} disabled={fields.some((other) => other !== field && mapping[other] === header)}>{header}</option>)}
          </select>
        </FormField>;
      })}
      <div className="bom-mapping-actions">
        <button className="pn-button pn-button--ghost" type="button" onClick={onCancel}>取消</button>
        <button className="pn-button pn-button--primary" type="button" onClick={onConfirm}>确认映射</button>
      </div>
    </div>
  </Dialog>;
}
