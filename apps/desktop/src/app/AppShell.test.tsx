import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Link, MemoryRouter, Route, Routes } from "react-router-dom";
import { StrictMode, useMemo, useState } from "react";
import { App } from "./App";
import { AppShell, usePageActions } from "./AppShell";
import "../styles/tokens.css";
import "../styles/primitives.css";
import "../styles/shell.css";

afterEach(cleanup);

describe("AppShell", () => {
  it("keeps an accessible PartNest mark visible when navigation is collapsed", () => {
    render(<App />);
    expect(screen.getByRole("img", { name: "PartNest" })).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "折叠导航" }));
    expect(screen.getByRole("img", { name: "PartNest" })).toBeVisible();
    expect(screen.queryByText("PartNest")).not.toBeInTheDocument();
  });

  it("collapses navigation without persistence and resets after remount", () => {
    const setItem = vi.spyOn(window.localStorage, "setItem");
    const { unmount } = render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "折叠导航" }));
    expect(screen.getByTestId("app-shell")).toHaveAttribute("data-nav", "collapsed");
    expect(setItem).not.toHaveBeenCalled();
    expect(screen.getByRole("link", { name: "库存" })).toHaveAttribute("aria-current", "page");

    unmount();
    render(<App />);
    expect(screen.getByTestId("app-shell")).toHaveAttribute("data-nav", "expanded");
    setItem.mockRestore();
  });

  it("shows one route title in the page toolbar", () => {
    render(<App />);
    fireEvent.click(screen.getByRole("link", { name: "项目" }));

    expect(screen.getByRole("toolbar", { name: "项目工具栏" })).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 1, name: "项目" })).toHaveLength(1);
  });

  it("cleans page actions when a route is replaced", () => {
    function ActionPage() {
      const action = useMemo(() => <button type="button">页面操作</button>, []);
      usePageActions(action);
      return <Link to="/plain">前往普通页</Link>;
    }

    render(
      <MemoryRouter initialEntries={["/actions"]}>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/actions" element={<ActionPage />} />
            <Route path="/plain" element={<p>普通页</p>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.getByRole("button", { name: "页面操作" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("link", { name: "前往普通页" }));
    expect(screen.queryByRole("button", { name: "页面操作" })).not.toBeInTheDocument();
  });

  it("renders the shell and fallback toolbar title for unknown paths", () => {
    render(
      <MemoryRouter initialEntries={["/unknown"]}>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="*" element={<p>未知页面</p>} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.getByRole("toolbar", { name: "PartNest工具栏" })).toBeInTheDocument();
    expect(screen.getByText("未知页面")).toBeInTheDocument();
  });

  it("updates memoized actions and uses the latest callback", async () => {
    function DynamicActionPage() {
      const [count, setCount] = useState(0);
      const action = useMemo(
        () => <button type="button" disabled={count > 0} onClick={() => setCount((value) => value + 1)}>计数 {count}</button>,
        [count],
      );
      usePageActions(action);
      return <p>动态 action</p>;
    }

    render(
      <MemoryRouter initialEntries={["/dynamic"]}>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/dynamic" element={<DynamicActionPage />} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );

    fireEvent.click(screen.getByRole("button", { name: "计数 0" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "计数 1" })).toBeDisabled());
  });

  it("aggregates multiple page actions and cleans up one registration independently", () => {
    function SecondaryAction() {
      const action = useMemo(() => <button type="button">次级操作</button>, []);
      usePageActions(action);
      return null;
    }

    function MultipleActionPage() {
      const [showSecondary, setShowSecondary] = useState(true);
      const primary = useMemo(() => <button type="button">主操作</button>, []);
      usePageActions(primary);
      return <>
        <button type="button" onClick={() => setShowSecondary(false)}>移除次级</button>
        {showSecondary && <SecondaryAction />}
      </>;
    }

    render(
      <MemoryRouter initialEntries={["/multiple"]}>
        <Routes>
          <Route element={<AppShell />}>
            <Route path="/multiple" element={<MultipleActionPage />} />
          </Route>
        </Routes>
      </MemoryRouter>,
    );

    expect(screen.getByRole("button", { name: "主操作" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "次级操作" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "移除次级" }));
    expect(screen.getByRole("button", { name: "主操作" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "次级操作" })).not.toBeInTheDocument();
  });

  it("keeps one action and cleans it under StrictMode", () => {
    function StrictActionPage() {
      const action = useMemo(() => <button type="button">严格模式操作</button>, []);
      usePageActions(action);
      return <p>严格模式</p>;
    }

    const view = render(
      <StrictMode>
        <MemoryRouter initialEntries={["/strict"]}>
          <Routes>
            <Route element={<AppShell />}>
              <Route path="/strict" element={<StrictActionPage />} />
            </Route>
          </Routes>
        </MemoryRouter>
      </StrictMode>,
    );

    expect(screen.getAllByRole("button", { name: "严格模式操作" })).toHaveLength(1);
    view.unmount();
    expect(screen.queryByRole("button", { name: "严格模式操作" })).not.toBeInTheDocument();
  });

  it("keeps the page toolbar at the 40px contract", () => {
    render(<App />);
    const toolbar = screen.getByRole("toolbar");
    expect(toolbar).toHaveClass("pn-toolbar");
    expect(getComputedStyle(toolbar).getPropertyValue("--toolbar-height").trim()).toBe("40px");
  });
});
