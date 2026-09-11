import {
  Link,
  useNavigate,
  useParams,
  useSearch,
} from "@tanstack/react-router";
import {
  Activity,
  ArrowRight,
  ArrowUpRight,
  CheckCircle2,
  CircleAlert,
  CircleDot,
  Database,
  FileClock,
  FileJson,
  Filter,
  Fingerprint,
  Gauge,
  HeartPulse,
  ListFilter,
  PackageOpen,
  Play,
  Plus,
  Radio,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  Terminal,
} from "lucide-react";
import { type FormEvent, useState } from "react";
import {
  type Artifact,
  type Attempt,
  api,
  type Diagnostic,
  type Execution,
  type WorkEvent,
  type WorkFilters,
  type WorkSnapshot,
  type WorkStatus,
  workStatuses,
} from "./api/client";
import { useLiveConnection } from "./live";
import { workRoute, worksRoute } from "./router";

function timeParts(unixMs: string) {
  const date = new Date(Number(unixMs));
  return {
    dateTime: date.toISOString(),
    label: new Intl.DateTimeFormat(undefined, {
      dateStyle: "medium",
      timeStyle: "short",
    }).format(date),
  };
}

function durationLabel(milliseconds: number) {
  if (milliseconds < 1_000) return `${Math.max(0, milliseconds)} ms`;
  const seconds = milliseconds / 1_000;
  if (seconds < 60) return `${seconds.toFixed(seconds < 10 ? 1 : 0)} s`;
  return `${Math.floor(seconds / 60)}m ${Math.round(seconds % 60)}s`;
}

function StatusBadge({ status }: { status: WorkStatus }) {
  return <span className={`status ${status}`}>{status}</span>;
}

function QueryError({
  message,
  retry,
}: {
  message: string;
  retry?: () => void;
}) {
  return (
    <div className="query-error" role="alert">
      <CircleAlert size={19} />
      <p>{message}</p>
      {retry && (
        <button className="text-button" onClick={retry} type="button">
          Try again
        </button>
      )}
    </div>
  );
}

function WorkRows({ works }: { works: WorkSnapshot[] }) {
  if (works.length === 0) {
    return (
      <div className="empty-state">
        <CircleDot size={24} />
        <p>No Work matches this view.</p>
      </div>
    );
  }

  return (
    <ol className="work-list">
      {works.map((work) => {
        const created = timeParts(work.createdAtUnixMs);
        return (
          <li key={work.workId}>
            <Link
              className="work-row"
              params={{ workId: work.workId }}
              to="/works/$workId"
            >
              <span
                className={`status-dot ${work.status}`}
                aria-hidden="true"
              />
              <span className="work-copy">
                <strong>{work.goal}</strong>
                <small>
                  {work.workerProfile} ·{" "}
                  <time dateTime={created.dateTime}>{created.label}</time>
                </small>
              </span>
              <StatusBadge status={work.status} />
              <ArrowRight className="row-arrow" size={17} aria-hidden="true" />
            </Link>
          </li>
        );
      })}
    </ol>
  );
}

function LoadMore({
  hasMore,
  loading,
  onLoad,
}: {
  hasMore: boolean;
  loading: boolean;
  onLoad: () => void;
}) {
  if (!hasMore) return null;
  return (
    <button
      className="load-more"
      disabled={loading}
      onClick={onLoad}
      type="button"
    >
      <RefreshCw size={15} className={loading ? "spinning" : undefined} />
      {loading ? "Loading…" : "Load more Work"}
    </button>
  );
}

