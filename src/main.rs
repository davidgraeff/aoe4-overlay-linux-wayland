#![allow(incomplete_features)]
#![feature(async_drop)]
#![feature(str_as_str)]
#![feature(stmt_expr_attributes)]

use anyhow::Result;
use aoe4_overlay::{
    events::{ControlEvent, create_control_event_channel},
    frame_processor, pipewire_stream,
    pipewire_stream::PipeWireStopHandler,
    pixelbuf_wrapper::PixelBufWrapperWithDroppedFramesTS,
    process_monitor,
    process_monitor::ProcessMonitor,
    ui,
    ui::{GuiCommand, OverlayConfig},
    utils, wayland_record,
    wayland_record::SourceType,
};
use ashpd::enumflags2::BitFlags;
use clap::Parser;
use gio::prelude::ApplicationExtManual;
use log::{error, info};
use std::sync::mpsc as std_mpsc;
use tokio::{
    signal,
    sync::mpsc::{Receiver, Sender},
};

/// AOE4 Overlay - Screen capture and overlay for Age of Empires IV
#[derive(Parser, Debug)]
#[command(name = "aoe4_overlay")]
#[command(about = "Screen capture overlay for AoE4 on Wayland", long_about = None)]
struct Args {
    /// Capture mode: "monitor" for full screen, "window" for application window
    #[arg(short = 'm', long, default_value = "window", value_parser = ["monitor", "window"])]
    capture_mode: String,

    /// No debug window, only show overlay
    #[arg(short = 'd', long, default_value_t = true)]
    debug_window: bool,

    /// Process name to monitor (if set, capture only starts when this process is running)
    #[arg(short = 'p', long, default_value = "RelicCardinal.")]
    process_name: Option<String>,

    /// Process check interval in milliseconds
    #[arg(short = 'i', long, default_value = "3000")]
    check_interval: u64,
}

fn main() -> Result<()> {
    env_logger::builder()
        .filter(None, log::LevelFilter::Info)
        .filter(Some("aoe4_overlay"), log::LevelFilter::Debug)
        .init();

    let args = Args::parse();

    if !utils::is_wayland() {
        anyhow::bail!("This program only works in a Wayland session.");
    }

    // Determine record type based on capture mode
    let record_type: BitFlags<SourceType> = match args.capture_mode.as_str() {
        "window" => BitFlags::from(SourceType::Window),
        "monitor" => BitFlags::from(SourceType::Monitor),
        _ => SourceType::Monitor | SourceType::Window,
    };

    // Create overlay configuration
    let overlay_config = OverlayConfig {
        show_debug_window: args.debug_window,
    };

    info!(
        "Starting AOE4 Overlay with configuration: {:?}",
        overlay_config
    );
    info!("Capture mode: {}", args.capture_mode);

    // Start frame processor
    info!("Initializing frame processor...");
    let frame_processor = match frame_processor::FrameProcessor::new() {
        Ok(processor) => processor,
        Err(e) => {
            anyhow::bail!("Frame processor initialization failed: {}", e);
        }
    };

    let process_monitor =
        ProcessMonitor::new(args.process_name.unwrap_or_default(), args.check_interval);

    // Create std_mpsc channel for GTK (since GTK needs to run in its own thread)
    let (gtk_sender, gtk_receiver) = tokio::sync::mpsc::channel::<GuiCommand>(2);
    let (pipewire_sender, pipewire_receiver) = std_mpsc::sync_channel::<bool>(1);

    let pixelbuf_content = PixelBufWrapperWithDroppedFramesTS::default();
    let pixelbuf_content_clone = pixelbuf_content.clone();

    let gtk_sender_clone = gtk_sender.clone();
    let wait_for_process = process_monitor.armed;

    // Run image processing in a separate thread. Quit by sending an empty frame.
    let frame_processor_handler = std::thread::spawn(move || {
        let gtk_sender = gtk_sender_clone.clone();
        let _ = frame_processor.run(pipewire_receiver, pixelbuf_content, gtk_sender_clone);
        info!("Frame processor thread exiting");
        let _ = gtk_sender.try_send(GuiCommand::Quit);
    });

    // Start PipeWire stream
    let gtk_sender_clone = gtk_sender.clone();
    let (receiver, pipewire_control_handler) = pipewire_stream::PipeWireStream::new_communication();

    let pipewire_sender_clone = pipewire_sender.clone();
    let pipewire_handler: std::thread::JoinHandle<Result<()>> = std::thread::spawn(move || {
        let pipewire_stream =
            pipewire_stream::PipeWireStream::new(pipewire_sender_clone, pixelbuf_content_clone)?;

        let pipewire_stream_clone = pipewire_stream.clone();
        let _receiver_handler = receiver.attach(pipewire_stream.mainloop.loop_(), move |m| {
            pipewire_stream_clone.handle(m)
        });

        pipewire_stream.mainloop.run();
        info!("Pipewire thread exiting");
        let _ = gtk_sender_clone.try_send(GuiCommand::Quit);
        Ok(())
    });

    let (control_sender, control_receiver) = create_control_event_channel();
    let control_sender_ui = control_sender.clone();

    let gtk_sender_clone = gtk_sender.clone();
    let pipewire_control_handler_clone = pipewire_control_handler.clone();
    let tokio_handler: std::thread::JoinHandle<Result<()>> = std::thread::spawn(move || {
        Ok(tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(tokio_thread(
                gtk_sender_clone,
                process_monitor,
                record_type,
                pipewire_control_handler_clone,
                control_sender,
                control_receiver,
            ))?)
    });

    let gui_handler = std::thread::spawn(move || {
        let application = ui::create(control_sender_ui.clone(), gtk_receiver, overlay_config, wait_for_process);
        let application = match application {
            Ok(application) => application,
            Err(err) => {
                error!("Overlay window error: {}", err);
                return Err(anyhow::anyhow!(err));
            }
        };
        application.run();
        let _ = control_sender_ui.blocking_send(ControlEvent::Quit);
        info!("GUI thread exiting");
        Ok(())
    });

    // Wait for UI thread to finish
    let _ = gui_handler.join();

    pipewire_control_handler.stop();
    let _ = pipewire_sender.send(false);

    let _ = tokio_handler.join();
    let _ = frame_processor_handler.join();
    let _ = pipewire_handler.join();

    Ok(())
}

