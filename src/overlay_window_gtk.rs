use crate::{frame_processor::ProcessedFrame, system_menu::SystemTray};
use anyhow::Result;
use aoe4_overlay::consts::{AOE4_STATS_POS, AREA_HEIGHT, AREA_WIDTH, INDEX_IDLE, INDEX_POP};
use gtk::{Application, Button, IconTheme, Label, cairo, glib, prelude::*};
use tokio::{
    sync::mpsc::{Receiver, Sender},
    task,
};

#[derive(Clone, Debug)]
pub struct OverlayConfig {
    pub show_debug_window: bool,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            show_debug_window: false,
        }
    }
}

pub enum GuiCommand {
    AboutToProcessFrames,
    ProcessedFrame(ProcessedFrame),
    Quit,
}

pub struct OverlayWindow {
    window: gtk::ApplicationWindow,
    image_widget: gtk::Picture,
    _overlay_container: gtk::Overlay,
    _text_labels_box: gtk::Box,
    _icon_labels_box: gtk::Box,
    config: OverlayConfig,
    pub centered_label: Label,
    pub labels: [Label; AOE4_STATS_POS.len()],
}

pub struct InteractWindow {
    window: gtk::Window,
    _quit_button: Button,
}

impl InteractWindow {
    pub fn new(sender: Sender<GuiCommand>, app: &Application) -> Result<Self> {
        let window = gtk::Window::builder()
            .title("AOE4 Overlay Interaction")
            .maximized(false)
            .decorated(false)
            .resizable(false)
            .focusable(true)
            .focus_visible(true)
            .modal(false)
            .css_classes(vec!["interactive-window"])
            .application(app)
            .build();

        // Create quit button
        let quit_button = gtk::Button::with_label("Quit");
        quit_button.set_halign(gtk::Align::Start);
        quit_button.set_valign(gtk::Align::Start);
        quit_button.set_child_visible(true);
        quit_button.connect_clicked(move |_| {
            log::info!("Quit button clicked, quitting...");
            let _ = sender.try_send(GuiCommand::Quit);
        });

        // Add overlay container to window
        window.set_child(Some(&quit_button));

        Ok(Self {
            window,
            _quit_button: quit_button,
        })
    }

    pub fn show(&self) {
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }
}

fn gtk_init_with_style() -> Result<IconTheme> {
    // Initialize GTK
    gtk::init()?;

    // Set up CSS for transparency and styling
    let css_provider = gtk::CssProvider::new();
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

    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("Could not connect to display"),
        &css_provider,
        gtk::STYLE_PROVIDER_PRIORITY_USER,
    );

    let display = gdk::Display::default().unwrap();
    let icon_theme = IconTheme::builder()
        .display(&display)
        .theme_name("Aoe4Icons")
        .search_path(vec!["src_images/icons"])
        .build();
    log::info!("icon_theme: {:?} {:?}", icon_theme, icon_theme.icon_names());

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
            .default_height(monitor.geometry().height())
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

        // Create overlay container
        let overlay_container = gtk::Overlay::new();

        // Create image widget for displaying screen capture
        let image_widget = gtk::Picture::new();
        image_widget.set_halign(gtk::Align::End);
        image_widget.set_valign(gtk::Align::Start);
        image_widget.set_size_request(AREA_WIDTH, AREA_HEIGHT);
        image_widget.set_child_visible(config.show_debug_window);
        overlay_container.set_child(Some(&image_widget));

        // Create vertical box for text labels (top-left)
        let text_labels_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text_labels_box.set_halign(gtk::Align::Start);
        text_labels_box.set_valign(gtk::Align::Start);
        text_labels_box.set_margin_start(5);
        text_labels_box.set_margin_top(5);
        overlay_container.add_overlay(&text_labels_box);

        let mut labels: [gtk::Label; AOE4_STATS_POS.len()] = Default::default();
        if config.show_debug_window {
            for (index, stat) in aoe4_overlay::consts::AOE4_STATS_POS.iter().enumerate() {
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
            image_widget,
            _overlay_container: overlay_container,
            _text_labels_box: text_labels_box,
            _icon_labels_box: icon_labels_box,
            labels,
            centered_label,
            config,
        })
    }

    pub fn enable_waiting(&self, enable: bool) {
        if enable {
            self.centered_label.set_text("Waiting...");
        } else {
            self.centered_label.set_text("");
        }
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

    pub fn update_image_from_processed_frame(&self, frame: ProcessedFrame) {
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
            self.centered_label.set_text("");
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

            // Crop to region of interest (bottom 500px)
            let pixbuf = frame.original.to_pixbuf();
            let crop_height = pixbuf.height().min(500);
            let crop_width = pixbuf.width().min(300);
            let pixbuf =
                pixbuf.new_subpixbuf(0, pixbuf.height() - crop_height, crop_width, crop_height);

            let texture = gdk::Texture::for_pixbuf(&pixbuf);
            self.image_widget.set_paintable(Some(&texture));
            //
            // if let Some(scaled_pixbuf) = pixbuf.scale_simple(
            //     self.config.width,
            //     self.config.height,
            //     gdk_pixbuf::InterpType::Bilinear,
            // ) {
            //     let texture = gdk::Texture::for_pixbuf(&scaled_pixbuf);
            //     self.image_widget.set_paintable(Some(&texture));
            // }
        }
    }
}

pub async fn run(
    gtk_sender: Sender<GuiCommand>,
    mut gtk_receiver: Receiver<GuiCommand>,
    config: OverlayConfig,
    enable_waiting: bool,
) -> Result<()> {
    // Start the GTK thread
    let gtk_handle = std::thread::spawn(move || -> Result<()> {
        let _icon_theme = gtk_init_with_style()?;
        let main_context = glib::MainContext::default();
        let main_loop = glib::MainLoop::new(Some(&main_context), false);

        let app = Application::builder()
            .application_id("org.aoe4_overlay")
            .version("0.1")
            .build();

        let window = OverlayWindow::new(config, &app)?;
        let interactive_window = InteractWindow::new(gtk_sender.clone(), &app)?;

        if enable_waiting {
            interactive_window.show();
        }
        window.enable_waiting(enable_waiting);

        // Initialize system tray icon with quit handler
        let _tray = SystemTray::new(gtk_sender.clone())?;

        // Set up a timeout to check for quit signal and process images
        let main_loop_quit = main_loop.clone();

        // Use Rc for single-threaded reference counting within GTK thread
        let window_rc = std::rc::Rc::new(window);
        let window_for_image_updates = std::rc::Rc::clone(&window_rc);

        main_context.spawn_local(async move {
            while let Some(gui_command) = gtk_receiver.recv().await {
                match gui_command {
                    GuiCommand::ProcessedFrame(processed_frame) => {
                        window_for_image_updates.update_image_from_processed_frame(processed_frame);
                    }
                    GuiCommand::Quit => {
                        log::info!("Quit command received from channel, quitting...");
                        main_loop_quit.quit();
                        break;
                    }
                    GuiCommand::AboutToProcessFrames => {
                        interactive_window.hide();
                        window_for_image_updates.enable_waiting(false);
                    }
                }
            }
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
