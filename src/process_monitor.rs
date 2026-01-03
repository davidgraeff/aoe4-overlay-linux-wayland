use log::{debug};
use std::{time::Duration};
use tokio::sync::mpsc::Sender;
// use tokio::sync::oneshot::Receiver;
use tokio::time::timeout;
use crate::events::ControlEvent;

/// Monitor for checking if a specific process is running
pub struct ProcessMonitor {
    pub(crate) process_name: String,
    pub(crate) check_interval: Duration,
    pub armed: bool
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
        is_process_running(&self.process_name)
    }

    pub fn notify_control_channel(&self) -> (tokio::sync::oneshot::Sender<()>, tokio::sync::oneshot::Receiver<()>) {
        tokio::sync::oneshot::channel::<()>()
    }

    pub async fn notify_on_change(
        &mut self,
        mut stopper: tokio::sync::oneshot::Receiver<()>,
        control_sender: Sender<ControlEvent>
    )  {
        if !self.armed {
            return;
        }

        let process_running = self.is_process_running();
        let process_name = self.process_name.clone();
        let check_interval = self.check_interval;

        control_sender.send(ControlEvent::ProcessStatusChanged(process_running)).await.ok();

        tokio::spawn(async move {
            loop {
                let currently_running = is_process_running(&process_name);
                if currently_running != process_running {
                    log::info!("Process {} running status changed: {}", &process_name, process_running);
                    control_sender.send(ControlEvent::ProcessStatusChanged(process_running)).await.ok();
                    break;
                }
                if let Ok(v) = timeout(check_interval, &mut stopper).await {
                    log::info!("Process monitor: Stopping monitoring for process {}. {:?}", &process_name, v);
                    break;
                }
            }
        });
    }
}


/// Check if the target process is currently running
pub fn is_process_running(process_name: &str) -> bool {
    match procfs::process::all_processes() {
        Ok(processes) => {
            for process_result in processes {
                if let Ok(process) = process_result {
                    if let Ok(stat) = process.stat() {
                        // Check both comm (command name) and cmdline (full command line)
                        if stat.comm.contains(&process_name) {
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
