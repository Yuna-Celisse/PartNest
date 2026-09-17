import { BrowserRouter, Navigate, Route, Routes, useNavigate } from "react-router-dom";
import { primaryRoutes, RoutePlaceholder } from "./routes";
import { AppShell } from "./AppShell";
import { BoxesPage } from "../features/boxes/BoxesPage";
import { InventoryPage } from "../features/inventory/InventoryPage";
import { ProjectsPage } from "../features/projects/ProjectsPage";
import { WeldingPage } from "../features/welding/WeldingPage";
import { MovementsPage } from "../features/movements/MovementsPage";
import { SettingsPage } from "../features/settings/SettingsPage";

/** Pages that move the operator somewhere else in-app, not by reloading. */
function ProjectRoute() {
  const navigate = useNavigate();
  return <ProjectsPage navigate={navigate} />;
}

function WeldingRoute() {
  const navigate = useNavigate();
  return <WeldingPage navigate={navigate} />;
}

export function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<AppShell />}>
          <Route index element={<Navigate to="/inventory" replace />} />
          {primaryRoutes.map((route) => <Route key={route.id} path={route.path} element={route.id === "inventory" ? <InventoryPage /> : route.id === "boxes" ? <BoxesPage /> : route.id === "projects" ? <ProjectRoute /> : route.id === "welding" ? <WeldingRoute /> : route.id === "movements" ? <MovementsPage /> : route.id === "settings" ? <SettingsPage /> : <RoutePlaceholder />} />)}
          <Route path="*" element={<RoutePlaceholder />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}
