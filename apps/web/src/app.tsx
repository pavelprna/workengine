import { Link, Outlet } from "@tanstack/react-router";
import {
  Activity,
  HeartPulse,
  LayoutDashboard,
  ListTodo,
  TerminalSquare,
} from "lucide-react";
import { api } from "./api/client";
import { useLiveConnection } from "./live";

function RuntimeState() {
  const health = api.useHealth();
  const connection = useLiveConnection();
  const online = health.data?.status === "ok";

  return (
    <div className="runtime-state" aria-live="polite">
      <span
        className={`signal ${online ? "healthy" : "offline"}`}
        aria-hidden="true"
      />
      <div>
        <strong>
          {online ? "control plane online" : "control plane unavailable"}
        </strong>
        <span>events: {connection}</span>
      </div>
      {health.data && <code>{health.data.version}</code>}
    </div>
  );
}

export function AppShell() {
  return (
    <main className="shell">
      <aside className="sidebar">
        <Link className="brand" to="/" aria-label="Workengine overview">
          <TerminalSquare size={22} />
          <span>workengine</span>
        </Link>
        <nav aria-label="Primary navigation">
          <Link
            activeProps={{ className: "active" }}
            activeOptions={{ exact: true }}
            to="/"
          >
            <LayoutDashboard size={17} /> Overview
          </Link>
          <Link
            activeProps={{ className: "active" }}
            search={{ profile: undefined, status: undefined }}
            to="/works"
          >
            <ListTodo size={17} /> Work queue
          </Link>
          <Link activeProps={{ className: "active" }} to="/doctor">
            <HeartPulse size={17} /> Doctor
          </Link>
        </nav>
        <div className="sidebar-note">
          <Activity size={15} />
          <p>
            Local-first observer. Execution controls are intentionally offline.
          </p>
        </div>
        <RuntimeState />
      </aside>
      <section className="content">
        <Outlet />
      </section>
    </main>
  );
}
