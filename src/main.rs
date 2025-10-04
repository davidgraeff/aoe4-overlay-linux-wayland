use std::sync::Arc;
use anyhow::{Result};
use log::{error, info};
use tokio::signal;
use crate::overlay_window_gtk::{run_async_with_image_receiver, PixbufWrapper};
use std::sync::mpsc as std_mpsc;

mod screen_cast;
mod wayland_record;
mod utils;
mod overlay_window_gtk;
mod pipewire_stream;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::builder()
        .filter(None, log::LevelFilter::Info)
        .filter(Some("aoe4_overlay"), log::LevelFilter::Debug)
        .init();

    if !utils::is_wayland() {
        anyhow::bail!("This program only works in a Wayland session.");
    }

    // Create channel for image communication (using Vec<u8> for RGB data)
    // let (image_sender, image_receiver) = mpsc::unbounded_channel();
    let (gtk_sender, gtk_receiver) = std_mpsc::sync_channel::<PixbufWrapper>(2);

    // Start the Wayland recorder
    let mut wayland_recorder = wayland_record::WaylandRecorder::new("aoe4_screen", gtk_sender).await?;

    wayland_recorder.start(
        wayland_record::RecordTypes::Monitor,
        wayland_record::CursorModeTypes::Hidden,
    ).await;

    info!("Screen recording started and overlay window displayed!");

    let should_quit = overlay_window_gtk::create_quit_signal();
    let should_quit_clone = Arc::clone(&should_quit);

    // Run both the overlay window and wait for shutdown signal concurrently
    tokio::select! {
        result = run_async_with_image_receiver(should_quit, gtk_receiver) => {
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
                    should_quit_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                Err(err) => {
                    error!("Unable to listen for shutdown signal: {}", err);
                }
            }
        }
    }

    wayland_recorder.stop().await;

    Ok(())
}
