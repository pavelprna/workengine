use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use workengine_application::{AppError, ProcessEvent, RunRequest};

use crate::stream::{
    EVENT_CHILD_STDERR, EVENT_CHILD_STDOUT, EVENT_EXITED, EVENT_KILLED, EVENT_SPAWNED, emit,
};

pub(crate) enum ChildWait {
    Exited { success: bool },
    BudgetExceeded,
}

pub(crate) fn spawn_supervised(
    mut cmd: Command,
    request: &mut RunRequest<'_>,
) -> Result<ChildWait, AppError> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    apply_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(AppError::worker)?;
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
        Ok(ChildWait::BudgetExceeded) => {
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
        Ok(ChildWait::BudgetExceeded) => record(request, ProcessEvent::Killed)?,
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

fn kill_group(pid: u32) {
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
