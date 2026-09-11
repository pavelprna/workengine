import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  Activity,
  CircleHelp,
  LayoutDashboard,
  TerminalSquare,
} from "lucide-react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { api } from "./api/client";
import { Button } from "./components/ui/button";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, staleTime: 5_000 } },
});

function Health() {
  const query = api.useHealth();
  if (query.isPending)
    return <p className="muted">Checking the local control plane…</p>;
  if (query.isError) {
    return (
      <section className="card error" aria-live="polite">
        <h2>Workengine is unavailable</h2>
        <p>{query.error.message}</p>
        <Button onClick={() => query.refetch()}>Try again</Button>
      </section>
    );
  }
  return (
    <section className="card" aria-live="polite">
      <div className="health-title">
        <Activity size={20} /> <span>Local observer is healthy</span>
      </div>
      <dl>
        <div>
          <dt>Status</dt>
          <dd>{query.data.status}</dd>
        </div>
        <div>
          <dt>Store</dt>
          <dd>{query.data.store}</dd>
        </div>
        <div>
          <dt>Version</dt>
          <dd>{query.data.version}</dd>
        </div>
      </dl>
    </section>
  );
}

function Placeholder({ title, text }: { title: string; text: string }) {
  return (
    <section className="placeholder">
      <h2>{title}</h2>
      <p>{text}</p>
    </section>
  );
}

function App() {
  const path = window.location.pathname;
  return (
    <main className="shell">
      <aside>
        <div className="brand">
          <TerminalSquare size={22} />
          <span>workengine</span>
        </div>
        <nav aria-label="Primary navigation">
          <a className={path === "/" ? "active" : ""} href="/">
            <Activity size={17} /> Health
          </a>
          <a href="/works">
            <LayoutDashboard size={17} /> Work{" "}
            <span className="soon">Soon</span>
          </a>
          <a href="/events">
            <CircleHelp size={17} /> Events <span className="soon">Soon</span>
          </a>
        </nav>
      </aside>
      <section className="content">
        <header>
          <p className="eyebrow">LOCAL OBSERVER</p>
          <h1>{path === "/" ? "Health & doctor" : "Workengine"}</h1>
        </header>
        {path === "/" ? (
          <Health />
        ) : (
          <Placeholder
            title="Observer foundation"
            text="This route is reserved for the next v0.1 observer screen."
          />
        )}
      </section>
    </main>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing Web UI root");

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <App />
    </QueryClientProvider>
  </StrictMode>,
);
