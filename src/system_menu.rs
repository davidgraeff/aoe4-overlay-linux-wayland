use anyhow::Result;
use gtk::gio;
use log::info;
use gtk::prelude::{ActionMapExt, ApplicationExt};
use crate::overlay_window_gtk::GuiCommand;

pub struct SystemTray {
    _app: gtk::Application,
}

impl SystemTray {
    pub fn new(gtk_sender: tokio::sync::mpsc::Sender<GuiCommand>) -> Result<Self> {
        // Create an application for the system tray
        let app = gtk::Application::builder()
            .application_id("com.aoe4.overlay.tray")
            .flags(gio::ApplicationFlags::default())
            .build();

        // Create a simple action for quit
        let quit_action = gio::SimpleAction::new("quit", None);
        quit_action.connect_activate(move |_, _| {
            info!("Quit action triggered from menu");
            let _ = gtk_sender.try_send(GuiCommand::Quit);
        });

        app.add_action(&quit_action);

        // Register the application (required for actions to work)
        app.register(None::<&gio::Cancellable>)?;

        // Create a notification to indicate the app is running
        let notification = gio::Notification::new("AOE4 Overlay");
        notification.set_body(Some("Running - Press Ctrl+C to quit"));
        notification.add_button("Quit", "app.quit");
        app.send_notification(Some("running"), &notification);

        Ok(Self {
            _app: app,
        })
    }
}
