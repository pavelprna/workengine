use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use workengine_adapters_store::SqliteStore;
use workengine_adapters_worker::{StubBehavior, StubWorkerRunner, decode_outcome};
use workengine_adapters_workspace::DirWorkspaceFactory;
use workengine_application::{
    AppError, StartRequest, SystemClock, WorkStore, complete, create, next, park,
    recover_unconfirmed, start,
};
use workengine_domain::{DomainError, OutcomeKind, WorkId, WorkStatus};

const DEFAULT_BUDGET: Duration = Duration::from_secs(60);

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
    /// Extra channel retries. Hidden; default 0.
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
    /// Bind a workspace, run a Worker, and complete
    Start {
        #[arg(long)]
        work: String,
        /// Copy this directory into the workspace on first bind
        #[arg(long)]
        checkout: Option<PathBuf>,
    },
    /// Apply an outcome file to leftover running or parked Work
    Complete {
        #[arg(long)]
        work: String,
        #[arg(long)]
        file: PathBuf,
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
            let mut store = open_store(&data_dir, true)?;
            let work = create(&mut store, &SystemClock, goal, profile)?;
            println!("{} {}", work.id(), work.status());
            Ok(0)
        }
        Command::Next => {
            let store = open_store(&data_dir, true)?;
            if let Some(id) = next(&store)? {
                println!("{id}");
            }
            Ok(0)
        }
        Command::Start { work, checkout } => {
            let mut store = open_store(&data_dir, true)?;
            let workspaces = DirWorkspaceFactory::new(&data_dir);
            let behavior: StubBehavior = cli
                .stub_behavior
                .parse()
                .map_err(|err: String| anyhow::anyhow!(err))?;
            let runner = StubWorkerRunner::new(behavior);
            let id = WorkId::parse(work)?;
            let budget = cli
                .budget_ms
                .map(Duration::from_millis)
                .unwrap_or(DEFAULT_BUDGET);
            let work = start(
                &mut store,
                &workspaces,
                &runner,
                &SystemClock,
                &StartRequest {
                    id: &id,
                    budget,
                    retry_limit: cli.retry_limit.unwrap_or(0),
                    checkout: checkout.as_deref(),
                },
            )?;
            println!("{} {}", work.id(), work.status());
            Ok(exit_for_status(work.status(), outcome_kind(&store, &id)?))
        }
        Command::Complete { work, file } => {
            let mut store = open_store(&data_dir, false)?;
            let workspaces = DirWorkspaceFactory::new(&data_dir);
            let bytes = std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
            let outcome = decode_outcome(&bytes).map_err(anyhow::Error::from)?;
            let id = WorkId::parse(work)?;
            let done = complete(&mut store, &workspaces, &SystemClock, &id, &outcome)?;
            println!("{} {}", done.id(), done.status());
            Ok(exit_for_status(done.status(), Some(outcome.kind())))
        }
        Command::Park { work } => {
            let mut store = open_store(&data_dir, true)?;
            let id = WorkId::parse(work)?;
            let parked = park(&mut store, &SystemClock, &id)?;
            println!("{} {}", parked.id(), parked.status());
            Ok(0)
        }
    }
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
