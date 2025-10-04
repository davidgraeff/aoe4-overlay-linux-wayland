use log::{debug, info};
use std::{sync::Arc, time::Duration};

/// Monitor for checking if a specific process is running
pub struct ProcessMonitor {
    pub(crate) process_name: String,
    pub(crate) check_interval: Duration,
    pub(crate) armed: bool,
}

pub enum WaitForProcessResult {
    ProcessFound,
    ProcessNotFound,
    QuitSignalReceived,
}

impl ProcessMonitor {
    pub fn new(process_name: String, check_interval_ms: u64) -> Self {
        Self {
            armed: !process_name.is_empty(),
            process_name,
            check_interval: Duration::from_millis(check_interval_ms),
        }
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
                                debug!("Found process: {} (pid: {})", stat.comm, process.pid);
                                return true;
                            }

                            // Also check cmdline for full path matches
                            if let Ok(cmdline) = process.cmdline() {
                                if cmdline.iter().any(|arg| arg.contains(&self.process_name)) {
                                    debug!(
                                        "Found process in cmdline: {:?} (pid: {})",
                                        cmdline, process.pid
                                    );
                                    return true;
                                }
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

    /// Wait until the target process is running
    pub async fn wait_for_process(
        &self,
        should_quit: Arc<std::sync::atomic::AtomicBool>,
    ) -> WaitForProcessResult {
        if !self.armed {
            return WaitForProcessResult::ProcessNotFound;
        }

        info!(
            "Waiting for process '{}' to start (checking every {:?})...",
            self.process_name, self.check_interval
        );

        loop {
            if self.is_process_running() {
                info!(
                    "Process '{}' detected, starting capture...",
                    self.process_name
                );
                return WaitForProcessResult::ProcessFound;
            }
            tokio::time::sleep(self.check_interval).await;

            if should_quit.load(std::sync::atomic::Ordering::Relaxed) {
                return WaitForProcessResult::QuitSignalReceived;
            }
        }
    }

    /// Continuously monitor if the process is still running
    pub async fn monitor_process_running(
        &self,
        should_quit: Arc<std::sync::atomic::AtomicBool>,
    ) -> WaitForProcessResult {
        if !self.armed {
            tokio::time::sleep(Duration::from_secs(30 * 3600 * 24)).await;
            return WaitForProcessResult::ProcessFound;
        }
        loop {
            if !self.is_process_running() {
                info!(
                    "Process '{}' is no longer running, stopping capture...",
                    self.process_name
                );
                return WaitForProcessResult::ProcessNotFound;
            }
            tokio::time::sleep(self.check_interval).await;

            if should_quit.load(std::sync::atomic::Ordering::Relaxed) {
                return WaitForProcessResult::QuitSignalReceived;
            }
        }
    }
}
