use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;
pub(crate) const DEFAULT_NODE_TIMEOUT_SECONDS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConcurrentTaskTimeout(Duration);

impl ConcurrentTaskTimeout {
    pub(crate) fn from_secs(seconds: u64) -> Self {
        Self(Duration::from_secs(seconds))
    }

    pub(crate) fn duration(self) -> Duration {
        self.0
    }
}

#[derive(Debug)]
pub(crate) enum NodeTimeoutError {
    Io(std::io::Error),
    TimedOut { elapsed: Duration, limit: Duration },
}

pub(crate) fn terminate_process_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let _ = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = child.kill();
    }
}

pub(crate) fn run_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Output, NodeTimeoutError> {
    use std::io::Read;
    use std::thread;

    let started = std::time::Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(NodeTimeoutError::Io)?;
    let mut stdout = child.stdout.take().ok_or_else(|| {
        NodeTimeoutError::Io(std::io::Error::other("child stdout is unavailable"))
    })?;
    let mut stderr = child.stderr.take().ok_or_else(|| {
        NodeTimeoutError::Io(std::io::Error::other("child stderr is unavailable"))
    })?;
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });

    loop {
        let status = child.try_wait().map_err(NodeTimeoutError::Io)?;
        if let Some(status) = status {
            let stdout = stdout_reader
                .join()
                .map_err(|_| NodeTimeoutError::Io(std::io::Error::other("stdout reader panicked")))?
                .map_err(NodeTimeoutError::Io)?;
            let stderr = stderr_reader
                .join()
                .map_err(|_| NodeTimeoutError::Io(std::io::Error::other("stderr reader panicked")))?
                .map_err(NodeTimeoutError::Io)?;
            return Ok(Output {
                status,
                stdout,
                stderr,
            });
        }
        if started.elapsed() >= timeout {
            terminate_process_tree(&mut child);
            let _ = child.wait();
            // Reader threads are detached on timeout: a descendant may retain
            // the inherited pipe handles after the direct child is killed.
            return Err(NodeTimeoutError::TimedOut {
                elapsed: started.elapsed(),
                limit: timeout,
            });
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;

    use super::{run_with_timeout, NodeTimeoutError};

    #[test]
    fn hanging_child_is_classified_as_timeout_with_limit_and_elapsed_time() {
        let command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/C", "ping 127.0.0.1 -n 4 > nul"]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 1"]);
            command
        };
        let limit = Duration::from_millis(50);

        let error = run_with_timeout(command, limit).expect_err("child should time out");
        match error {
            NodeTimeoutError::TimedOut {
                elapsed,
                limit: actual,
            } => {
                assert_eq!(actual, limit);
                assert!(elapsed >= limit);
            }
            NodeTimeoutError::Io(error) => panic!("unexpected I/O error: {error}"),
        }
    }
    #[test]
    fn one_timed_out_node_does_not_block_an_independent_node() {
        use crate::budget::Budget;
        use crate::budget_dispatch::{run_to_completion_with_estimator, DispatchCost};
        use crate::dag_coordinator::{
            Coordinator, DagSpec, Dispatch, DomainVerdict, NodeExecutor, NodeOutcome, NodeSpec,
            RunOutcome,
        };

        struct MixedExecutor;

        impl NodeExecutor for MixedExecutor {
            fn execute(&self, dispatch: Dispatch) -> NodeOutcome {
                if dispatch.node_id == "a" {
                    let command = if cfg!(windows) {
                        let mut command = Command::new("cmd");
                        command.args(["/C", "ping 127.0.0.1 -n 4 > nul"]);
                        command
                    } else {
                        let mut command = Command::new("sh");
                        command.args(["-c", "sleep 1"]);
                        command
                    };
                    match run_with_timeout(command, Duration::from_millis(50)) {
                        Err(NodeTimeoutError::TimedOut { elapsed, limit }) => {
                            NodeOutcome::timeout(format!(
                                "node '{}' timed out; configured timeout limit: {}ms; actual elapsed: {}ms",
                                dispatch.node_id,
                                limit.as_millis(),
                                elapsed.as_millis(),
                            ))
                        }
                        Err(NodeTimeoutError::Io(error)) => {
                            NodeOutcome::process_failure(
                                crate::dag_coordinator::ExecutionFailure::Io,
                                error.to_string(),
                            )
                        }
                        Ok(_) => NodeOutcome::success(DomainVerdict::Pass, Vec::new()),
                    }
                } else {
                    NodeOutcome::success(DomainVerdict::Pass, Vec::new())
                }
            }
        }

        let coordinator = Coordinator::new(DagSpec::new(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            2,
            vec![
                NodeSpec::new("a", "fixture.timeout", "request:a"),
                NodeSpec::new("b", "fixture.pass", "request:b"),
            ],
            Vec::new(),
        ))
        .unwrap();
        let run = run_to_completion_with_estimator(
            coordinator,
            &MixedExecutor,
            Budget::new(10, 100, true),
            |_| DispatchCost::new(1, 1),
        )
        .unwrap();

        assert_eq!(run.manifest.outcome, RunOutcome::Timeout);
        assert_eq!(run.manifest_json["nodes"]["a"]["status"], "timeout");
        assert!(run.manifest_json["nodes"]["a"]["diagnostic"]
            .as_str()
            .is_some_and(|diagnostic| {
                diagnostic.contains("a")
                    && diagnostic.contains("configured timeout limit")
                    && diagnostic.contains("actual elapsed")
            }));
        assert_eq!(run.manifest_json["nodes"]["b"]["status"], "succeeded");
    }
}
