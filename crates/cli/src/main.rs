use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use workengine_adapters_http::{LifecycleControl, LocalClient, serve};
use workengine_adapters_store::{SqliteObserver, SqliteStore};
use workengine_adapters_worker::{
    ProcessWorkerRunner, StubBehavior, StubWorkerRunner, reclaim_owned_process,
};
use workengine_adapters_workspace::DirWorkspaceFactory;
use workengine_application::{
    AppError, AttemptState, ControlKind, OperatorInputKind, StartRequest, SystemClock, WorkQuery,
    WorkerExit, create, next, record_operator_input, recover_unconfirmed, request_control, start,
};
use workengine_domain::{
    ChannelPolicy, ContentDigest, DomainError, EXECUTION_SPEC_SCHEMA_VERSION, ExecutionSpec,
    OutcomeKind, RuntimeKind, SecretRef, Work, WorkId, WorkStatus,
};

mod profile;

const DEFAULT_BUDGET: Duration = Duration::from_secs(60);
const DEFAULT_SERVE_PORT: u16 = 9410;

/// Control plane for coding agents.
#[derive(Parser)]
#[command(
    name = "workengine",
    version,
    about = "Control plane for coding agents"
)]
#[command(arg_required_else_help = true)]
struct Cli {
    /// Directory for the SQLite store and workspaces
    #[arg(long, env = "WORKENGINE_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Worker profile file (TOML). Validated lazily for the Work's profile.
    #[arg(long, env = "WORKENGINE_CONFIG", conflicts_with = "config_dir")]
    config: Option<PathBuf>,
    /// Directory with one `<profile>.toml` file per Worker profile.
    #[arg(long, env = "WORKENGINE_CONFIG_DIR", conflicts_with = "config")]
    config_dir: Option<PathBuf>,
    /// Hidden first-slice knobs for the stub Worker. Not a public contract.
    #[arg(
        long,
        env = "WORKENGINE_STUB_BEHAVIOR",
        default_value = "succeed",
        hide = true
    )]
    stub_behavior: String,
    /// Worker time budget in milliseconds. Hidden; default 60000.
    #[arg(long, env = "WORKENGINE_BUDGET_MS", hide = true)]
    budget_ms: Option<u64>,
    /// Extra channel retries. Hidden; default 0, or the profile's retry_limit.
    #[arg(long, env = "WORKENGINE_RETRY_LIMIT", hide = true)]
    retry_limit: Option<u32>,
    /// Local daemon TCP port used by lifecycle commands.
    #[arg(long, env = "WORKENGINE_DAEMON_PORT", default_value_t = DEFAULT_SERVE_PORT)]
    daemon_port: u16,
    /// Test-only foreground control path.
    #[arg(long, env = "WORKENGINE_DIRECT_CONTROL", hide = true)]
    direct_control: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the Workengine version
    Version,
    /// Persist a new Work as ready
    Create {
        /// What this Work is for
        #[arg(long)]
        goal: String,
        /// Worker profile name (opaque configuration key)
        #[arg(long, default_value = "stub")]
        profile: String,
    },
    /// Print the next startable Work id
    Next,
    /// List Work snapshots
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one Work snapshot
    Show {
        #[arg(long)]
        work: String,
        #[arg(long)]
        json: bool,
    },
    /// Read append-only events as JSONL. Resume with `--after-seq`.
    Events {
        #[arg(long, default_value_t = 0)]
        after_seq: u64,
        #[arg(long)]
        work: Option<String>,
    },
    /// Run the local lifecycle daemon, Web operator, and observer
    Serve {
        /// Localhost TCP port for the Web observer
        #[arg(long, default_value_t = DEFAULT_SERVE_PORT)]
        port: u16,
    },
    /// Bind a workspace, run a Worker, and complete
    Start {
        #[arg(long)]
        work: String,
        /// Copy this directory into the workspace on first bind
        #[arg(long)]
        checkout: Option<PathBuf>,
    },
    /// Continue parked Work under the same execution
    Resume {
        #[arg(long)]
        work: String,
    },
    /// Request a validated checkpoint and park running Work
    Park {
        #[arg(long)]
        work: String,
    },
    /// Immediately tear down and fail running Work
    Abort {
        #[arg(long)]
        work: String,
    },
    /// Record an operator answer and continue parked Work
    Answer {
        #[arg(long)]
        work: String,
        #[arg(long)]
        answer: String,
    },
    /// Record explicit consent and continue parked Work
    Consent {
        #[arg(long)]
        work: String,
        #[arg(long)]
        action: String,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(err) => {
            eprintln!("{err:#}");
            ExitCode::from(exit_status(&err))
        }
    }
}