function NewWorkForm() {
  const createWork = api.useCreateWork();
  const navigate = useNavigate();
  const [goal, setGoal] = useState("");
  const [profile, setProfile] = useState("stub");

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    createWork.mutate(
      { goal, workerProfile: profile },
      {
        onSuccess: (work) => {
          void navigate({
            to: "/works/$workId",
            params: { workId: work.workId },
          });
        },
      },
    );
  }

  return (
    <section className="intake-panel" aria-labelledby="new-work-title">
      <div className="panel-kicker">
        <Plus size={15} /> OPERATOR INTAKE
      </div>
      <h2 id="new-work-title">Put a task in the queue</h2>
      <p className="quiet">
        A goal and profile become durable Work immediately. Launch it explicitly
        from its record when you are ready.
      </p>
      <form onSubmit={submit}>
        <label htmlFor="goal">Goal</label>
        <textarea
          id="goal"
          value={goal}
          onChange={(event) => setGoal(event.target.value)}
          placeholder="Describe the outcome you want…"
          required
          rows={5}
        />
        <label htmlFor="profile">Worker profile</label>
        <input
          id="profile"
          value={profile}
          onChange={(event) => setProfile(event.target.value)}
          required
        />
        {createWork.isError && (
          <p className="form-error" role="alert">
            {createWork.error.message}
          </p>
        )}
        <button
          className="primary-action"
          disabled={createWork.isPending}
          type="submit"
        >
          <ArrowUpRight size={16} />
          {createWork.isPending ? "Adding…" : "Add to queue"}
        </button>
      </form>
    </section>
  );
}

export function OverviewPage() {
  const overview = api.useOverview();
  const recent = api.useWorks({}, 5);
  const connection = useLiveConnection();
  const recentWorks = recent.data?.pages.flatMap((page) => page.items) ?? [];

  return (
    <>
      <header className="page-header overview-header">
        <p className="eyebrow">LOCAL CONTROL PLANE · v0.4</p>
        <h1>Every outcome has a trail.</h1>
        <p className="lede">
          Durable Work is the record. Follow every execution from immutable
          configuration to the attempt that proved its outcome.
        </p>
      </header>
      <section className="overview-grid" aria-label="System overview">
        <div className="overview-card signal-card">
          <div className="panel-kicker">
            <Radio size={15} /> LIVE CONNECTION
          </div>
          <strong className={`connection-readout ${connection}`}>
            {connection}
          </strong>
          <p>
            {connection === "live"
              ? "Events are refreshing this view."
              : "The observer is reconnecting to its event stream."}
          </p>
        </div>
        <div className="overview-card queue-total">
          <div className="panel-kicker">
            <ListFilter size={15} /> DURABLE QUEUE
          </div>
          {overview.isPending && (
            <strong className="metric-loading">Reading…</strong>
          )}
          {overview.isError && (
            <QueryError
              message={overview.error.message}
              retry={() => void overview.refetch()}
            />
          )}
          {overview.data && (
            <>
              <strong>{overview.data.totalWorks}</strong>
              <span>
                Work item{overview.data.totalWorks === 1 ? "" : "s"} recorded
              </span>
            </>
          )}
        </div>
        <div className="overview-card boundary-card">
          <div className="panel-kicker">
            <ShieldCheck size={15} /> BOUNDARY
          </div>
          <strong>localhost only</strong>
          <p>
            Observe, create, and explicit start are available. Execution
            settings stay on the local host.
          </p>
        </div>
      </section>
      {overview.data && (
        <section className="status-grid" aria-label="Work status counts">
          {workStatuses.map((status) => (
            <Link
              key={status}
              className={`status-count ${status}`}
              search={{ profile: undefined, status }}
              to="/works"
            >
              <span>{status}</span>
              <strong>{overview.data.statusCounts[status]}</strong>
            </Link>
          ))}
        </section>
      )}
      <section
        className="content-panel recent-panel"
        aria-labelledby="recent-title"
      >
        <div className="section-heading">
          <div>
            <p className="panel-kicker">LATEST RECORDS</p>
            <h2 id="recent-title">Recent Work</h2>
          </div>
          <Link
            className="text-link"
            search={{ profile: undefined, status: undefined }}
            to="/works"
          >
            View queue <ArrowRight size={15} />
          </Link>
        </div>
        {recent.isPending && <p className="quiet">Reading recent Work…</p>}
        {recent.isError && (
          <QueryError
            message={recent.error.message}
            retry={() => void recent.refetch()}
          />
        )}
        {recent.data && <WorkRows works={recentWorks} />}
      </section>
    </>
  );
}

