#![feature(async_drop)]

use crate::{
    overlay_window_gtk::{OverlayConfig, PixbufWrapper, run_async_with_image_receiver},
    process_monitor::WaitForProcessResult,
};
use anyhow::Result;
use clap::Parser;
use log::{error, info};
use std::sync::{mpsc as std_mpsc};
use tokio::signal;

mod dbus_portal_screen_cast;
mod overlay_window_gtk;
mod pipewire_stream;
mod process_monitor;
mod utils;
mod wayland_record;

/// AOE4 Overlay - Screen capture and overlay for Age of Empires IV
#[derive(Parser, Debug)]
#[command(name = "aoe4_overlay")]
#[command(about = "Screen capture overlay for AoE4 on Wayland", long_about = None)]
struct Args {
    /// Capture mode: "monitor" for full screen, "window" for application window
    #[arg(short = 'm', long, default_value = "monitor", value_parser = ["monitor", "window"])]
    capture_mode: String,

    /// Opacity of the overlay window (0.0 - 1.0)
    #[arg(short = 'o', long, default_value = "0.8")]
    opacity: f64,

    /// Width of the overlay window in pixels
    #[arg(short = 'r', default_value = "320*240")]
    resolution: String,

    /// X position of the overlay window (0 = default position)
    #[arg(short = 'x', long, default_value = "0")]
    x_position: i32,

    /// Y position of the overlay window (0 = default position)
    #[arg(short = 'y', long, default_value = "0")]
    y_position: i32,

    /// Process name to monitor (if set, capture only starts when this process is running)
    #[arg(short = 'p', long)]
    process_name: Option<String>,

    /// Process check interval in milliseconds
    #[arg(short = 'i', long, default_value = "1000")]
    check_interval: u64,

    /// Show cursor in capture
    #[arg(short = 'c', long, default_value = "false")]
    show_cursor: bool,
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

    // Validate opacity range
    if args.opacity < 0.0 || args.opacity > 1.0 {
        anyhow::bail!("Opacity must be between 0.0 and 1.0");
    }

    // Parse resolution
    let (width, height) = args
        .resolution
        .split_once('*')
        .map(|(w_str, h_str)| {
            let w = w_str.parse::<i32>().unwrap_or(320);
            let h = h_str.parse::<i32>().unwrap_or(240);
            (w, h)
        })
        .unwrap_or_else(|| (320i32, 240i32));

    // Create overlay configuration
    let overlay_config = OverlayConfig {
        opacity: args.opacity,
        width,
        height,
        x_position: args.x_position,
        y_position: args.y_position,
    };

    info!(
        "Starting AOE4 Overlay with configuration: {:?}",
        overlay_config
    );
    info!("Capture mode: {}", args.capture_mode);

    // Create channel for image communication (using Vec<u8> for RGB data)
    let (gtk_sender, gtk_receiver) = std_mpsc::sync_channel::<PixbufWrapper>(2);

    // Start the Wayland recorder
    let mut wayland_recorder =
        wayland_record::WaylandRecorder::new("aoe4_screen").await?;

    // Determine record type based on capture mode
    let record_type = match args.capture_mode.as_str() {
        "window" => wayland_record::RecordTypes::Window,
        "monitor" => wayland_record::RecordTypes::Monitor,
        _ => wayland_record::RecordTypes::Monitor,
    };

    // Determine cursor mode
    let cursor_mode = if args.show_cursor {
        wayland_record::CursorModeTypes::Show
    } else {
        wayland_record::CursorModeTypes::Hidden
    };

    let should_quit = overlay_window_gtk::create_quit_signal();
    let should_quit_process_monitor = should_quit.clone();
    let should_quit_ctrl_c = should_quit.clone();

    let process_monitor = process_monitor::ProcessMonitor::new(
        args.process_name.unwrap_or_default(),
        args.check_interval,
    );

    if process_monitor.armed {
        tokio::select! {
            result = process_monitor.wait_for_process(should_quit_process_monitor) => {
                match result {
                    WaitForProcessResult::ProcessFound => {
                        if !wayland_recorder.start(record_type, cursor_mode, gtk_sender).await {
                            error!("Failed to start Wayland recorder");
                        }
                    }
                _ => {}}
            }
            result = signal::ctrl_c() => {
                match result {
                    Ok(()) => {
                        info!("Received Ctrl-C before process showed up. Shutting down gracefully...");
                        should_quit_ctrl_c.store(true, std::sync::atomic::Ordering::Relaxed);
                        return Ok(());
                    }
                    Err(err) => {
                        error!("Unable to listen for shutdown signal: {}", err);
                    }
                }
            }
        }
    } else {
        if !wayland_recorder.start(record_type, cursor_mode, gtk_sender).await {
            error!("Failed to start Wayland recorder");
        }
    }

    let should_quit_process_monitor = should_quit.clone();
    let should_quit_ctrl_c = should_quit.clone();
    let should_quit_process_quit = should_quit.clone();

    // Run both the overlay window and wait for shutdown signal concurrently
    tokio::select! {
        result = process_monitor.monitor_process_running(should_quit_process_monitor) => {
                match result {
                    WaitForProcessResult::ProcessNotFound => {
                        info!("Monitored process has exited, shutting down...");
                        should_quit_process_quit.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                _ => {}}
            }
        result = run_async_with_image_receiver(should_quit, gtk_receiver, overlay_config) => {
            match result {
                Ok(()) => {
                    info!("Overlay window closed, shutting down...");
                }
                Err(err) => {
                    error!("Overlay window error: {}", err);
                }
            }
        }
        result = signal::ctrl_c() => {
            match result {
                Ok(()) => {
                    info!("Received Ctrl-C, shutting down gracefully...");
                    should_quit_ctrl_c.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Err(err) => {
                    error!("Unable to listen for shutdown signal: {}", err);
                }
            }
        }
    }

    wayland_recorder.stop().await;
    info!("Done");

    Ok(())
}
