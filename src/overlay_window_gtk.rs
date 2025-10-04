use anyhow::Result;
use enigo::{Enigo, Key, Keyboard, Settings};
use gdk::{gdk_pixbuf, gdk_pixbuf::Pixbuf};
use gtk::{cairo, glib, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Receiver,
};

pub struct OverlayWindow {
    window: gtk::Window,
    should_quit: Arc<AtomicBool>,
    image_widget: gtk::Picture,
}

pub struct PixbufWrapper {
    pub bgr_buffer: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
}

impl PixbufWrapper {
    pub fn to_pixbuf(mut self) -> Pixbuf {
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
    pub fn new(should_quit: Arc<AtomicBool>) -> Result<Self> {
        // Initialize GTK
        gtk::init()?;

        // Create the main window with size for scaled image (320x240)
        let window = gtk::Window::builder()
            .title("AOE4 Overlay")
            .default_width(320)
            .default_height(240)
            .decorated(false)
            .resizable(false)
            .build();

        window.set_modal(false);
        window.set_focusable(false);
        window.set_focus_visible(false);

        // Set up CSS for transparency and image styling
        let css_provider = gtk::CssProvider::new();
        css_provider.load_from_string(
            "window {
                background-color: rgba(0, 0, 0, 0.8);
            }
            picture {
                border: 2px solid white;
                border-radius: 5px;
            }",
        );

        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("Could not connect to display"),
            &css_provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // Create image widget for displaying screen capture
        let image_widget = gtk::Picture::new();
        image_widget.set_halign(gtk::Align::Center);
        image_widget.set_valign(gtk::Align::Center);
        image_widget.set_size_request(320, 240);

        // Add image widget to window
        window.set_child(Some(&image_widget));

        // Handle window close event
        let should_quit_clone = Arc::clone(&should_quit);
        window.connect_close_request(move |_| {
            should_quit_clone.store(true, Ordering::Relaxed);
            glib::Propagation::Proceed
        });

        Ok(Self {
            window,
            should_quit,
            image_widget,
        })
    }

    pub fn show(&self) {
        self.window.present();
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

    pub fn close(&self) {
        self.should_quit.store(true, Ordering::Relaxed);
        self.window.close();
    }

    pub fn update_image_from_data(&self, pixbuf: PixbufWrapper) {
        // Scale the image to 320x240
        if let Some(scaled_pixbuf) =
            pixbuf
                .to_pixbuf()
                .scale_simple(320, 240, gdk_pixbuf::InterpType::Bilinear)
        {
            // Create a texture from the scaled pixbuf
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
    gtk_receiver: Receiver<PixbufWrapper>,
) -> Result<()> {
    let window_should_quit = Arc::clone(&should_quit);
    // let receiver_should_quit = Arc::clone(&should_quit);

    // Start the GTK thread
    let gtk_handle = std::thread::spawn(move || -> Result<()> {
        let window = OverlayWindow::new(window_should_quit.clone())?;
        let should_quit_for_timeout = Arc::clone(&window_should_quit);

        // Create main loop
        let main_context = glib::MainContext::default();
        let main_loop = glib::MainLoop::new(Some(&main_context), false);

        // Set up a timeout to check for quit signal and process images
        let main_loop_quit = main_loop.clone();

        // Use Rc for single-threaded reference counting within GTK thread
        let window_rc = std::rc::Rc::new(window);
        let window_for_timeout = std::rc::Rc::clone(&window_rc);

        glib::idle_add_local(move || {
            // Process any pending images
            while let Ok(pixbuf) = gtk_receiver.try_recv() {
                window_for_timeout.update_image_from_data(pixbuf);
            }
            glib::ControlFlow::Continue
        });

        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            // Check for quit signal
            if should_quit_for_timeout.load(Ordering::Relaxed) {
                main_loop_quit.quit();
                return glib::ControlFlow::Break;
            }

            glib::ControlFlow::Continue
        });

        // Show the window
        window_rc.show();

        // Run the main loop
        main_loop.run();
        Ok(())
    });

    // Wait for either the image handler to finish or quit signal
    tokio::select! {
        _ = async {
            while !should_quit.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        } => {},
    }

    // Wait for GTK thread to finish
    let _ = gtk_handle.join();

    Ok(())
}
