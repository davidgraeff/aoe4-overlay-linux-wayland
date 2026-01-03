use crate::{
    consts::{AOE4_STATS_POS, AREA_HEIGHT, AREA_WIDTH, INDEX_IDLE, INDEX_POP},
    events::ControlEvent,
    frame_processor::ProcessedFrame,
};
use anyhow::Result;
use gtk::{Application, Button, IconTheme, Label, cairo, glib, prelude::*, gdk_pixbuf};
use log::{error, info};
use std::cell::{Cell, RefCell};
use tokio::sync::mpsc::{Receiver, Sender};
use crate::utils::data_directory;

#[derive(Clone, Debug)]
pub struct OverlayConfig {
    pub show_debug_window: bool,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            show_debug_window: true,
        }
    }
}

pub enum GuiCommand {
    ProcessedFrame(ProcessedFrame),
    Quit,
    StateCaptureStarted,
    StateCaptureStopped,
    ProcessRunning(bool),
}

pub struct OverlayWindow {
    window: gtk::ApplicationWindow,
    _overlay_container: gtk::Overlay,
    _text_labels_box: gtk::Box,
    _icon_labels_box: gtk::Box,
    config: OverlayConfig,
    pub centered_label: Label,
    pub labels: [Label; AOE4_STATS_POS.len()],
}

pub struct InteractWindow {
    window: gtk::Window,
    image_widget: gtk::Picture,
    _quit_button: Button,
    pub status_label: Label,
    pub capturing: Cell<bool>,
    pub process_running: Cell<bool>,
    pub wait_for_process: bool,
    pub capture_button: Button,
}

