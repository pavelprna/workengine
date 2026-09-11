import { afterEach, beforeEach, expect, test } from "bun:test";
import { GlobalWindow } from "happy-dom";

const window = new GlobalWindow({ url: "http://localhost/" });
Object.assign(globalThis, {
  document: window.document,
  HTMLElement: window.HTMLElement,
  history: window.history,
  location: window.location,
  navigator: window.navigator,
  Node: window.Node,
  scrollTo: () => {},
  window,
});

const { cleanup, render, screen, waitFor } = await import(
  "@testing-library/react"
);
const userEvent = (await import("@testing-library/user-event")).default;
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const { RouterProvider } = await import("@tanstack/react-router");
const { LiveEventsProvider } = await import("./live");
const { createAppRouter } = await import("./router");

class FakeEventSource {
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;

  addEventListener() {}
  close() {}
}

Object.defineProperty(globalThis, "EventSource", {
  value: FakeEventSource,
  writable: true,
});

const requests: { method: string; url: URL; body?: string }[] = [];

function response(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function work(id: string, goal: string, status = "ready") {
  return {
    workId: id,
    status,
    goal,
    workerProfile: "stub",
    createdAtUnixMs: "1000",
    outcomeKind: null,
    workspaceBound: false,
  };
}

beforeEach(() => {
  requests.length = 0;
  globalThis.fetch = (async (input, init) => {
    const requestLike = input as {
      body?: BodyInit | null;
      method?: string;
      url?: string;
    };
    const rawUrl =
      typeof input === "string" || input instanceof URL
        ? input.toString()
        : requestLike.url;
    if (!rawUrl) throw new Error("The mocked request did not provide a URL");
    const url = new URL(rawUrl, "http://localhost/");
    const method = requestLike.method ?? init?.method ?? "GET";
    requests.push({
      method,
      url,
      body:
        method === "POST"
          ? typeof init?.body === "string"
            ? init.body
            : typeof requestLike.body === "string"
              ? requestLike.body
              : undefined
          : undefined,
    });
    if (url.pathname === "/api/v0/health") {
      return response({
        status: "ok",
        version: "workengine test",
        store: "readable",
      });
    }
    if (url.pathname === "/api/v0/overview") {
      return response({
        totalWorks: 2,
        statusCounts: {
          ready: 1,
          running: 0,
          succeeded: 1,
          failed: 0,
          parked: 0,
        },
      });
    }
    if (url.pathname === "/api/v0/works" && method === "POST") {
      return response(work("work-new", "created from browser"), 201);
    }
    if (url.pathname === "/api/v0/works/work-new/start" && method === "POST") {
      return response({
        ...work("work-new", "created from browser", "succeeded"),
        outcomeKind: "succeeded",
        workspaceBound: true,
      });
    }
    if (url.pathname === "/api/v0/works/work-new") {
      return response(work("work-new", "created from browser"));
    }
    if (
      url.pathname === "/api/v0/works/work-running/park" &&
      method === "POST"
    ) {
      return response(work("work-running", "interrupt safely", "parked"));
    }
    if (url.pathname === "/api/v0/works/work-running") {
      return response(work("work-running", "interrupt safely", "running"));
    }
    if (url.pathname === "/api/v0/works/work-detail") {
      return response({
        ...work("work-detail", "inspect history", "succeeded"),
        outcomeKind: "succeeded",
      });
    }
    if (url.pathname === "/api/v0/works/work-detail/observation") {
      return response({
        executions: [
          {
            executionId: "execution-detail",
            createdAtUnixMs: "1100",
            spec: {
              schemaVersion: 1,
              workerProfile: "stub",
              workerConfigDigest: `sha256:${"1".repeat(64)}`,
              runtimeKind: "bubblewrap",
              runtimeDigest: `sha256:${"2".repeat(64)}`,
              wallClockBudgetMs: "5000",
              retryLimit: 1,
              channelPolicy: "retry_then_fail",
              secretRefs: [],
            },
            attempts: [
              {
                attemptId: "attempt-detail",
                state: "confirmed",
                retryOrdinal: 0,
                startedAtUnixMs: "1100",
                lastHeartbeatAtUnixMs: "1900",
                finishedAtUnixMs: "2000",
                terminalReason: "succeeded",
                checkpoint: "not_recorded",
                processRecords: [
                  {
                    event: "spawned",
                    occurrences: "1",
                    firstObservedAtUnixMs: "1100",
                    lastObservedAtUnixMs: "1100",
                    payloadRedacted: false,
                  },
                  {
                    event: "child_stdout",
                    occurrences: "4",
                    firstObservedAtUnixMs: "1200",
                    lastObservedAtUnixMs: "1800",
                    payloadRedacted: true,
                  },
                ],
                confirmedOutcome: {
                  schemaVersion: 1,
                  kind: "succeeded",
                  workerProfile: "stub",
                  confirmedAtUnixMs: "2000",
                },
              },
            ],
          },
        ],
        artifacts: [
          {
            kind: "confirmed_outcome",
            label: "Confirmed succeeded outcome",
            href: "/api/v0/proof",
            executionId: "execution-detail",
            attemptId: "attempt-detail",
            authoredBy: "stub",
            createdAtUnixMs: "2000",
          },
        ],
        diagnostics: [
          {
            severity: "info",
            code: "confirmed_success",
            message:
              "The matching attempt produced a confirmed successful outcome.",
            executionId: "execution-detail",
            attemptId: "attempt-detail",
          },
        ],
      });
    }
    if (url.pathname === "/api/v0/works") {
      return response({
        items: [work("work-a", "alpha"), work("work-b", "bravo", "succeeded")],
        nextCursor: null,
      });
    }
    if (url.pathname === "/api/v0/events") {
      return response({
        items: [
          {
            cursor: "1",
            workId: "work-detail",
            kind: "created",
            from: null,
            to: "ready",
            outcomeKind: null,
            createdAtUnixMs: "1000",
          },
          {
            cursor: "2",
            workId: "work-detail",
            kind: "completed",
            from: "running",
            to: "succeeded",
            outcomeKind: "succeeded",
            createdAtUnixMs: "2000",
          },
        ],
        nextCursor: null,
      });
    }
    return response({ code: "not_found", message: "not found" }, 404);
  }) as typeof fetch;
  window.fetch = globalThis.fetch as unknown as typeof window.fetch;
});

afterEach(() => cleanup());

function renderApp(router: ReturnType<typeof createAppRouter>) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <LiveEventsProvider>
        <RouterProvider router={router} />
      </LiveEventsProvider>
    </QueryClientProvider>,
  );
}

