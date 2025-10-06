use anyhow::Result;
use enigo::{Enigo, Key, Keyboard, Settings};
use gdk::{gdk_pixbuf, gdk_pixbuf::Pixbuf};
use gtk::{cairo, glib, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Receiver,
};

use crate::system_menu::SystemTray;
use crate::frame_processor::ProcessedFrame;

use tokio::task;

#[derive(Clone, Debug)]
pub struct OverlayConfig {
    pub opacity: f64,
    pub width: i32,
    pub height: i32,
    pub x_position: i32,
    pub y_position: i32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            opacity: 0.8,
            width: 320,
            height: 240,
            x_position: 0,
            y_position: 0,
        }
    }
}

pub struct OverlayWindow {
    window: gtk::Window,
    image_widget: gtk::Picture,
    overlay_container: gtk::Overlay,
    text_labels_box: gtk::Box,
    icon_labels_box: gtk::Box,
    config: OverlayConfig,
}

#[derive(Clone)]
pub struct PixbufWrapper {
    pub bgr_buffer: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
}

impl PixbufWrapper {
    // Create an empty PixbufWrapper indicating to quit the processing thread
    pub(crate) fn quit() -> PixbufWrapper {
        PixbufWrapper {
            bgr_buffer: vec![],
            width: 0,
            height: 0,
            stride: 0,
        }
    }
}

impl PixbufWrapper {
    pub fn to_pixbuf(self) -> Pixbuf {
        Pixbuf::from_bytes(
            &glib::Bytes::from(&self.bgr_buffer),
            gdk_pixbuf::Colorspace::Rgb,
            true, // no alpha
            8,    // bits per sample
            self.width,
            self.height,
            self.stride,
        )
    }
}

impl OverlayWindow {
    pub fn new(config: OverlayConfig) -> Result<Self> {
        // Initialize GTK
        gtk::init()?;

        // Create the main window with configured size
        let window = gtk::Window::builder()
            .title("AOE4 Overlay")
            .default_width(config.width)
            .default_height(config.height)
            .decorated(false)
            .resizable(false)
            .build();

        window.set_modal(false);
        window.set_focusable(false);
        window.set_focus_visible(false);

        // Set up CSS for transparency and styling
        let css_provider = gtk::CssProvider::new();
        let css_content = format!(
            "window {{
                background-color: rgba(0, 0, 0, {});
            }}
            picture {{
                border: 2px solid white;
                border-radius: 5px;
            }}
            .stat-label {{
                background-color: rgba(0, 0, 0, 0.7);
                color: white;
                padding: 2px 5px;
                margin: 2px;
                font-family: monospace;
                font-size: 12px;
                border-radius: 3px;
            }}
            .icon-label {{
                background-color: rgba(0, 128, 0, 0.7);
                color: white;
                padding: 2px 5px;
                margin: 2px;
                font-weight: bold;
                font-size: 11px;
                border-radius: 3px;
            }}",
            config.opacity
        );
        css_provider.load_from_string(&css_content);

        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("Could not connect to display"),
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Create image widget for displaying screen capture
        let image_widget = gtk::Picture::new();
        image_widget.set_halign(gtk::Align::Center);
        image_widget.set_valign(gtk::Align::Center);
        image_widget.set_size_request(config.width, config.height);

        // Create overlay container
        let overlay_container = gtk::Overlay::new();
        overlay_container.set_child(Some(&image_widget));

        // Create vertical box for text labels (top-left)
        let text_labels_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text_labels_box.set_halign(gtk::Align::Start);
        text_labels_box.set_valign(gtk::Align::Start);
        text_labels_box.set_margin_start(5);
        text_labels_box.set_margin_top(5);
        overlay_container.add_overlay(&text_labels_box);

        // Create vertical box for icon labels (top-right)
        let icon_labels_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        icon_labels_box.set_halign(gtk::Align::End);
        icon_labels_box.set_valign(gtk::Align::Start);
        icon_labels_box.set_margin_end(5);
        icon_labels_box.set_margin_top(5);
        overlay_container.add_overlay(&icon_labels_box);

        // Add overlay container to window
        window.set_child(Some(&overlay_container));

        Ok(Self {
            window,
            image_widget,
            overlay_container,
            text_labels_box,
            icon_labels_box,
            config,
        })
    }

    pub fn show(&self) {
        self.window.present();

        // Set window position if specified (non-zero values)
        if self.config.x_position != 0 || self.config.y_position != 0 {
            // Note: On Wayland, window positioning is limited for security reasons
            // This may not work as expected on all compositors
            log::warn!("Window positioning on Wayland may be limited by the compositor");
        }

        // Make window input-transparent (non-clickable)
        if let Some(surface) = self.window.surface() {
            surface.set_input_region(&cairo::Region::create());
        } else {
            eprintln!("Warning: Could not get GDK surface for the window.");
        }

        // Send ALT+Space after a short delay
        glib::timeout_add_local_once(std::time::Duration::from_millis(500), || {
            std::thread::spawn(|| {
                if let Ok(mut enigo) = Enigo::new(&Settings::default()) {
                    // Send ALT+Space combination
                    let _ = enigo.key(Key::Alt, enigo::Direction::Press);
                    let _ = enigo.key(Key::Space, enigo::Direction::Press);
                    let _ = enigo.key(Key::Space, enigo::Direction::Release);
                    let _ = enigo.key(Key::Alt, enigo::Direction::Release);
                }
            });
        });
    }

    pub fn update_image_from_processed_frame(&self, frame: ProcessedFrame) {
        // Clear existing labels
        while let Some(child) = self.text_labels_box.first_child() {
            self.text_labels_box.remove(&child);
        }
        while let Some(child) = self.icon_labels_box.first_child() {
            self.icon_labels_box.remove(&child);
        }

        // Add text detection labels
        for detected_text in &frame.analysis.detected_texts {
            let label_text = format!(
                "{}: {} ({:.0}%)",
                detected_text.stat_name,
                detected_text.text,
                detected_text.confidence * 100.0
            );
            let label = gtk::Label::new(Some(&label_text));
            label.add_css_class("stat-label");
            label.set_xalign(0.0);
            self.text_labels_box.append(&label);
        }

        // Add icon detection labels
        for detected_icon in &frame.analysis.detected_icons {
            let label_text = format!(
                "{} ({:.0}%)",
                detected_icon.name,
                detected_icon.confidence * 100.0
            );
            let label = gtk::Label::new(Some(&label_text));
            label.add_css_class("icon-label");
            label.set_xalign(0.0);
            self.icon_labels_box.append(&label);
        }

        // Crop to region of interest (bottom 500px)
        let pixbuf = frame.original.to_pixbuf();
        let crop_height = pixbuf.height().min(500);
        let crop_width = pixbuf.width().min(300);
        let pixbuf = pixbuf.new_subpixbuf(
            0,
            pixbuf.height() - crop_height,
            crop_width,
            crop_height,
        );

        if let Some(scaled_pixbuf) = pixbuf.scale_simple(
            self.config.width,
            self.config.height,
            gdk_pixbuf::InterpType::Bilinear,
        ) {
            let texture = gdk::Texture::for_pixbuf(&scaled_pixbuf);
            self.image_widget.set_paintable(Some(&texture));
        }
    }
}