impl InteractWindow {
    pub fn new(
        sender: Sender<ControlEvent>,
        app: &Application,
        wait_for_process: bool,
    ) -> Result<Self> {
        let window = gtk::Window::builder()
            .title("AOE4 Overlay Interaction")
            .maximized(false)
            .decorated(true)
            .resizable(false)
            .focusable(true)
            .focus_visible(true)
            .modal(false)
            .application(app)
            .build();

        // React to window close request
        let app_clone = app.clone();
        window.connect_close_request(move |_| {
            log::info!("Window close requested, quitting...");
            app_clone.quit();
            glib::signal::Propagation::Proceed
        });

        // Add vbox for layout
        let vbox = gtk::Box::new(gtk::Orientation::Vertical, 5);
        vbox.set_margin_top(10);
        vbox.set_margin_bottom(10);
        vbox.set_margin_start(10);
        vbox.set_margin_end(10);
        window.set_child(Some(&vbox));

        // Add hbox for buttons
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        hbox.set_margin_top(10);
        hbox.set_margin_bottom(10);
        hbox.set_margin_start(10);
        hbox.set_margin_end(10);
        vbox.append(&hbox);

        // Create quit button
        let quit_button = Button::with_label("Quit");
        quit_button.set_halign(gtk::Align::Start);
        quit_button.set_valign(gtk::Align::Start);
        quit_button.set_child_visible(true);
        let app_clone = app.clone();
        quit_button.connect_clicked(move |_| {
            app_clone.quit();
        });
        hbox.append(&quit_button);

        // Create button to start/stop capture
        let capture_button = Button::with_label("Start Capture");
        capture_button.set_halign(gtk::Align::Start);
        capture_button.set_valign(gtk::Align::Start);
        let sender_clone = sender.clone();
        let capturing = Cell::new(false);
        let capturing_clone = capturing.clone();
        capture_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            if capturing_clone.get() {
                button.set_label("Stopping Capture...");
                let _ = sender_clone.try_send(ControlEvent::StopCapture);
            } else {
                if wait_for_process {
                    button.set_label("Starting Capture (Wait for process)...");
                    let _ = sender_clone.try_send(ControlEvent::StartCaptureWaitForProcess);
                } else {
                    button.set_label("Starting Capture...");
                    let _ = sender_clone.try_send(ControlEvent::StartCapture);
                }
            }
        });
        hbox.append(&capture_button);

        // Create status label
        let status_label = Label::new(Some("Process Status: Unknown"));
        status_label.set_halign(gtk::Align::Start);
        status_label.set_valign(gtk::Align::Start);
        vbox.append(&status_label);

        // Create image widget for displaying screen capture
        let image_widget = gtk::Picture::new();
        image_widget.set_halign(gtk::Align::End);
        image_widget.set_valign(gtk::Align::Start);
        image_widget.set_size_request(AREA_WIDTH, AREA_HEIGHT);
        image_widget.set_child_visible(true);
        vbox.append(&image_widget);


        Ok(Self {
            window,
            image_widget,
            status_label,
            wait_for_process,
            capturing,
            capture_button,
            process_running: Cell::new(false),
            _quit_button: quit_button,
        })
    }

    fn update_status_label(&self) {
        if !self.wait_for_process || self.process_running.get() {
            if self.capturing.get() {
                self.status_label
                    .set_text("Process Status: Running | Capturing");
            } else {
                self.status_label
                    .set_text("Process Status: Running | Not Capturing");
            }
        } else {
            if self.capturing.get() {
                self.status_label
                    .set_text("Process Status: Not Running | Capturing");
            } else {
                self.status_label
                    .set_text("Process Status: Not Running | Not Capturing");
            }
        }
    }

    pub fn update_process_running_state(&self, running: bool) {
        self.process_running.set(running);
        self.update_status_label();
    }

    pub fn update_capture_state(&self, capturing: bool) {
        self.capture_button.set_sensitive(true);
        if capturing {
            self.capture_button.set_label("Stop Capture");
        } else {
            self.capture_button.set_label("Start Capture");
        }
        self.capturing.set(capturing);
        self.update_status_label();
    }

    pub fn show(&self) {
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }


    pub fn update_image_from_processed_frame(&self, frame: ProcessedFrame) {
        // Crop to region of interest (bottom 500px)
        let pixbuf = frame.original.to_pixbuf();
        let crop_height = pixbuf.height().min(500);
        let crop_width = pixbuf.width().min(300);
        let y = (pixbuf.height() - crop_height).max(0);
        //info!("pixbuf size: {}x{}, crop to {}x{} at y={}", pixbuf.width(), pixbuf.height(), crop_width, crop_height, y);
        let pixbuf_sub =
            pixbuf.new_subpixbuf(0, y, crop_width, crop_height); // pixbuf.height() - crop_height

        let texture = gdk::Texture::for_pixbuf(&pixbuf_sub);
        self.image_widget.set_paintable(Some(&texture));
        //self.image_widget.set_visible(true);
        //info!("update_image_from_processed_frame {}x{}", pixbuf_sub.width(), pixbuf_sub.height());


        // if let Some(scaled_pixbuf) = pixbuf.scale_simple(
        //     256,
        //     256,
        //     gdk_pixbuf::InterpType::Bilinear,
        // ) {
        //     let texture = gdk::Texture::for_pixbuf(&scaled_pixbuf);
        //     self.image_widget.set_paintable(Some(&texture));
        // }
    }
}

fn gtk_init_with_style() -> Result<IconTheme> {
    // Initialize GTK
    gtk::init()?;

    // Set up CSS for transparency and styling
    let css_provider = gtk::CssProvider::new(); //
    let css_content = format!(
        ".main-window {{
                background-color: transparent;
            }}
            .interactive-window {{

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
                font-size: 50px;
                border-radius: 3px;
            }}"
    );
    css_provider.load_from_string(&css_content);

    gtk::style_context_remove_provider_for_display(
        &gdk::Display::default().expect("Could not connect to display"),
        &css_provider,
    );

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("Could not connect to display"),
        &css_provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    let icon_path = data_directory()?.join("src_images/icons");
    let display = gdk::Display::default().unwrap();
    let icon_theme = IconTheme::builder()
        .display(&display)
        .theme_name("Aoe4Icons")
        .search_path(vec![icon_path.to_str().unwrap()])
        .build();
    // log::info!("icon_theme: {:?} {:?}", icon_theme, icon_theme.icon_names());

    Ok(icon_theme)
}