fn run() -> anyhow::Result<u8> {
    init_tracing();
    let cli = Cli::parse();
    let data_dir = cli
        .data_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(".workengine"));
    let launch_options = LaunchOptions::from_cli(&cli, data_dir.clone());
    let client = LocalClient::new(cli.daemon_port);
    match cli.command {
        Command::Version => {
            println!("{}", version_line());
            Ok(0)
        }
        Command::Create { goal, profile } => {
            if cli.direct_control {
                let work = launch_options.create(goal, profile)?;
                println!("{} {}", work.id(), work.status());
            } else {
                let work = client.create(&goal, &profile)?;
                println!("{} {}", work.work_id, work.status);
            }
            Ok(0)
        }
        Command::Next => {
            let observer = SqliteObserver::open(&data_dir)?;
            if let Some(id) = next(&observer)? {
                println!("{id}");
            }
            Ok(0)
        }
        Command::List { json } => {
            let observer = SqliteObserver::open(&data_dir)?;
            let works = observer.list()?;
            for work in works {
                if json {
                    println!("{}", serde_json::to_string(&work_json(&observer, &work))?);
                } else {
                    println!("{} {}", work.id(), work.status());
                }
            }
            Ok(0)
        }
        Command::Show { work, json } => {
            let observer = SqliteObserver::open(&data_dir)?;
            let id = WorkId::parse(work)?;
            let work = observer.get(&id)?.ok_or(AppError::NotFound(id))?;
            if json {
                println!("{}", serde_json::to_string(&work_json(&observer, &work))?);
            } else {
                println!("{} {}", work.id(), work.status());
            }
            Ok(0)
        }
        Command::Events { after_seq, work } => {
            let observer = SqliteObserver::open(&data_dir)?;
            let id = work.map(WorkId::parse).transpose()?;
            for event in observer.events_after(after_seq, id.as_ref())? {
                println!("{}", serde_json::to_string(&event_json(&event))?);
            }
            Ok(0)
        }
        Command::Serve { port } => {
            let _daemon_lock = acquire_daemon_lock(&data_dir)?;
            let mut store = SqliteStore::open(&data_dir)?;
            reclaim_active_processes(&data_dir, &store)?;
            recover_unconfirmed(&mut store, &SystemClock)?;
            drop(store);
            let observer = SqliteObserver::open(&data_dir)?;
            let control = Arc::new(launch_options);
            serve(observer, control, port, version_line())?;
            Ok(0)
        }
        Command::Start { work, checkout } => {
            if checkout.is_some() && !cli.direct_control {
                return Err(anyhow::anyhow!(
                    "start --checkout cannot cross the daemon boundary; configure checkout in the Worker profile"
                ));
            }
            let id = WorkId::parse(work)?;
            if cli.direct_control {
                let work = launch_options.start_with_checkout(&id, checkout)?;
                println!("{} {}", work.id(), work.status());
                let observer = SqliteObserver::open(&data_dir)?;
                Ok(exit_for_status(
                    work.status(),
                    outcome_kind(&observer, &id)?,
                ))
            } else {
                let work = client.start(&id)?;
                println!("{} {}", work.work_id, work.status);
                Ok(exit_for_status(work.status, work.outcome_kind))
            }
        }
        Command::Resume { work } => {
            let id = WorkId::parse(work)?;
            let work = client.resume(&id)?;
            println!("{} {}", work.work_id, work.status);
            Ok(exit_for_status(work.status, work.outcome_kind))
        }
        Command::Park { work } => {
            let id = WorkId::parse(work)?;
            if cli.direct_control {
                let mut store = SqliteStore::open(&data_dir)?;
                let parked = workengine_application::park(&mut store, &SystemClock, &id)?;
                println!("{} {}", parked.id(), parked.status());
            } else {
                let parked = client.park(&id)?;
                println!("{} {}", parked.work_id, parked.status);
            }
            Ok(0)
        }
        Command::Abort { work } => {
            let id = WorkId::parse(work)?;
            let aborted = client.abort(&id)?;
            println!("{} {}", aborted.work_id, aborted.status);
            Ok(0)
        }
        Command::Answer { work, answer } => {
            let id = WorkId::parse(work)?;
            let resumed = client.answer(&id, &answer)?;
            println!("{} {}", resumed.work_id, resumed.status);
            Ok(exit_for_status(resumed.status, resumed.outcome_kind))
        }
        Command::Consent { work, action } => {
            let id = WorkId::parse(work)?;
            let resumed = client.consent(&id, &action)?;
            println!("{} {}", resumed.work_id, resumed.status);
            Ok(exit_for_status(resumed.status, resumed.outcome_kind))
        }
    }
}

