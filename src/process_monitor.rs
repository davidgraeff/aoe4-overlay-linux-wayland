use log::{debug};
use std::{time::Duration};
use tokio::time::timeout;

/// Monitor for checking if a specific process is running
pub struct ProcessMonitor {
    pub(crate) process_name: String,
    pub(crate) check_interval: Duration,
    pub(crate) armed: bool,
    pub receiver: tokio::sync::oneshot::Receiver<()>,
}

pub enum WaitForProcessResult {
    ProcessFound,
    ProcessNotFound,
    Terminated
}

#[derive(PartialEq)]
pub enum WaitForProcessTask {
    WaitForProcess,
    WaitForProcessEnd,
}

impl ProcessMonitor {
    pub fn new(process_name: String, check_interval_ms: u64) -> (Self, tokio::sync::oneshot::Sender<()>) {
        // Create tokio once channel for quiting the process monitor task
        let (sender,receiver) = tokio::sync::oneshot::channel::<()>();

        (Self {
            armed: !process_name.is_empty(),
            process_name,
            check_interval: Duration::from_millis(check_interval_ms),
            receiver
        }, sender)
    }

    /// Check if the target process is currently running
    pub fn is_process_running(&self) -> bool {
        if !self.armed {
            return false;
        }
        match procfs::process::all_processes() {
            Ok(processes) => {
                for process_result in processes {
                    if let Ok(process) = process_result {
                        if let Ok(stat) = process.stat() {
                            // Check both comm (command name) and cmdline (full command line)
                            if stat.comm.contains(&self.process_name) {
                                //debug!("Found process: {} (pid: {})", stat.comm, process.pid);
                                return true;
                            }
                        }
                    }
                }
                false
            }
            Err(e) => {
                debug!("Error reading processes: {}", e);
                false
            }
        }
    }

    pub async fn act_on_process(
        &mut self,
        task: WaitForProcessTask
    ) -> WaitForProcessResult {
        if !self.armed {
            return if task == WaitForProcessTask::WaitForProcess {
                WaitForProcessResult::ProcessFound
            } else {
                WaitForProcessResult::ProcessNotFound
            }
        }

        loop {
            if task == WaitForProcessTask::WaitForProcess {
                if self.is_process_running() {
                    return WaitForProcessResult::ProcessFound;
                }
            } else {
                if !self.is_process_running() {
                    return WaitForProcessResult::ProcessNotFound;
                }
            }
            if let Ok(_) = timeout(self.check_interval, &mut self.receiver).await {
                self.armed = false;
                break;
            }
        }
        WaitForProcessResult::Terminated
    }
}
