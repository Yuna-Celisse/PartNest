import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { App } from "../app/App";
import "../styles/tokens.css";
import "../styles/primitives.css";
import "../styles/shell.css";
import "../styles/inventory.css";
import "../styles/welding.css";
import "../styles/operations.css";

afterEach(() => {
  cleanup();
  document.documentElement.removeAttribute("data-theme");
  window.history.pushState({}, "", "/");
});

describe("compact desktop layout contract", () => {
  it("marks the app shell and dark theme at the target viewport", () => {
    Object.defineProperty(window, "innerWidth", { value: 1280, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
    render(<App />);

    expect(screen.getByTestId("app-shell")).toHaveClass("compact-desktop-shell");
    expect(document.documentElement).toHaveAttribute("data-theme", "dark");
    expect(document.querySelector(".pn-toolbar")).toHaveClass("pn-toolbar");
    expect(document.documentElement).not.toHaveAttribute("data-layout-measured");
  });

  it("keeps the movements ledger filling the page", () => {
    window.history.pushState({}, "", "/movements");
    render(<App />);

    const page = screen.getByRole("region", { name: "库存流水" });
    expect(page).toHaveClass("operations-page", "movements-page");
    // The fill comes from the panel being the page section's own child, which is
    // what the `.movements-page > .pn-table-wrap` flex rule stretches.
    const panel = page.querySelector(".pn-table-wrap");
    expect(panel).toBeTruthy();
    expect(panel?.parentElement).toBe(page);
    expect(panel?.querySelector("table")).toHaveClass("pn-data-table");
  });

  it("exposes stable CSS contracts without measuring jsdom layout", () => {
    render(<App />);
    const toolbar = document.querySelector<HTMLElement>(".pn-toolbar");
    expect(toolbar).toHaveStyle({ "--toolbar-height": "40px" });
    expect(document.querySelector(".pn-control") ?? document.querySelector(".pn-toolbar")).toBeTruthy();
    expect(document.querySelector(".pn-shell")).toHaveClass("compact-desktop-shell");
  });
});