impl OverlayWindow {
    pub fn new(config: OverlayConfig, app: &Application) -> Result<Self> {
        let monitors: gdk::gio::ListModel = gdk::Display::default().unwrap().monitors();
        let monitor = monitors
            .item(0)
            .unwrap()
            .downcast::<gdk::Monitor>()
            .unwrap();

        // Create the main window with configured size
        let window = gtk::ApplicationWindow::builder()
            .title("AOE4 Overlay")
            .default_width(monitor.geometry().width())
            .default_height(monitor.geometry().height()-300)
            .maximized(false)
            .decorated(false)
            .resizable(false)
            .focusable(false)
            .focus_visible(false)
            .modal(false)
            .application(app)
            .css_classes(vec!["main-window"])
            .icon_name("logo")
            .build();

        // React to window close request
        let app_clone = app.clone();
        window.connect_close_request(move |_| {
            log::info!("Window close requested, quitting...");
            app_clone.quit();
            glib::signal::Propagation::Proceed
        });

        // Create overlay container
        let overlay_container = gtk::Overlay::new();

        // Create vertical box for text labels (top-left)
        let text_labels_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text_labels_box.set_halign(gtk::Align::Start);
        text_labels_box.set_valign(gtk::Align::Start);
        text_labels_box.set_margin_start(5);
        text_labels_box.set_margin_top(5);
        overlay_container.add_overlay(&text_labels_box);

        let mut labels: [gtk::Label; AOE4_STATS_POS.len()] = Default::default();
        if config.show_debug_window {
            for (index, stat) in AOE4_STATS_POS.iter().enumerate() {
                let label_text = format!("{}: --", stat.name);
                let label = gtk::Label::new(Some(&label_text));
                label.add_css_class("stat-label");
                label.set_xalign(0.0);
                text_labels_box.append(&label);
                labels[index] = label;
            }
        }

        // Create vertical box for icon labels (top-right)
        let icon_labels_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        icon_labels_box.set_halign(gtk::Align::Center);
        icon_labels_box.set_valign(gtk::Align::Center);
        icon_labels_box.set_margin_end(5);
        icon_labels_box.set_margin_top(5);
        overlay_container.add_overlay(&icon_labels_box);

        let centered_label = gtk::Label::new(None);
        centered_label.add_css_class("icon-label");
        centered_label.set_xalign(0.0);
        //centered_label.set_visible(false);
        icon_labels_box.append(&centered_label);

        // Add overlay container to window
        window.set_child(Some(&overlay_container));

        Ok(Self {
            window,
            _overlay_container: overlay_container,
            _text_labels_box: text_labels_box,
            _icon_labels_box: icon_labels_box,
            labels,
            centered_label,
            config,
        })
    }

    pub fn show(&self) {
        self.window.set_visible(true);
        // Make window input-transparent (non-clickable)
        if let Some(surface) = self.window.surface() {
            surface.set_input_region(&cairo::Region::create());
        } else {
            log::error!("Warning: Could not get GDK surface for the window.");
        }
    }