export function WorksPage() {
  const search = useSearch({ from: worksRoute.id });
  const navigate = useNavigate();
  const [profileInput, setProfileInput] = useState(search.profile ?? "");
  const filters: WorkFilters = {
    status: search.status,
    profile: search.profile,
  };
  const works = api.useWorks(filters);
  const items = works.data?.pages.flatMap((page) => page.items) ?? [];

  function setSearch(next: WorkFilters) {
    void navigate({
      to: "/works",
      search: { profile: next.profile, status: next.status },
    });
  }

  function applyProfile(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSearch({ status: search.status, profile: profileInput || undefined });
  }

  return (
    <>
      <header className="page-header compact-header">
        <p className="eyebrow">WORK REGISTRY</p>
        <h1>Queue, not guesswork.</h1>
        <p className="lede">
          Filter the durable record. Selecting an item opens its append-only
          history.
        </p>
      </header>
      <div className="works-layout">
        <section
          className="content-panel queue-panel"
          aria-labelledby="queue-title"
        >
          <div className="section-heading">
            <div>
              <p className="panel-kicker">FILTERABLE SNAPSHOT</p>
              <h2 id="queue-title">Work queue</h2>
            </div>
            <span className="readout">NEWEST FIRST</span>
          </div>
          <div className="filters">
            <fieldset className="status-filter">
              <legend>Status</legend>
              <button
                className={!search.status ? "selected" : ""}
                onClick={() => setSearch({ profile: search.profile })}
                type="button"
              >
                all
              </button>
              {workStatuses.map((status) => (
                <button
                  className={search.status === status ? "selected" : ""}
                  key={status}
                  onClick={() => setSearch({ status, profile: search.profile })}
                  type="button"
                >
                  {status}
                </button>
              ))}
            </fieldset>
            <form className="profile-filter" onSubmit={applyProfile}>
              <label htmlFor="profile-filter">
                <Filter size={14} /> Exact profile
              </label>
              <input
                id="profile-filter"
                onChange={(event) => setProfileInput(event.target.value)}
                placeholder="all profiles"
                value={profileInput}
              />
              <button className="text-button" type="submit">
                Apply
              </button>
            </form>
          </div>
          {works.isPending && <p className="quiet">Reading the queue…</p>}
          {works.isError && (
            <QueryError
              message={works.error.message}
              retry={() => void works.refetch()}
            />
          )}
          {works.data && (
            <>
              <WorkRows works={items} />
              <LoadMore
                hasMore={works.hasNextPage}
                loading={works.isFetchingNextPage}
                onLoad={() => void works.fetchNextPage()}
              />
            </>
          )}
        </section>
        <NewWorkForm />
      </div>
    </>
  );
}

function Timeline({ events }: { events: WorkEvent[] }) {
  if (events.length === 0)
    return (
      <div className="empty-state">
        <FileClock size={24} />
        <p>No events were recorded for this Work.</p>
      </div>
    );
  return (
    <ol className="timeline">
      {events.map((event) => {
        const time = timeParts(event.createdAtUnixMs);
        return (
          <li key={event.cursor}>
            <span
              className={`timeline-marker ${event.to}`}
              aria-hidden="true"
            />
            <div>
              <div className="timeline-title">
                <strong>{event.kind}</strong>
                <StatusBadge status={event.to} />
              </div>
              <p>
                {event.from
                  ? `${event.from} → ${event.to}`
                  : `entered ${event.to}`}
                {event.outcomeKind ? ` · ${event.outcomeKind}` : ""}
              </p>
              <time dateTime={time.dateTime}>
                {time.label} · event #{event.cursor}
              </time>
            </div>
          </li>
        );
      })}
    </ol>
  );
}

function DiagnosticList({ diagnostics }: { diagnostics: Diagnostic[] }) {
  return (
    <section className="diagnostic-ribbon" aria-label="Execution diagnostics">
      {diagnostics.map((diagnostic, index) => (
        <div
          className={`diagnostic ${diagnostic.severity}`}
          key={`${diagnostic.code}-${diagnostic.attemptId ?? index}`}
        >
          <CircleAlert size={16} />
          <div>
            <strong>{diagnostic.code.replaceAll("_", " ")}</strong>
            <p>{diagnostic.message}</p>
          </div>
        </div>
      ))}
    </section>
  );
}

function ProcessRecords({ attempt }: { attempt: Attempt }) {
  if (attempt.processRecords.length === 0) {
    return <p className="record-empty">No process metadata recorded.</p>;
  }
  return (
    <section className="process-records" aria-label="Redacted process records">
      {attempt.processRecords.map((record) => (
        <div className="process-record" key={record.event}>
          <Terminal size={14} />
          <code>{record.event}</code>
          <strong>×{record.occurrences}</strong>
          {record.payloadRedacted && <span>payload redacted</span>}
        </div>
      ))}
    </section>
  );
}