async fn tokio_thread(
    gtk_sender: Sender<GuiCommand>,
    mut process_monitor: ProcessMonitor,
    record_type: BitFlags<SourceType>,
    pipewire_control_handler: PipeWireStopHandler,
    control_sender: Sender<ControlEvent>,
    control_receiver: Receiver<ControlEvent>,
) -> Result<()> {
    // Start the Wayland recorder
    let (mut wayland_recorder, wayland_stop_handler) =
        wayland_record::WaylandRecorder::new().await?; // "aoe4_screen2"

    let gtk_sender_clone = gtk_sender.clone();
    let control_sender_clone = control_sender.clone();
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("Received Ctrl-C, shutting down gracefully...");
                let _ = gtk_sender_clone.try_send(GuiCommand::Quit);
                let _ = control_sender_clone.send(ControlEvent::Quit).await;
            }
            Err(err) => {
                error!("Unable to listen for shutdown signal: {}", err);
            }
        }
    });

    let pipewire_sender_frames = pipewire_control_handler.get_frame_sender();

    // TODO: This should be controlled from the UI
    process_monitor.armed = false;
    if process_monitor.armed {
        let _ = control_sender
            .send(ControlEvent::StartCaptureWaitForProcess)
            .await;
    } else {
        info!("Process monitoring is not armed, starting capture immediately");
        let _ = control_sender.send(ControlEvent::StartCapture).await;
    }

    fn stop_process_monitoring(
        process_monitor_quit_sender: &mut Option<tokio::sync::oneshot::Sender<()>>,
    ) {
        if let Some(quit_sender) = process_monitor_quit_sender.take() {
            info!("Process monitor: Quitting any remaining listeners");
            let _ = quit_sender.send(());
        }
    }

    // Handle control events
    let mut control_receiver = control_receiver;
    while let Some(event) = control_receiver.recv().await {
        let mut process_monitor_quit_sender: Option<tokio::sync::oneshot::Sender<()>> = None;
        match event {
            ControlEvent::Quit => {
                stop_process_monitoring(&mut process_monitor_quit_sender);
                info!("Control event: Quit");
                break;
            }
            ControlEvent::StartCapture => {
                stop_process_monitoring(&mut process_monitor_quit_sender);
                if let Err(e) = wayland_recorder
                    .start(record_type, pipewire_sender_frames.clone())
                    .await
                {
                    error!("Failed to start Wayland recorder: {}", e);
                } else {
                    let _ = gtk_sender.send(GuiCommand::StateCaptureStarted).await;
                }
            }
            ControlEvent::StopCapture => {
                stop_process_monitoring(&mut process_monitor_quit_sender);
                if let Err(e) = wayland_recorder.stop().await {
                    error!("Failed to start Wayland recorder: {}", e);
                } else {
                    let _ = gtk_sender.send(GuiCommand::StateCaptureStopped).await;
                }
            }
            ControlEvent::ProcessStatusChanged(running) => {
                if !process_monitor.armed {
                    info!("Process monitoring is disarmed, ignoring status change");
                    continue;
                }
                let _ = gtk_sender.send(GuiCommand::ProcessRunning(running)).await;

                if running && !wayland_recorder.is_running() {
                    info!("Process is running, starting capture");
                    let _ = control_sender.send(ControlEvent::StartCapture).await;
                } else if wayland_recorder.is_running() {
                    info!("Process is not running, stopping capture");
                    let _ = control_sender.send(ControlEvent::StopCapture).await;
                }
            }
            ControlEvent::StartCaptureWaitForProcess => {
                info!("Control event: StartWaitForProcess");
                stop_process_monitoring(&mut process_monitor_quit_sender);
                let control_sender_clone = control_sender.clone();
                let (quit_sender, quit_receiver) = process_monitor.notify_control_channel();
                process_monitor_quit_sender = Some(quit_sender);
                process_monitor
                    .notify_on_change(quit_receiver, control_sender_clone)
                    .await;
            }
        }
    }

    wayland_stop_handler.stop().await;
    pipewire_control_handler.stop();

    let _ = gtk_sender.try_send(GuiCommand::Quit);
    info!("Tokio thread exiting");
    Ok(())
}
