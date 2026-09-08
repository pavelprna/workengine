use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use workengine_application::AppError;

use crate::stream::{
    EVENT_CHILD_STDERR, EVENT_CHILD_STDOUT, EVENT_EXITED, EVENT_KILLED, EVENT_SPAWNED, emit,
};

pub(crate) enum ChildWait {
    Exited { success: bool },
    BudgetExceeded,
}

pub(crate) fn spawn_supervised(
    mut cmd: Command,
    work_id: &str,
    budget: Duration,
) -> Result<ChildWait, AppError> {
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    apply_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(AppError::worker)?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_h = drain_pipe(stdout, work_id.to_owned(), EVENT_CHILD_STDOUT);
    let err_h = drain_pipe(stderr, work_id.to_owned(), EVENT_CHILD_STDERR);
    emit(work_id, EVENT_SPAWNED, None);
    let result = wait_child(&mut child, budget);
    match &result {
        Ok(ChildWait::BudgetExceeded) => emit(work_id, EVENT_KILLED, None),
        Ok(ChildWait::Exited { .. }) => emit(work_id, EVENT_EXITED, None),
        Err(_) => {}
    }
    let _ = out_h.join();
    let _ = err_h.join();
    result
}

fn wait_child(child: &mut std::process::Child, budget: Duration) -> Result<ChildWait, AppError> {
    let deadline = Instant::now() + budget;
    loop {
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
    work_id: String,
    event: &'static str,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Some(pipe) = pipe else {
            return;
        };
        for line in BufReader::new(pipe).lines() {
            match line {
                Ok(line) => emit(&work_id, event, Some(&line)),
                Err(_) => break,
            }
        }
    })
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