function AttemptCard({
  attempt,
  budgetMs,
}: {
  attempt: Attempt;
  budgetMs: number;
}) {
  const started = timeParts(attempt.startedAtUnixMs);
  const heartbeat = timeParts(attempt.lastHeartbeatAtUnixMs);
  const observedUntil = Number(
    attempt.finishedAtUnixMs ?? attempt.lastHeartbeatAtUnixMs,
  );
  const used = Math.max(0, observedUntil - Number(attempt.startedAtUnixMs));
  const budgetPercent = Math.min(100, Math.round((used / budgetMs) * 100));
  return (
    <article className={`attempt-card ${attempt.state}`}>
      <div className="attempt-heading">
        <div>
          <p className="attempt-index">
            ATTEMPT {attempt.retryOrdinal + 1}
            {attempt.retryOrdinal > 0 && (
              <span>
                <RotateCcw size={11} /> retry
              </span>
            )}
          </p>
          <code>{attempt.attemptId}</code>
        </div>
        <span className={`attempt-state ${attempt.state}`}>
          {attempt.state}
        </span>
      </div>
      <div className="attempt-vitals">
        <div>
          <HeartPulse size={14} />
          <span>last heartbeat</span>
          <time dateTime={heartbeat.dateTime}>{heartbeat.label}</time>
        </div>
        <div>
          <Gauge size={14} />
          <span>budget observed</span>
          <strong>
            {durationLabel(used)} / {durationLabel(budgetMs)}
          </strong>
        </div>
        <div>
          <PackageOpen size={14} />
          <span>checkpoint</span>
          <strong>{attempt.checkpoint.replaceAll("_", " ")}</strong>
        </div>
      </div>
      <div
        aria-label="Observed wall-clock budget"
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={budgetPercent}
        className="budget-track"
        role="progressbar"
      >
        <span style={{ width: `${budgetPercent}%` }} />
      </div>
      <p className="attempt-started">
        Started <time dateTime={started.dateTime}>{started.label}</time>
        {attempt.terminalReason && (
          <>
            {" "}
            · terminal reason <strong>{attempt.terminalReason}</strong>
          </>
        )}
      </p>
      <ProcessRecords attempt={attempt} />
    </article>
  );
}

function ExecutionCard({
  execution,
  number,
}: {
  execution: Execution;
  number: number;
}) {
  const created = timeParts(execution.createdAtUnixMs);
  const budgetMs = Number(execution.spec.wallClockBudgetMs);
  return (
    <article className="execution-card">
      <header className="execution-heading">
        <div>
          <p className="panel-kicker">EXECUTION {number}</p>
          <h3>{execution.spec.workerProfile}</h3>
          <code>{execution.executionId}</code>
        </div>
        <time dateTime={created.dateTime}>{created.label}</time>
      </header>
      <div className="spec-grid">
        <div>
          <Fingerprint size={15} />
          <span>CONFIG DIGEST</span>
          <code title={execution.spec.workerConfigDigest}>
            {execution.spec.workerConfigDigest}
          </code>
        </div>
        <div>
          <ShieldCheck size={15} />
          <span>SANDBOX</span>
          <strong>{execution.spec.runtimeKind}</strong>
          <code title={execution.spec.runtimeDigest}>
            {execution.spec.runtimeDigest}
          </code>
        </div>
        <div>
          <Gauge size={15} />
          <span>BUDGET</span>
          <strong>{durationLabel(budgetMs)}</strong>
          <small>{execution.spec.channelPolicy.replaceAll("_", " ")}</small>
        </div>
        <div>
          <RotateCcw size={15} />
          <span>RETRIES</span>
          <strong>
            {Math.max(0, execution.attempts.length - 1)} /{" "}
            {execution.spec.retryLimit}
          </strong>
          <small>{execution.attempts.length} process attempt(s)</small>
        </div>
      </div>
      <div className="attempt-stack">
        {execution.attempts.map((attempt) => (
          <AttemptCard
            attempt={attempt}
            budgetMs={budgetMs}
            key={attempt.attemptId}
          />
        ))}
      </div>
    </article>
  );
}

