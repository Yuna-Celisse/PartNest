import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { open, confirm } from "@tauri-apps/plugin-dialog";
import { pickBackupFile, SettingsPage } from "./SettingsPage";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), confirm: vi.fn() }));

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("SettingsPage", () => {
  it("creates a backup and restores the explicitly selected file", async () => {
    const api = {
      createBackup: vi.fn().mockResolvedValue("C:/backups/partnest.db"),
      restoreBackup: vi.fn().mockResolvedValue(undefined),
      resetData: vi.fn(),
    };
    const pickFile = vi.fn().mockResolvedValue("C:/backups/selected.db");
    render(<SettingsPage api={api} pickFile={pickFile} />);

    fireEvent.click(screen.getByRole("button", { name: "立即备份" }));
    await waitFor(() => expect(api.createBackup).toHaveBeenCalledOnce());
    expect(screen.getByText(/备份已创建/)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "选择备份恢复" }));
    await waitFor(() => expect(pickFile).toHaveBeenCalledOnce());
    await waitFor(() => expect(api.restoreBackup).toHaveBeenCalledWith("C:/backups/selected.db"));
    expect(screen.getByText(/已恢复备份/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "数据与备份" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "界面" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "立即备份" })).toHaveClass("pn-button", "pn-button--primary");
    expect(screen.getByRole("button", { name: "选择备份恢复" })).toHaveClass("pn-button", "pn-button--secondary");
    expect(screen.getAllByRole("heading")).toHaveLength(3);
    expect(screen.queryByText("备份包含当前库存、流水和焊接进度。")) .not.toBeInTheDocument();
    expect(screen.getByText("深色主题")).toBeInTheDocument();
  });

  it("configures the file picker for database backups and handles cancellation", async () => {
    vi.mocked(open).mockResolvedValueOnce("C:/backups/selected.db");
    await expect(pickBackupFile()).resolves.toBe("C:/backups/selected.db");
    expect(open).toHaveBeenCalledWith({
      multiple: false,
      filters: [{ name: "PartNest 备份", extensions: ["db"] }],
    });

    vi.mocked(open).mockResolvedValueOnce(["C:/backups/ignored.db"]);
    await expect(pickBackupFile()).resolves.toBeNull();
  });

  it.each([false, true])("waits for reset confirmation and respects cancellation: %s", async (accepted) => {
    const api = { createBackup: vi.fn(), restoreBackup: vi.fn(), resetData: vi.fn().mockResolvedValue("before-reset.db") };
    let decide!: (value: boolean) => void;
    vi.mocked(confirm).mockReturnValue(new Promise<boolean>((resolve) => { decide = resolve; }));
    render(<SettingsPage api={api} />);
    fireEvent.click(screen.getByRole("button", { name: "重置数据" }));
    expect(api.resetData).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "立即备份" })).toBeDisabled();
    decide(accepted);
    await waitFor(() => expect(screen.getByRole("button", { name: "重置数据" })).toBeEnabled());
    expect(api.resetData).toHaveBeenCalledTimes(accepted ? 1 : 0);
    if (accepted) expect(screen.getByText(/数据已重置/)).toHaveTextContent("before-reset.db");
  });

  it("reports reset failure and allows retry", async () => {
    vi.mocked(confirm).mockResolvedValue(true);
    const api = { createBackup: vi.fn(), restoreBackup: vi.fn(), resetData: vi.fn().mockRejectedValue(new Error("备份失败")) };
    render(<SettingsPage api={api} />);
    fireEvent.click(screen.getByRole("button", { name: "重置数据" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("备份失败");
    expect(screen.queryByText(/数据已重置/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重置数据" })).toBeEnabled();
  });
});
