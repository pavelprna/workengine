use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use workengine_application::{AppError, ProcessEvent, RunRequest};
use workengine_application::{ControlDirective, ControlKind};

use crate::stream::{
    EVENT_CHILD_STDERR, EVENT_CHILD_STDOUT, EVENT_EXITED, EVENT_KILLED, EVENT_SPAWNED, emit,
};

pub(crate) enum ChildWait {
    Exited { success: bool },
    BudgetExceeded,
    Parked { control_request_id: Option<i64> },
    Aborted { control_request_id: i64 },
}

pub(crate) fn spawn_supervised(
    mut cmd: Command,
    request: &mut RunRequest<'_>,
) -> Result<ChildWait, AppError> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    apply_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(AppError::worker)?;
    record_runtime_owner(request, child.id())?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (sender, receiver) = mpsc::channel();
    let out_h = drain_pipe(stdout, sender.clone(), ProcessEvent::ChildStdout);
    let err_h = drain_pipe(stderr, sender, ProcessEvent::ChildStderr);
    let work_id = request.work.id().as_str().to_owned();
    emit(&work_id, EVENT_SPAWNED, None);
    if let Err(error) = record(request, ProcessEvent::Spawned) {
        kill_group(child.id());
        let _ = child.wait();
        let _ = out_h.join();
        let _ = err_h.join();
        return Err(error);
    }
    let result = wait_child(&mut child, request, &receiver);
    match &result {
        Ok(ChildWait::BudgetExceeded | ChildWait::Parked { .. } | ChildWait::Aborted { .. }) => {
            emit(&work_id, EVENT_KILLED, None);
        }
        Ok(ChildWait::Exited { .. }) => {
            // The direct child can exit while a background descendant still
            // owns stdout/stderr or continues to mutate the workspace. A
            // Worker is the whole process group, so successful parent exit
            // also tears down anything left in that group before completion.
            kill_group(child.id());
            emit(&work_id, EVENT_EXITED, None);
        }
        Err(_) => {
            kill_group(child.id());
            let _ = child.wait();
        }
    }
    let _ = out_h.join();
    let _ = err_h.join();
    drain_records(request, &receiver)?;
    match &result {
        Ok(ChildWait::BudgetExceeded | ChildWait::Parked { .. } | ChildWait::Aborted { .. }) => {
            record(request, ProcessEvent::Killed)?
        }
        Ok(ChildWait::Exited { .. }) => record(request, ProcessEvent::Exited)?,
        Err(_) => {}
    }
    result
}

fn wait_child(
    child: &mut std::process::Child,
    request: &mut RunRequest<'_>,
    receiver: &Receiver<ProcessEvent>,
) -> Result<ChildWait, AppError> {
    let deadline = Instant::now() + request.budget;
    let mut next_heartbeat = Instant::now() + Duration::from_secs(1);
    let mut park_request = None;
    loop {
        if let Err(error) = drain_records(request, receiver) {
            kill_group(child.id());
            let _ = child.wait();
            return Err(error);
        }
        if Instant::now() >= next_heartbeat {
            if let Err(error) = heartbeat(request) {
                kill_group(child.id());
                let _ = child.wait();
                return Err(error);
            }
            next_heartbeat = Instant::now() + Duration::from_secs(1);
        }
        if park_request.is_none()
            && let Some(directive) = request.recorder.control_directive(
                request.work.id(),
                request.execution_id,
                request.attempt_id,
            )?
        {
            match directive.kind {
                ControlKind::Abort => {
                    kill_group(child.id());
                    let _ = child.wait();
                    return Ok(ChildWait::Aborted {
                        control_request_id: directive.request_id,
                    });
                }
                ControlKind::Park => {
                    write_checkpoint_request(request, directive)?;
                    park_request = Some(directive.request_id);
                }
            }
        }
        if let Some(control_request_id) = park_request
            && checkpoint_is_valid(request)?
        {
            request.recorder.record_checkpoint(
                request.work.id(),
                request.execution_id,
                request.attempt_id,
                now_unix_ms()?,
            )?;
            kill_group(child.id());
            let _ = child.wait();
            return Ok(ChildWait::Parked {
                control_request_id: Some(control_request_id),
            });
        }
        match child.try_wait().map_err(AppError::worker)? {
            Some(status) => {
                return Ok(ChildWait::Exited {
                    success: status.success(),
                });
            }
            None if Instant::now() >= deadline => {
                kill_group(child.id());
                let _ = child.wait();
                return Ok(ChildWait::BudgetExceeded);
            }
            None => thread::sleep(Duration::from_millis(5)),
        }
    }
}