function ArtifactCatalogue({ artifacts }: { artifacts: Artifact[] }) {
  return (
    <section
      className="content-panel artifact-panel"
      aria-labelledby="artifact-title"
    >
      <div className="section-heading">
        <div>
          <p className="panel-kicker">FINITE PROOF PATH</p>
          <h2 id="artifact-title">Artifact catalogue</h2>
        </div>
        <span className="readout">CONTROL PLANE ONLY</span>
      </div>
      <div className="artifact-list">
        {artifacts.map((artifact) => (
          <a
            href={artifact.href}
            key={artifact.href}
            rel="noreferrer"
            target="_blank"
          >
            <FileJson size={17} />
            <span>
              <strong>{artifact.label}</strong>
              <small>
                {artifact.kind.replaceAll("_", " ")} · authored by{" "}
                {artifact.authoredBy}
              </small>
            </span>
            <ArrowUpRight size={15} />
          </a>
        ))}
      </div>
    </section>
  );
}

export function WorkDetailPage() {
  const { workId } = useParams({ from: workRoute.id });
  const work = api.useWork(workId);
  const events = api.useEvents(workId);
  const observation = api.useObservation(workId);
  const startWork = api.useStartWork();
  const allEvents = events.data?.pages.flatMap((page) => page.items) ?? [];

  if (work.isPending)
    return <p className="quiet page-loading">Opening Work record…</p>;
  if (work.isError)
    return (
      <QueryError
        message={work.error.message}
        retry={() => void work.refetch()}
      />
    );
  const created = timeParts(work.data.createdAtUnixMs);

  return (
    <>
      <Link
        className="back-link"
        search={{ profile: undefined, status: undefined }}
        to="/works"
      >
        ← Back to queue
      </Link>
      <header className="detail-hero">
        <div>
          <p className="eyebrow">WORK RECORD</p>
          <h1>{work.data.goal}</h1>
        </div>
        <StatusBadge status={work.data.status} />
      </header>
      <section className="launch-strip" aria-label="Execution control">
        <div>
          <p className="panel-kicker">OPERATOR LAUNCH · FOREGROUND</p>
          <strong>
            {work.data.status === "ready" || work.data.status === "parked"
              ? "Ready for an explicit start"
              : work.data.status === "running"
                ? "An attempt owns this Work"
                : "Execution is confirmed"}
          </strong>
          <p>
            The persisted profile supplies the runtime. No command, secret, or
            workspace setting comes from this browser.
          </p>
          {startWork.isError && (
            <p className="form-error" role="alert">
              {startWork.error.message}
            </p>
          )}
        </div>
        <button
          className="launch-action"
          disabled={
            startWork.isPending ||
            (work.data.status !== "ready" && work.data.status !== "parked")
          }
          onClick={() => startWork.mutate(workId)}
          type="button"
        >
          <Play size={16} fill="currentColor" />
          {startWork.isPending ? "Attempt running…" : "Start Work"}
        </button>
      </section>
      <section className="fact-grid" aria-label="Work facts">
        <div>
          <span>WORK ID</span>
          <code>{work.data.workId}</code>
        </div>
        <div>
          <span>PROFILE</span>
          <strong>{work.data.workerProfile}</strong>
        </div>
        <div>
          <span>CREATED</span>
          <time dateTime={created.dateTime}>{created.label}</time>
        </div>
        <div>
          <span>WORKSPACE</span>
          <strong>{work.data.workspaceBound ? "bound" : "not bound"}</strong>
        </div>
        <div>
          <span>OUTCOME</span>
          <strong>{work.data.outcomeKind ?? "not confirmed"}</strong>
        </div>
      </section>
      {observation.isPending && (
        <p className="quiet observation-loading">Reading execution proof…</p>
      )}
      {observation.isError && (
        <QueryError
          message={observation.error.message}
          retry={() => void observation.refetch()}
        />
      )}
      {observation.data && (
        <>
          <DiagnosticList diagnostics={observation.data.diagnostics} />
          <section
            className="execution-section"
            aria-labelledby="execution-title"
          >
            <div className="section-heading execution-section-heading">
              <div>
                <p className="panel-kicker">COMPLETE OBSERVATION · v0.4</p>
                <h2 id="execution-title">Execution ledger</h2>
              </div>
              <span className="readout">OLDEST FIRST</span>
            </div>
            {observation.data.executions.length === 0 ? (
              <div className="empty-state compact-empty">
                <Fingerprint size={24} />
                <p>The first immutable execution will appear after launch.</p>
              </div>
            ) : (
              <div className="execution-list">
                {observation.data.executions.map((execution, index) => (
                  <ExecutionCard
                    execution={execution}
                    key={execution.executionId}
                    number={index + 1}
                  />
                ))}
              </div>
            )}
          </section>
          <ArtifactCatalogue artifacts={observation.data.artifacts} />
        </>
      )}
      <section
        className="content-panel timeline-panel"
        aria-labelledby="timeline-title"
      >
        <div className="section-heading">
          <div>
            <p className="panel-kicker">APPEND-ONLY HISTORY</p>
            <h2 id="timeline-title">Event timeline</h2>
          </div>
          <span className="readout">OLDEST FIRST</span>
        </div>
        {events.isPending && <p className="quiet">Reading recorded events…</p>}
        {events.isError && (
          <QueryError
            message={events.error.message}
            retry={() => void events.refetch()}
          />
        )}
        {events.data && (
          <>
            <Timeline events={allEvents} />
            <LoadMore
              hasMore={events.hasNextPage}
              loading={events.isFetchingNextPage}
              onLoad={() => void events.fetchNextPage()}
            />
          </>
        )}
      </section>
    </>
  );
}

