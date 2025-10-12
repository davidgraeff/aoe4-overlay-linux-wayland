#![allow(incomplete_features)]
#![feature(async_drop)]
#![feature(str_as_str)]
#![feature(stmt_expr_attributes)]

use crate::{
    overlay_window_gtk::{OverlayConfig},
    process_monitor::WaitForProcessResult,
    system_tray::{Base, Menu},
};
use anyhow::{anyhow, Result};
use clap::Parser;
use libappindicator_zbus::{tray, utils::Category};
use log::{error, info};
use std::sync::mpsc as std_mpsc;
use tokio::{signal, task};

mod dbus_portal_screen_cast;
mod frame_processor;
mod image_analyzer;
pub mod ocr;
mod overlay_window_gtk;
mod pipewire_stream;
mod pixelbuf_wrapper;
mod process_monitor;
mod system_menu;
mod system_tray;
mod utils;
mod wayland_record;

use crate::{
    overlay_window_gtk::GuiCommand,
    pixelbuf_wrapper::PixelBufWrapperWithDroppedFramesTS,
};
pub use aoe4_overlay::consts;

/// AOE4 Overlay - Screen capture and overlay for Age of Empires IV
#[derive(Parser, Debug)]
#[command(name = "aoe4_overlay")]
#[command(about = "Screen capture overlay for AoE4 on Wayland", long_about = None)]
struct Args {
    /// Capture mode: "monitor" for full screen, "window" for application window
    #[arg(short = 'm', long, default_value = "window", value_parser = ["monitor", "window"])]
    capture_mode: String,

    /// No debug window, only show overlay
    #[arg(short = 'd', long, default_value_t = false)]
    debug_window: bool,

    /// Process name to monitor (if set, capture only starts when this process is running)
    #[arg(short = 'p', long, default_value = "RelicCardinal.")]
    process_name: Option<String>,

    /// Process check interval in milliseconds
    #[arg(short = 'i', long, default_value = "3000")]
    check_interval: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::builder()
        .filter(None, log::LevelFilter::Info)
        .filter(Some("aoe4_overlay"), log::LevelFilter::Debug)
        .init();

    let args = Args::parse();

    if !utils::is_wayland() {
        anyhow::bail!("This program only works in a Wayland session.");
    }

