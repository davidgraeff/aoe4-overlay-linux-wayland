pub enum ControlEvent {
    Quit,
    ProcessStatusChanged(bool),
    StartCapture,
    StartCaptureWaitForProcess,
    StopCapture,
}

pub fn create_control_event_channel() -> (
    tokio::sync::mpsc::Sender<ControlEvent>,
    tokio::sync::mpsc::Receiver<ControlEvent>,
) {
    tokio::sync::mpsc::channel(100)
}