#[derive(Clone)]
struct LaunchOptions {
    data_dir: PathBuf,
    config: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    stub_behavior: String,
    budget_ms: Option<u64>,
    retry_limit: Option<u32>,
}

impl LaunchOptions {
    fn from_cli(cli: &Cli, data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            config: cli.config.clone(),
            config_dir: cli.config_dir.clone(),
            stub_behavior: cli.stub_behavior.clone(),
            budget_ms: cli.budget_ms,
            retry_limit: cli.retry_limit,
        }
    }

    fn start_with_checkout(&self, id: &WorkId, checkout: Option<PathBuf>) -> anyhow::Result<Work> {
        let mut store = open_store(&self.data_dir, false)?;
        let profile_name = store
            .get(id)?
            .ok_or_else(|| AppError::NotFound(id.clone()))?
            .attributes()
            .worker_profile()
            .to_owned();
        let selected = select_runner(
            self.config.as_deref(),
            self.config_dir.as_deref(),
            &self.stub_behavior,
            self.retry_limit,
            &profile_name,
            checkout,
        )?;
        let budget = self
            .budget_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_BUDGET);
        let execution_spec = selected.execution_spec(id, &profile_name, budget)?;
        let workspaces = DirWorkspaceFactory::new(&self.data_dir);
        Ok(start(
            &mut store,
            &workspaces,
            &selected.runner,
            &SystemClock,
            &StartRequest {
                id,
                execution_spec: &execution_spec,
                checkout: selected.checkout.as_deref(),
            },
        )?)
    }
}

impl LifecycleControl for LaunchOptions {
    fn create(&self, goal: String, profile: String) -> Result<Work, AppError> {
        create(
            &mut SqliteStore::open(&self.data_dir)?,
            &SystemClock,
            goal,
            profile,
        )
    }

    fn start(&self, id: &WorkId) -> Result<Work, AppError> {
        self.start_with_checkout(id, None).map_err(|error| {
            error
                .downcast::<AppError>()
                .unwrap_or_else(|error| AppError::worker(error.to_string()))
        })
    }

    fn resume(&self, id: &WorkId) -> Result<Work, AppError> {
        let observer = SqliteObserver::open(&self.data_dir)?;
        let work = observer
            .get(id)?
            .ok_or_else(|| AppError::NotFound(id.clone()))?;
        if work.status() != WorkStatus::Parked {
            return Err(AppError::Conflict(format!("Work {id} is not parked")));
        }
        self.start(id)
    }