    pub fn update_image_from_processed_frame(&self, frame: &ProcessedFrame) {
        let mut parts = frame.analysis.detected_texts[INDEX_POP].split("/");
        let current = parts
            .next()
            .unwrap_or_default()
            .parse::<i32>()
            .unwrap_or_default();
        let total = parts
            .next()
            .unwrap_or_default()
            .parse::<i32>()
            .unwrap_or_default();
        let is_useful = total > 0;

        if !is_useful {
            self.centered_label.set_text("NO");
        } else {
            let is_pop = current + 2 >= total;
            let is_idle = frame.analysis.detected_texts[INDEX_IDLE]
                .parse::<i32>()
                .unwrap_or_default()
                > 0;
            let has_villager = frame.analysis.has_villager_icon;

            if is_pop {
                self.centered_label.set_text("Haus!");
                //self.centered_label.set_visible(true);
            } else if is_idle {
                self.centered_label.set_text("Idle!");
                //self.centered_label.set_visible(true);
            } else if !has_villager {
                self.centered_label.set_text("Villager!");
                //self.centered_label.set_visible(true);
            } else {
                self.centered_label.set_text("");
                // self.centered_label.set_visible(false);
                // self.centered_label.set_child_visible(false);
            }
        }

        if self.config.show_debug_window {
            for (index, stat) in AOE4_STATS_POS.iter().enumerate() {
                let text = &frame.analysis.detected_texts[index];
                let label = &self.labels[index];
                if text.is_empty() || text == "--" {
                    label.set_text(&format!("{}: --", stat.name));
                } else {
                    label.set_text(&format!("{}: {}", stat.name, text));
                }
            }
            //
            // // Crop to region of interest (bottom 500px)
            // let pixbuf = frame.original.to_pixbuf();
            // let crop_height = pixbuf.height().min(500);
            // let crop_width = pixbuf.width().min(300);
            // let y = (pixbuf.height() - crop_height).max(0);
            // let pixbuf_sub =
            //     pixbuf.new_subpixbuf(0, y, crop_width, crop_height); // pixbuf.height() - crop_height
            //
            // let texture = gdk::Texture::for_pixbuf(&pixbuf_sub);
            // self.image_widget.set_paintable(Some(&texture));
            // self.image_widget.set_visible(true);
            // info!("update_image_from_processed_frame {}x{}", pixbuf_sub.width(), pixbuf_sub.height());
        }
    }
}

fn activate_ui(
    app: &Application,
    control_sender: Sender<ControlEvent>,
    mut gtk_receiver: Receiver<GuiCommand>,
    config: OverlayConfig,
    wait_for_process: bool,
) -> Result<()> {
    let window = OverlayWindow::new(config, &app)?;
    let interactive_window = InteractWindow::new(control_sender.clone(), &app, wait_for_process)?;
    interactive_window.show();
    window.show();

    // Use Rc for single-threaded reference counting within GTK thread
    let window = std::rc::Rc::new(window);
    let interactive_window = std::rc::Rc::new(interactive_window);

    let app_clone = app.clone();
    glib::spawn_future_local(async move {
        while let Some(gui_command) = gtk_receiver.recv().await {
            match gui_command {
                GuiCommand::ProcessedFrame(processed_frame) => {
                    window.update_image_from_processed_frame(&processed_frame);
                    interactive_window.update_image_from_processed_frame(processed_frame);
                }
                GuiCommand::Quit => {
                    log::info!("Quit command received from channel, quitting...");
                    app_clone.quit();
                    break;
                }
                GuiCommand::StateCaptureStarted => {
                    interactive_window.update_capture_state(true);
                }
                GuiCommand::StateCaptureStopped => {
                    interactive_window.update_capture_state(false);
                }
                GuiCommand::ProcessRunning(running) => {
                    interactive_window.update_process_running_state(running);
                }
            }
        }
    });

    Ok(())
}

pub fn create(
    control_sender: Sender<ControlEvent>,
    gtk_receiver: Receiver<GuiCommand>,
    config: OverlayConfig,
    wait_for_process: bool,
) -> Result<Application> {
    // Initialize GTK in the main thread
    if gtk::init().is_err() {
        error!("Failed to initialize GTK.");
    }

    // Start the GTK thread
    let _icon_theme = gtk_init_with_style()?;
    // let main_context = glib::MainContext::default();
    // let main_loop = glib::MainLoop::new(Some(&main_context), false);

    let app = Application::builder()
        .application_id("org.aoe4_overlay")
        .version("0.1")
        .build();

    let receiver = RefCell::new(Some(gtk_receiver));

    app.connect_activate(move |app| {
        let gtk_receiver = receiver
            .borrow_mut()
            .take()
            .expect("GTK receiver already taken");
        activate_ui(app, control_sender.clone(), gtk_receiver, config.clone(), wait_for_process).unwrap();
    });

    Ok(app)
}
