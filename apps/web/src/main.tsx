import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { LiveEventsProvider } from "./live";
import { router } from "./router";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, staleTime: 5_000 } },
});

const root = document.getElementById("root");
if (!root) throw new Error("Missing Web UI root");

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <LiveEventsProvider>
        <RouterProvider router={router} />
      </LiveEventsProvider>
    </QueryClientProvider>
  </StrictMode>,
);