    fn park(&self, id: &WorkId) -> Result<Work, AppError> {
        self.interrupt(id, ControlKind::Park)
    }

    fn abort(&self, id: &WorkId) -> Result<Work, AppError> {
        self.interrupt(id, ControlKind::Abort)
    }

    fn answer(&self, id: &WorkId, answer: String) -> Result<Work, AppError> {
        self.input_and_resume(id, OperatorInputKind::Answer, answer)
    }

    fn consent(&self, id: &WorkId, action: String) -> Result<Work, AppError> {
        self.input_and_resume(id, OperatorInputKind::Consent, action)
    }
}

impl LaunchOptions {
    fn interrupt(&self, id: &WorkId, kind: ControlKind) -> Result<Work, AppError> {
        let mut store = SqliteStore::open(&self.data_dir)?;
        request_control(&mut store, &SystemClock, id, kind)?;
        drop(store);
        let deadline = std::time::Instant::now()
            + self
                .budget_ms
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_BUDGET)
            + Duration::from_secs(2);
        loop {
            let observer = SqliteObserver::open(&self.data_dir)?;
            let work = observer
                .get(id)?
                .ok_or_else(|| AppError::NotFound(id.clone()))?;
            if work.status() != WorkStatus::Running {
                let expected = match kind {
                    ControlKind::Park => WorkStatus::Parked,
                    ControlKind::Abort => WorkStatus::Failed,
                };
                if work.status() != expected {
                    return Err(AppError::Conflict(format!(
                        "control ended in {} instead of {expected}",
                        work.status()
                    )));
                }
                return Ok(work);
            }
            if std::time::Instant::now() >= deadline {
                return Err(AppError::Conflict(format!(
                    "timed out waiting for {kind:?} confirmation"
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn input_and_resume(
        &self,
        id: &WorkId,
        kind: OperatorInputKind,
        body: String,
    ) -> Result<Work, AppError> {
        let mut store = SqliteStore::open(&self.data_dir)?;
        let workspaces = DirWorkspaceFactory::new(&self.data_dir);
        record_operator_input(&mut store, &workspaces, &SystemClock, id, kind, body)?;
        drop(store);
        self.resume(id)
    }
}

enum AnyRunner {
    Stub(StubWorkerRunner),
    Process(ProcessWorkerRunner),
}

impl workengine_application::WorkerRunner for AnyRunner {
    fn run(
        &self,
        request: &mut workengine_application::RunRequest<'_>,
    ) -> Result<WorkerExit, AppError> {
        match self {
            Self::Stub(runner) => runner.run(request),
            Self::Process(runner) => runner.run(request),
        }
    }

    fn decode(&self, bytes: &[u8]) -> Result<workengine_domain::Outcome, AppError> {
        match self {
            Self::Stub(runner) => runner.decode(bytes),
            Self::Process(runner) => runner.decode(bytes),
        }
    }
}

struct SelectedRunner {
    runner: AnyRunner,
    retry_limit: u32,
    checkout: Option<PathBuf>,
    worker_config_digest: ContentDigest,
    runtime_kind: RuntimeKind,
    runtime_digest: ContentDigest,
    secret_refs: Vec<SecretRef>,
}

impl SelectedRunner {
    fn execution_spec(
        &self,
        work_id: &WorkId,
        profile: &str,
        budget: Duration,
    ) -> anyhow::Result<ExecutionSpec> {
        Ok(ExecutionSpec::new(
            EXECUTION_SPEC_SCHEMA_VERSION,
            work_id.clone(),
            profile,
            self.worker_config_digest.clone(),
            self.runtime_kind,
            self.runtime_digest.clone(),
            u64::try_from(budget.as_millis()).unwrap_or(u64::MAX),
            self.retry_limit,
            if self.retry_limit == 0 {
                ChannelPolicy::Fail
            } else {
                ChannelPolicy::RetryThenFail
            },
            self.secret_refs.clone(),
        )?)
    }
}

fn config_has_profile(table: &toml::Table, name: &str) -> bool {
    table
        .get("profile")
        .and_then(toml::Value::as_table)
        .is_some_and(|profiles| profiles.contains_key(name))
}

fn select_runner(
    config: Option<&std::path::Path>,
    config_dir: Option<&std::path::Path>,
    stub_behavior: &str,
    retry_limit: Option<u32>,
    profile_name: &str,
    checkout: Option<PathBuf>,
) -> anyhow::Result<SelectedRunner> {
    if let Some(path) = config.or(config_dir) {
        let table = match config_dir {
            Some(dir) => profile::load_profile_from_dir(dir, profile_name)?,
            None => profile::load_table(path)?,
        };
        let listed = config_has_profile(&table, profile_name);
        match profile::resolve_profile(&table, profile_name, |key| std::env::var(key).ok()) {
            Ok(resolved) => {
                let config_material = format!(
                    "argv={:?};sandbox={:?};secrets={:?}",
                    resolved.argv,
                    resolved.sandbox,
                    resolved
                        .secret_refs
                        .iter()
                        .map(|reference| (reference.name(), reference.source().reference()))
                        .collect::<Vec<_>>()
                );
                let runtime_material = format!("{:?}", resolved.sandbox);
                let runtime_kind = match &resolved.sandbox {
                    workengine_adapters_worker::Sandbox::Bubblewrap { .. } => {
                        RuntimeKind::Bubblewrap
                    }
                    workengine_adapters_worker::Sandbox::Oci { .. } => RuntimeKind::Oci,
                };
                let runner = ProcessWorkerRunner::new(
                    resolved.argv,
                    resolved.secret_files,
                    resolved.sandbox,
                )?;
                return Ok(SelectedRunner {
                    runner: AnyRunner::Process(runner),
                    retry_limit: retry_limit.unwrap_or(resolved.retry_limit),
                    checkout: checkout.or(resolved.checkout),
                    worker_config_digest: digest(config_material.as_bytes())?,
                    runtime_kind,
                    runtime_digest: digest(runtime_material.as_bytes())?,
                    secret_refs: resolved.secret_refs,
                });
            }
            Err(_) if profile_name == "stub" && !listed => {}
            Err(err) => return Err(err),
        }
    } else if profile_name != "stub" {
        anyhow::bail!("profile {profile_name} needs --config-dir with a profile file");
    }

    let behavior: StubBehavior = stub_behavior
        .parse()
        .map_err(|err: String| anyhow::anyhow!(err))?;
    Ok(SelectedRunner {
        runner: AnyRunner::Stub(StubWorkerRunner::new(behavior)),
        retry_limit: retry_limit.unwrap_or(0),
        checkout,
        worker_config_digest: digest(format!("stub:{stub_behavior}").as_bytes())?,
        runtime_kind: RuntimeKind::Stub,
        runtime_digest: digest(b"workengine-stub-process-v1")?,
        secret_refs: Vec::new(),
    })
}

fn digest(bytes: &[u8]) -> anyhow::Result<ContentDigest> {
    let hex = format!("{:x}", Sha256::digest(bytes));
    Ok(ContentDigest::parse(format!("sha256:{hex}"))?)
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn version_line() -> String {
    let ver = env!("CARGO_PKG_VERSION");
    let sha = env!("WORKENGINE_GIT_SHA");
    if sha.is_empty() {
        format!("workengine {ver}")
    } else {
        format!("workengine {ver} ({sha})")
    }
}

fn open_store(data_dir: &std::path::Path, recover: bool) -> anyhow::Result<SqliteStore> {
    let mut store = SqliteStore::open(data_dir)?;
    if recover {
        recover_unconfirmed(&mut store, &SystemClock)?;
    }
    Ok(store)
}

fn acquire_daemon_lock(data_dir: &std::path::Path) -> Result<std::fs::File, AppError> {
    std::fs::create_dir_all(data_dir).map_err(AppError::store)?;
    let path = data_dir.join("daemon.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(AppError::store)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(AppError::store)?;
    }
    file.try_lock().map_err(|error| {
        AppError::Conflict(format!("another daemon owns this data directory: {error}"))
    })?;
    Ok(file)
}

fn reclaim_active_processes(
    data_dir: &std::path::Path,
    query: &impl WorkQuery,
) -> anyhow::Result<()> {
    for work in query
        .list()?
        .into_iter()
        .filter(|work| work.status() == WorkStatus::Running)
    {
        for execution in query.executions(work.id())? {
            if let Some(attempt) = execution
                .attempts
                .last()
                .filter(|attempt| attempt.state == AttemptState::Active)
            {
                let control_root = data_dir
                    .join("control")
                    .join(work.id().as_str())
                    .join(execution.execution_id.as_str())
                    .join(attempt.attempt_id.as_str());
                reclaim_owned_process(
                    &control_root,
                    work.id().as_str(),
                    execution.execution_id.as_str(),
                    attempt.attempt_id.as_str(),
                )?;
            }
        }
    }
    Ok(())
}

fn outcome_kind(store: &impl WorkQuery, id: &WorkId) -> Result<Option<OutcomeKind>, AppError> {
    Ok(store
        .events(id)?
        .last()
        .and_then(|event| event.outcome_kind()))
}

fn work_json(query: &impl WorkQuery, work: &workengine_domain::Work) -> serde_json::Value {
    let outcome = query
        .events(work.id())
        .ok()
        .and_then(|events| events.last().and_then(|event| event.outcome_kind()))
        .map(|kind| kind.as_str());
    serde_json::json!({
        "workId": work.id().as_str(),
        "status": work.status().as_str(),
        "goal": work.attributes().goal(),
        "workerProfile": work.attributes().worker_profile(),
        "workspaceRoot": work.workspace_root(),
        "createdAtUnixMs": work.created_at_unix_ms(),
        "outcomeKind": outcome,
        "activeExecution": null,
        "activeAttempt": null,
        "checkpoint": null,
    })
}

fn event_json(event: &workengine_application::SequencedEvent) -> serde_json::Value {
    let record = &event.event;
    serde_json::json!({
        "schemaVersion": record.schema_version(),
        "workId": record.work_id().as_str(),
        "kind": record.kind().as_str(),
        "from": record.from().map(WorkStatus::as_str),
        "to": record.to().as_str(),
        "goal": record.attributes().map(|attributes| attributes.goal()),
        "workerProfile": record.attributes().map(|attributes| attributes.worker_profile()),
        "outcomeKind": record.outcome_kind().map(|kind| kind.as_str()),
        "workspaceRoot": record.workspace_root(),
        "createdAtUnixMs": record.created_at_unix_ms(),
        "seq": event.seq,
    })
}

fn exit_for_status(status: WorkStatus, kind: Option<OutcomeKind>) -> u8 {
    match (status, kind) {
        (WorkStatus::Succeeded, _) => 0,
        (_, Some(OutcomeKind::BudgetExceeded)) => 20,
        (_, Some(OutcomeKind::TimedOut)) => 21,
        (WorkStatus::Failed, _) => 1,
        _ => 0,
    }
}

fn exit_status(err: &anyhow::Error) -> u8 {
    if let Some(app) = err.downcast_ref::<AppError>() {
        return match app {
            AppError::Domain(DomainError::IllegalTransition { .. })
            | AppError::Domain(DomainError::TerminalConflict(_)) => 10,
            AppError::Conflict(_) | AppError::Store(_) => 11,
            AppError::OutcomeSchema(_) => 30,
            AppError::Workspace(_) => 40,
            AppError::NotFound(_) => 2,
            AppError::Domain(_) | AppError::Worker(_) | AppError::Channel(_) => 1,
        };
    }
    2
}