test.serial(
  "queue filters are URL-backed and visible in the request",
  async () => {
    const router = createAppRouter();
    await router.navigate({
      to: "/works",
      search: { profile: undefined, status: undefined },
    });
    renderApp(router);
    await screen.findByText("alpha");

    await userEvent.click(screen.getByRole("button", { name: "succeeded" }));
    await waitFor(() => {
      expect(
        requests.some(
          ({ url }) => url.searchParams.get("status") === "succeeded",
        ),
      ).toBe(true);
    });
  },
);

test.serial("creating Work opens its durable detail record", async () => {
  const router = createAppRouter();
  await router.navigate({
    to: "/works",
    search: { profile: undefined, status: undefined },
  });
  renderApp(router);
  await screen.findByText("Put a task in the queue");

  await userEvent.type(screen.getByLabelText("Goal"), "created from browser");
  await userEvent.click(screen.getByRole("button", { name: "Add to queue" }));

  await screen.findByRole("heading", { name: "created from browser" });
  const request = requests.find(({ method }) => method === "POST");
  expect(request?.url.pathname).toBe("/api/v0/works");
});

test.serial("detail renders the chronological event timeline", async () => {
  const router = createAppRouter();
  await router.navigate({
    to: "/works/$workId",
    params: { workId: "work-detail" },
  });
  renderApp(router);
  await screen.findByRole("heading", { name: "inspect history" });
  expect(screen.getByRole("heading", { name: "Event timeline" })).toBeTruthy();
  expect(screen.getByText(/event #1/)).toBeTruthy();
  expect(screen.getByText(/event #2/)).toBeTruthy();
});

test.serial("detail explains execution and links proof artifacts", async () => {
  const router = createAppRouter();
  await router.navigate({
    to: "/works/$workId",
    params: { workId: "work-detail" },
  });
  renderApp(router);
  await screen.findByRole("heading", { name: "Execution ledger" });
  expect(screen.getByText("bubblewrap")).toBeTruthy();
  expect(screen.getByText("child_stdout")).toBeTruthy();
  expect(screen.getByText("payload redacted")).toBeTruthy();
  expect(screen.getByText("confirmed success")).toBeTruthy();
  expect(
    screen.getByRole("link", { name: /Confirmed succeeded outcome/ }),
  ).toBeTruthy();
});

test.serial(
  "ready Work can be explicitly started from its record",
  async () => {
    const router = createAppRouter();
    await router.navigate({
      to: "/works/$workId",
      params: { workId: "work-new" },
    });
    renderApp(router);
    await screen.findByRole("heading", { name: "created from browser" });

    await userEvent.click(screen.getByRole("button", { name: "Start Work" }));

    await screen.findByText("Execution is confirmed");
    expect(
      requests.some(
        ({ method, url }) =>
          method === "POST" && url.pathname === "/api/v0/works/work-new/start",
      ),
    ).toBe(true);
  },
);

test.serial("running Work can be checkpointed from its record", async () => {
  const router = createAppRouter();
  await router.navigate({
    to: "/works/$workId",
    params: { workId: "work-running" },
  });
  renderApp(router);
  await screen.findByRole("heading", { name: "interrupt safely" });

  await userEvent.click(screen.getByRole("button", { name: "Park" }));

  await screen.findByRole("button", { name: "Resume" });
  expect(
    requests.some(
      ({ method, url }) =>
        method === "POST" && url.pathname === "/api/v0/works/work-running/park",
    ),
  ).toBe(true);
});
