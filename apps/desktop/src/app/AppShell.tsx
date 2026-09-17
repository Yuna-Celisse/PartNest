import { createContext, Fragment, useCallback, useContext, useEffect, useId, useLayoutEffect, useMemo, useState, type ReactNode } from "react";
import { matchPath, NavLink, Outlet, useLocation } from "react-router-dom";
import { BrandMark, Icon } from "../components/ui/Icon";
import { PageToolbar } from "../components/ui/PageToolbar";
import { primaryRoutes } from "./routes";

interface PageActionEntry {
  id: string;
  routeKey: string;
  actions: ReactNode;
}

interface PageActionsContextValue {
  entries: PageActionEntry[];
  register: (id: string, routeKey: string, actions: ReactNode) => void;
  unregister: (id: string, routeKey: string) => void;
}

const PageActionsContext = createContext<PageActionsContextValue | null>(null);

function PageActionsProvider({ routeKey, children }: { routeKey: string; children: ReactNode }): JSX.Element {
  const [entries, setEntries] = useState<PageActionEntry[]>([]);
  useEffect(() => {
    setEntries((current) => {
      const next = current.filter((entry) => entry.routeKey === routeKey);
      return next.length === current.length ? current : next;
    });
  }, [routeKey]);
  const register = useCallback((id: string, nextRouteKey: string, actions: ReactNode) => {
    setEntries((current) => {
      const index = current.findIndex((entry) => entry.id === id);
      if (index >= 0) {
        const previous = current[index];
        if (previous.routeKey === nextRouteKey && Object.is(previous.actions, actions)) return current;
        const next = current.slice();
        next[index] = { id, routeKey: nextRouteKey, actions };
        return next;
      }
      return [...current, { id, routeKey: nextRouteKey, actions }];
    });
  }, []);
  const unregister = useCallback((id: string, nextRouteKey: string) => {
    setEntries((current) => {
      const next = current.filter((entry) => entry.id !== id || entry.routeKey !== nextRouteKey);
      return next.length === current.length ? current : next;
    });
  }, []);
  const value = useMemo(() => ({ entries, register, unregister }), [entries, register, unregister]);

  return <PageActionsContext.Provider value={value}>{children}</PageActionsContext.Provider>;
}

/**
 * Register actions in the current page toolbar for this route instance.
 * Callers must pass a useMemo-stable ReactNode; include every value used by
 * labels, disabled states, and callbacks in that memo's dependency list.
 */
export function usePageActions(actions: ReactNode): boolean {
  const context = useContext(PageActionsContext);
  const location = useLocation();
  const id = useId();
  const routeKey = `${location.pathname}${location.search}`;
  const register = context?.register;
  const unregister = context?.unregister;

  useEffect(() => {
    if (!register || !unregister) return undefined;
    register(id, routeKey, actions);
    return () => unregister(id, routeKey);
  }, [actions, register, unregister, id, routeKey]);
  return Boolean(context);
}

function AppShellContent(): JSX.Element {
  const location = useLocation();
  const routeKey = `${location.pathname}${location.search}`;
  const context = useContext(PageActionsContext);
  const [navigationExpanded, setNavigationExpanded] = useState(true);
  useLayoutEffect(() => {
    document.documentElement.setAttribute("data-theme", "dark");
  }, []);
  const route = primaryRoutes.find((candidate) => matchPath({ path: candidate.path, end: true }, location.pathname));
  const actions = context
    ? context.entries.filter((entry) => entry.routeKey === routeKey).map((entry) => <Fragment key={entry.id}>{entry.actions}</Fragment>)
    : undefined;

  return (
    <div
      className="pn-shell compact-desktop-shell"
      data-testid="app-shell"
      data-nav={navigationExpanded ? "expanded" : "collapsed"}
    >
      <aside className="pn-shell__sidebar">
        <div className="pn-shell__brand">
          {navigationExpanded ? <>
            <BrandMark />
            <span className="pn-shell__brand-name">PartNest</span>
            <button className="pn-button pn-button--icon pn-button--ghost pn-shell__toggle" type="button" aria-label="折叠导航" title="折叠导航" onClick={() => setNavigationExpanded(false)}><Icon name="menu" size={16} /></button>
          </> : <button className="pn-shell__brand-mark-button" type="button" aria-label="展开导航" title="展开导航" onClick={() => setNavigationExpanded(true)}><BrandMark /></button>}
        </div>
        <nav aria-label="主导航" className="pn-shell__nav">
          {primaryRoutes.map((candidate) => (
            <NavLink
              key={candidate.id}
              to={candidate.path}
              end
              aria-label={candidate.label}
              title={candidate.label}
              className="pn-shell__nav-link"
            >
              <Icon name={candidate.icon} size={16} />
              <span className="pn-shell__nav-label">{candidate.label}</span>
            </NavLink>
          ))}
        </nav>
      </aside>
      <main className="pn-shell__main">
        <PageToolbar title={route?.title ?? "PartNest"} actions={actions} />
        <div className="pn-shell__content"><Outlet /></div>
      </main>
    </div>
  );
}

export function AppShell(): JSX.Element {
  const location = useLocation();
  const routeKey = `${location.pathname}${location.search}`;
  return <PageActionsProvider routeKey={routeKey}><AppShellContent /></PageActionsProvider>;
}