pub fn create_quit_signal() -> Arc<AtomicBool> {
    let should_quit: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    should_quit
}

pub async fn run_async_with_image_receiver(
    should_quit: Arc<AtomicBool>,
    gtk_receiver: Receiver<ProcessedFrame>,
    config: OverlayConfig,
) -> Result<()> {
    let should_quit_for_gtk = should_quit.clone();

    // Start the GTK thread
    let gtk_handle = std::thread::spawn(move || -> Result<()> {
        let window = OverlayWindow::new(config)?;

        // Initialize system tray icon with quit handler
        let _tray = SystemTray::new(should_quit_for_gtk.clone())?;

        // Create main loop
        let main_context = glib::MainContext::default();
        let main_loop = glib::MainLoop::new(Some(&main_context), false);

        // Set up a timeout to check for quit signal and process images
        let main_loop_quit = main_loop.clone();

        // Use Rc for single-threaded reference counting within GTK thread
        let window_rc = std::rc::Rc::new(window);
        let window_for_image_updates = std::rc::Rc::clone(&window_rc);

        glib::idle_add_local(move || {
            // Process any pending processed frames
            while let Ok(processed_frame) = gtk_receiver.try_recv() {
                window_for_image_updates.update_image_from_processed_frame(processed_frame);
            }
            glib::ControlFlow::Continue
        });

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            // Check for quit signal
            if should_quit_for_gtk.load(Ordering::Relaxed) {
                log::info!("Quit signal received, quitting window");
                main_loop_quit.quit();
                return glib::ControlFlow::Break;
            }

            glib::ControlFlow::Continue
        });

        // React to window close request
        let main_loop_quit_clone = main_loop.clone();
        window_rc.window.connect_close_request(move |_| {
            log::info!("Window close requested, quitting...");
            main_loop_quit_clone.quit();
            glib::signal::Propagation::Proceed
        });

        // Show the window
        window_rc.show();

        // Run the main loop
        main_loop.run();
        Ok(())
    });

    // Run async blocking until GTK thread completes
    let _ = task::spawn_blocking(move || {
        let _ = gtk_handle.join();
    })
    .await?;
    Ok(())
}
