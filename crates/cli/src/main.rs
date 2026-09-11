use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use workengine_adapters_http::serve;
use workengine_adapters_store::{SqliteObserver, SqliteStore};
use workengine_adapters_worker::{ProcessWorkerRunner, StubBehavior, StubWorkerRunner};
use workengine_adapters_workspace::DirWorkspaceFactory;
use workengine_application::{
    AppError, StartRequest, SystemClock, WorkQuery, create, next, park, recover_unconfirmed, start,
};
use workengine_domain::{DomainError, OutcomeKind, WorkId, WorkStatus};

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
    /// Serve the local Web operator intake and observer
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
    /// Park leftover running Work
    Park {
        #[arg(long)]
        work: String,
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
    match cli.command {
        Command::Version => {
            println!("{}", version_line());
            Ok(0)
        }
        Command::Create { goal, profile } => {
            let mut store = open_store(&data_dir, false)?;
            let work = create(&mut store, &SystemClock, goal, profile)?;
            println!("{} {}", work.id(), work.status());
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
            // Initialize a fresh local store if needed, but never recover Work.
            // The HTTP adapter uses this writer only for explicit operator intake.
            let store = SqliteStore::open(&data_dir)?;
            let observer = SqliteObserver::open(&data_dir)?;
            serve(store, observer, port, version_line())?;
            Ok(0)
        }
        Command::Start { work, checkout } => {
            let mut store = open_store(&data_dir, true)?;
            let workspaces = DirWorkspaceFactory::new(&data_dir);
            let id = WorkId::parse(work)?;
            let profile_name = store
                .get(&id)?
                .ok_or_else(|| AppError::NotFound(id.clone()))?
                .attributes()
                .worker_profile()
                .to_owned();
            let selected = select_runner(
                cli.config.as_deref(),
                cli.config_dir.as_deref(),
                &cli.stub_behavior,
                cli.retry_limit,
                &profile_name,
                checkout,
            )?;
            let budget = cli
                .budget_ms
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_BUDGET);
            let work = start(
                &mut store,
                &workspaces,
                &selected.runner,
                &SystemClock,
                &StartRequest {
                    id: &id,
                    budget,
                    retry_limit: selected.retry_limit,
                    checkout: selected.checkout.as_deref(),
                },
            )?;
            println!("{} {}", work.id(), work.status());
            Ok(exit_for_status(work.status(), outcome_kind(&store, &id)?))
        }
        Command::Park { work } => {
            let mut store = open_store(&data_dir, false)?;
            let id = WorkId::parse(work)?;
            let parked = park(&mut store, &SystemClock, &id)?;
            println!("{} {}", parked.id(), parked.status());
            Ok(0)
        }
    }
}

enum AnyRunner {
    Stub(StubWorkerRunner),
    Process(ProcessWorkerRunner),
}

impl workengine_application::WorkerRunner for AnyRunner {
    fn run(
        &self,
        request: &workengine_application::RunRequest<'_>,
    ) -> Result<workengine_domain::Outcome, AppError> {
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
                let runner = ProcessWorkerRunner::new(
                    resolved.argv,
                    resolved.secret_files,
                    resolved.sandbox,
                )?;
                return Ok(SelectedRunner {
                    runner: AnyRunner::Process(runner),
                    retry_limit: retry_limit.unwrap_or(resolved.retry_limit),
                    checkout: checkout.or(resolved.checkout),
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
    })
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

fn outcome_kind(store: &SqliteStore, id: &WorkId) -> Result<Option<OutcomeKind>, AppError> {
    Ok(store.events(id)?.last().and_then(|e| e.outcome_kind()))
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