const CHECKPOINT_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckpointRequest<'a> {
    schema_version: u32,
    kind: &'static str,
    work_id: &'a str,
    execution_id: &'a str,
    attempt_id: &'a str,
    worker_profile: &'a str,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CheckpointCandidate {
    schema_version: u32,
    work_id: String,
    execution_id: String,
    attempt_id: String,
    worker_profile: String,
}

fn write_checkpoint_request(
    request: &RunRequest<'_>,
    directive: ControlDirective,
) -> Result<(), AppError> {
    let payload = CheckpointRequest {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        kind: directive.kind.as_str(),
        work_id: request.work.id().as_str(),
        execution_id: request.execution_id.as_str(),
        attempt_id: request.attempt_id.as_str(),
        worker_profile: request.work.attributes().worker_profile(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(AppError::worker)?;
    fs::write(request.control_root.join("checkpoint-request.json"), bytes).map_err(AppError::worker)
}

fn checkpoint_is_valid(request: &RunRequest<'_>) -> Result<bool, AppError> {
    let bytes = match fs::read(request.control_root.join("checkpoint.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(AppError::worker(error)),
    };
    let candidate: CheckpointCandidate =
        serde_json::from_slice(&bytes).map_err(AppError::outcome_schema)?;
    if candidate.schema_version != CHECKPOINT_SCHEMA_VERSION
        || candidate.work_id != request.work.id().as_str()
        || candidate.execution_id != request.execution_id.as_str()
        || candidate.attempt_id != request.attempt_id.as_str()
        || candidate.worker_profile != request.work.attributes().worker_profile()
    {
        return Err(AppError::outcome_schema(
            "checkpoint candidate does not match the active attempt",
        ));
    }
    Ok(true)
}

pub(crate) fn write_checkpoint_candidate(request: &RunRequest<'_>) -> Result<(), AppError> {
    let payload = CheckpointCandidate {
        schema_version: CHECKPOINT_SCHEMA_VERSION,
        work_id: request.work.id().to_string(),
        execution_id: request.execution_id.to_string(),
        attempt_id: request.attempt_id.to_string(),
        worker_profile: request.work.attributes().worker_profile().to_owned(),
    };
    fs::write(
        request.control_root.join("checkpoint.json"),
        serde_json::to_vec(&payload).map_err(AppError::worker)?,
    )
    .map_err(AppError::worker)
}

fn drain_pipe<R: Read + Send + 'static>(
    pipe: Option<R>,
    sender: Sender<ProcessEvent>,
    event: ProcessEvent,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Some(pipe) = pipe else {
            return;
        };
        for line in BufReader::new(pipe).lines() {
            match line {
                // Child output is untrusted and can contain a credential
                // supplied to the Worker. The public stream records that
                // output happened, but never republishes its bytes.
                Ok(_line) => {
                    if sender.send(event).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn drain_records(
    request: &mut RunRequest<'_>,
    receiver: &Receiver<ProcessEvent>,
) -> Result<(), AppError> {
    while let Ok(event) = receiver.try_recv() {
        let stream_event = match event {
            ProcessEvent::ChildStdout => EVENT_CHILD_STDOUT,
            ProcessEvent::ChildStderr => EVENT_CHILD_STDERR,
            ProcessEvent::Spawned => EVENT_SPAWNED,
            ProcessEvent::Exited => EVENT_EXITED,
            ProcessEvent::Killed => EVENT_KILLED,
        };
        emit(request.work.id().as_str(), stream_event, None);
        record(request, event)?;
    }
    Ok(())
}

fn record(request: &mut RunRequest<'_>, event: ProcessEvent) -> Result<(), AppError> {
    request.recorder.record_process_event(
        request.work.id(),
        request.execution_id,
        request.attempt_id,
        event,
        now_unix_ms()?,
    )
}

fn heartbeat(request: &mut RunRequest<'_>) -> Result<(), AppError> {
    request.recorder.heartbeat_attempt(
        request.work.id(),
        request.execution_id,
        request.attempt_id,
        now_unix_ms()?,
    )
}

fn now_unix_ms() -> Result<u64, AppError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(AppError::worker)?;
    Ok(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

fn apply_process_group(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
}

pub(crate) fn kill_group(pid: u32) {
    #[cfg(unix)]
    {
        use nix::sys::signal::{self, Signal};
        use nix::unistd::Pid;
        let pid = Pid::from_raw(pid as i32);
        let _ = signal::killpg(pid, Signal::SIGTERM);
        thread::sleep(Duration::from_millis(50));
        let _ = signal::killpg(pid, Signal::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        // First-slice process-group teardown is Unix. See docs/product/threat-model.md.
        let _ = pid;
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RuntimeOwner {
    schema_version: u32,
    work_id: String,
    execution_id: String,
    attempt_id: String,
    pid: u32,
    process_group: i32,
    start_ticks: u64,
}

fn record_runtime_owner(request: &RunRequest<'_>, pid: u32) -> Result<(), AppError> {
    #[cfg(target_os = "linux")]
    {
        let (process_group, start_ticks) = linux_process_identity(pid)?;
        if process_group != pid as i32 {
            return Err(AppError::worker(
                "Worker process group does not match its leader",
            ));
        }
        let owner = RuntimeOwner {
            schema_version: 1,
            work_id: request.work.id().to_string(),
            execution_id: request.execution_id.to_string(),
            attempt_id: request.attempt_id.to_string(),
            pid,
            process_group,
            start_ticks,
        };
        fs::write(
            request.control_root.join("runtime-owner.json"),
            serde_json::to_vec(&owner).map_err(AppError::worker)?,
        )
        .map_err(AppError::worker)?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (request, pid);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_process_identity(pid: u32) -> Result<(i32, u64), AppError> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).map_err(AppError::worker)?;
    let tail = stat
        .rsplit_once(") ")
        .map(|(_, tail)| tail)
        .ok_or_else(|| AppError::worker("invalid Linux process identity"))?;
    let fields: Vec<&str> = tail.split_whitespace().collect();
    let process_group = fields
        .get(2)
        .ok_or_else(|| AppError::worker("missing Linux process group"))?
        .parse()
        .map_err(AppError::worker)?;
    let start_ticks = fields
        .get(19)
        .ok_or_else(|| AppError::worker("missing Linux process start time"))?
        .parse()
        .map_err(AppError::worker)?;
    Ok((process_group, start_ticks))
}

pub(crate) fn reclaim_runtime_owner(
    control_root: &std::path::Path,
    work_id: &str,
    execution_id: &str,
    attempt_id: &str,
) -> Result<bool, AppError> {
    #[cfg(target_os = "linux")]
    {
        let bytes = match fs::read(control_root.join("runtime-owner.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(AppError::worker(error)),
        };
        let owner: RuntimeOwner =
            serde_json::from_slice(&bytes).map_err(AppError::outcome_schema)?;
        if owner.schema_version != 1
            || owner.work_id != work_id
            || owner.execution_id != execution_id
            || owner.attempt_id != attempt_id
            || owner.process_group != owner.pid as i32
        {
            return Err(AppError::worker(
                "runtime ownership proof does not match the active attempt",
            ));
        }
        match linux_process_identity(owner.pid) {
            Ok((process_group, start_ticks))
                if process_group == owner.process_group && start_ticks == owner.start_ticks =>
            {
                kill_group(owner.pid);
                Ok(true)
            }
            Ok(_) | Err(_) => Ok(false),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (control_root, work_id, execution_id, attempt_id);
        Ok(false)
    }
}