    // Determine record type based on capture mode
    let record_type = match args.capture_mode.as_str() {
        "window" => wayland_record::RecordTypes::Window,
        "monitor" => wayland_record::RecordTypes::Monitor,
        _ => wayland_record::RecordTypes::Monitor,
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


    let _connection = tray(
        Base::boot,
        "com.aoe4.overlay.tray",
        "Age of Empires IV Overlay",
        Menu::boot,
        Menu::menu,
        1,
    )
        .with_icon_pixmap(Base::icon_pixmap)
        .with_item_is_menu(false)
        .with_category(Category::ApplicationStatus)
        .with_menu_status(Menu::status)
        .with_on_clicked(Menu::on_clicked)
        .run()
        .await?;


    // Start frame processor
    info!("Initializing frame processor...");
    let frame_processor = match frame_processor::FrameProcessor::new() {
        Ok(processor) => processor,
        Err(e) => {
            error!("Failed to initialize frame processor: {}", e);
            anyhow::bail!("Frame processor initialization failed: {}", e);
        }
    };

    // Create std_mpsc channel for GTK (since GTK needs to run in its own thread)
    let (gtk_sender, gtk_receiver) = tokio::sync::mpsc::channel::<GuiCommand>(2);
    let (pipewire_sender, pipewire_receiver) = std_mpsc::sync_channel::<bool>(1);

    let pixelbuf_content = PixelBufWrapperWithDroppedFramesTS::default();
    let pixelbuf_content_clone = pixelbuf_content.clone();

    let gtk_sender_clone = gtk_sender.clone();

    // Run image processing in a separate thread. Quit by sending an empty frame.
    let gtk_sender = gtk_sender_clone.clone();
    let processor_join_handle = tokio::spawn(async move {
        let gtk_sender_clone = gtk_sender.clone();
        let _ = task::spawn_blocking(move || {
            let handler = std::thread::spawn(move || {
                let _ = frame_processor.run(pipewire_receiver, pixelbuf_content, gtk_sender_clone);
            });
            let _ = handler.join().map_err(|_| anyhow!("Failed to join frame_processor thread"));
        })
            .await;
        let _ = gtk_sender.try_send(GuiCommand::Quit);
    });

    let (mut process_monitor, process_monitor_quitter) = process_monitor::ProcessMonitor::new(
        args.process_name.unwrap_or_default(),
        args.check_interval,
    );

    // Start the Wayland recorder
    let mut wayland_recorder = wayland_record::WaylandRecorder::new("aoe4_screen2").await?;

    // Start PipeWire stream
    let (pipewire_control_handler, pipewire_join_handler) =
        pipewire_stream::run(pipewire_sender, pixelbuf_content_clone);

    let gtk_sender = gtk_sender_clone.clone();
    let pipewire_join_handler = tokio::spawn(async move {
        let _ = task::spawn_blocking(move || {
            let _ = pipewire_join_handler.join().map_err(|_| anyhow!("Failed to join pipewire thread"));
        })
            .await;
        let _ = gtk_sender.try_send(GuiCommand::Quit);
    });

    let enable_waiting = process_monitor.armed;

    let gtk_sender = gtk_sender_clone.clone();
    tokio::spawn(async move {
        match signal::ctrl_c().await {
            Ok(()) => {
                info!("Received Ctrl-C, shutting down gracefully...");
                let _ = gtk_sender.try_send(GuiCommand::Quit);
            }
            Err(err) => {
                error!("Unable to listen for shutdown signal: {}", err);
            }
        }
    });

    let wayland_stop_handler = wayland_recorder.get_stop_handler();

    let gtk_sender = gtk_sender_clone.clone();
    let pipewire_sender_frames = pipewire_control_handler.get_frame_sender();
    let process_monitor_handler = tokio::spawn(async move {
        if process_monitor.armed {
            info!("Waiting for process {}", process_monitor.process_name);
        }
        if let WaitForProcessResult::ProcessFound = process_monitor
            .act_on_process(process_monitor::WaitForProcessTask::WaitForProcess)
            .await
        {
            let _ = gtk_sender.try_send(GuiCommand::AboutToProcessFrames);
            if let Err(e) = wayland_recorder
                .run(
                    record_type,
                    wayland_record::CursorModeTypes::Hidden,
                    pipewire_sender_frames,
                )
                .await
            {
                let _ = gtk_sender.try_send(GuiCommand::Quit);
                error!("Failed to start Wayland recorder: {}", e);
            }
        }

        if process_monitor.armed {
            if let WaitForProcessResult::ProcessNotFound = process_monitor
                .act_on_process(process_monitor::WaitForProcessTask::WaitForProcessEnd)
                .await
            {
                info!("Monitored process ended, shutting down...");
                let _ = gtk_sender.try_send(GuiCommand::Quit);
            }
        }
    });

    match overlay_window_gtk::run(
        gtk_sender_clone,
        gtk_receiver,
        overlay_config,
        enable_waiting,
    )
        .await
    {
        Ok(()) => {
            info!("Overlay window closed, shutting down...");
        }
        Err(err) => {
            error!("Overlay window error: {}", err);
        }
    }

    let _ = process_monitor_quitter.send(());
    pipewire_control_handler.stop();
    pipewire_join_handler.await.map_err(|_| anyhow!("Failed to join pipewire thread"))?;
    wayland_stop_handler.stop().await;
    let _ = process_monitor_handler.await?;
    let _ = processor_join_handle.await;
    Ok(())
}
