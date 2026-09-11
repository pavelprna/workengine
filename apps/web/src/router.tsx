import {
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { type WorkStatus, workStatuses } from "./api/client";
import { AppShell } from "./app";
import {
  DoctorPage,
  NotFoundPage,
  OverviewPage,
  WorkDetailPage,
  WorksPage,
} from "./pages";

const rootRoute = createRootRoute({
  component: AppShell,
  notFoundComponent: NotFoundPage,
});

const overviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: OverviewPage,
});

export const worksRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "works",
  validateSearch: (search: Record<string, unknown>) => ({
    status: workStatuses.includes(search.status as WorkStatus)
      ? (search.status as WorkStatus)
      : undefined,
    profile:
      typeof search.profile === "string" && search.profile.length > 0
        ? search.profile
        : undefined,
  }),
  component: WorksPage,
});

export const workRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "works/$workId",
  component: WorkDetailPage,
});

const doctorRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "doctor",
  component: DoctorPage,
});

const routeTree = rootRoute.addChildren([
  overviewRoute,
  worksRoute,
  workRoute,
  doctorRoute,
]);

export function createAppRouter() {
  return createRouter({ routeTree, defaultPreload: "intent" });
}

export const router = createAppRouter();

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
