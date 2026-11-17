import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ProjectPackagePanel } from "./ProjectPackagePanel";

afterEach(cleanup);

it("keeps project metadata and directs PCB operations to the welding workspace", async () => {
  const load = vi.fn().mockResolvedValue({ files: [{ path: "board-F_Cu.gbr", kind: "Gerber", size: 1024 }], bom_source: "board.kicad_sch（KiCad 导出）", warnings: [], front_svg: "<svg>front</svg>", back_svg: "<svg>back</svg>", positions: [
    { reference: "R1", value: "10k", package: "0603", x_mm: 10.2, y_mm: -30, rotation: 90, side: "top" },
    { reference: "U1", value: "MCU", package: "QFN", x_mm: 40, y_mm: -20, rotation: 180, side: "bottom" },
  ] });
  render(<ProjectPackagePanel projectId="p-1" load={load} />);
  expect(await screen.findByText("PCB 3D 预览与器件位置请在焊接工作台中查看。")).toBeVisible();
  expect(screen.getByText(/board-F_Cu.gbr/)).toBeVisible();
  expect(screen.queryByLabelText("搜索器件位置")).not.toBeInTheDocument();
  expect(load).toHaveBeenCalledWith("p-1");
});