export function DoctorPage() {
  const health = api.useHealth();
  const connection = useLiveConnection();
  return (
    <>
      <header className="page-header compact-header">
        <p className="eyebrow">RUNTIME HEALTH</p>
        <h1>Know the boundary.</h1>
        <p className="lede">
          Doctor verifies the local observer boundary; it does not probe Worker
          profiles or execution runtimes.
        </p>
      </header>
      <section className="doctor-grid">
        <div className="doctor-card">
          <Activity size={20} />
          <span>API</span>
          <strong>
            {health.data?.status === "ok" ? "reachable" : "checking"}
          </strong>
          <p>Same-origin local API.</p>
        </div>
        <div className="doctor-card">
          <Database size={20} />
          <span>STORE</span>
          <strong>{health.data?.store ?? "unknown"}</strong>
          <p>Read-only observer query succeeded.</p>
        </div>
        <div className="doctor-card">
          <Radio size={20} />
          <span>EVENTS</span>
          <strong>{connection}</strong>
          <p>Resumable SSE observer connection.</p>
        </div>
        <div className="doctor-card">
          <ShieldCheck size={20} />
          <span>LISTENER</span>
          <strong>localhost</strong>
          <p>No remote bind or authentication in v0.4.</p>
        </div>
      </section>
      {health.isError && (
        <QueryError
          message={health.error.message}
          retry={() => void health.refetch()}
        />
      )}
      <section
        className="content-panel capability-panel"
        aria-labelledby="capability-title"
      >
        <div className="section-heading">
          <div>
            <p className="panel-kicker">V0.4 CAPABILITIES</p>
            <h2 id="capability-title">What this surface may do</h2>
          </div>
          {health.data && <code>{health.data.version}</code>}
        </div>
        <dl className="capability-list">
          <div>
            <dt>Observe Work and events</dt>
            <dd className="yes">yes</dd>
          </div>
          <div>
            <dt>Create ready Work</dt>
            <dd className="yes">yes</dd>
          </div>
          <div>
            <dt>Explicitly start ready or parked Work</dt>
            <dd className="yes">yes</dd>
          </div>
          <div>
            <dt>Explain execution, attempts, and proof artifacts</dt>
            <dd className="yes">yes</dd>
          </div>
          <div>
            <dt>Park or abort active Work</dt>
            <dd>not yet</dd>
          </div>
          <div>
            <dt>Read workspace or secrets</dt>
            <dd>never</dd>
          </div>
        </dl>
      </section>
    </>
  );
}

export function NotFoundPage() {
  return (
    <section className="not-found">
      <CheckCircle2 size={24} />
      <p className="eyebrow">NOT FOUND</p>
      <h1>That record is not here.</h1>
      <Link
        className="primary-action"
        search={{ profile: undefined, status: undefined }}
        to="/works"
      >
        Return to queue
      </Link>
    </section>
  );
}
