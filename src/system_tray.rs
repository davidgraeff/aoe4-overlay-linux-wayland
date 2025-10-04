use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use libappindicator_zbus::{
    utils::{
        ButtonOptions, EventUpdate, IconPixmap, MenuStatus, MenuUnit,
    },
};
use zbus::fdo::Result;

// Binary include "logo.png" as a byte array
const LOGO: &[u8] = include_bytes!("logo.png");

pub(crate) struct Base {
    pixmap: IconPixmap,
}

impl Base {
    pub(crate) fn boot() -> Self {
        let data = image::load_from_memory(LOGO).unwrap();
        let pixmap = IconPixmap {
            width: 140,
            height: 140,
            data: data.as_bytes().to_vec(),
        };
        Self { pixmap }
    }
    pub(crate) fn icon_pixmap(&self) -> Result<Vec<IconPixmap>> {
        Ok(vec![self.pixmap.clone()])
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Message {
    Clicked,
    Toggled,
}

pub(crate) struct Menu {
    menu: MenuUnit<Message>,
}

impl Menu {
    pub(crate) fn boot() -> Self {
        let menu = MenuUnit::root()
            .push_sub_menu(MenuUnit::button(
                ButtonOptions {
                    label: "Quit".to_owned(),
                    enabled: true,
                    icon_name: "nheko".to_owned(),
                },
                Message::Clicked,
            ));
        Menu { menu}
    }

    pub(crate) fn menu(&self) -> MenuUnit<Message> {
        self.menu.clone()
    }
    pub(crate) fn status(&self) -> MenuStatus {
        MenuStatus::Normal
    }

    pub(crate) fn on_clicked(&mut self, _message: Message, _timestamp: u32) -> EventUpdate {
        //self.should_quit_tray_icon.store(true, std::sync::atomic::Ordering::Relaxed);
        EventUpdate::None
    }
}
//
// pub async fn show_tray_icon() -> zbus::Result<impl Future<Output = ()>> {
//     let connection: TrayConnection<_, _> = tray(
//         Base::boot,
//         "com.aoe4.overlay.tray",
//         "Age of Empires IV Overlay",
//         Menu::boot,
//         Menu::menu,
//         1,
//     )
//     .with_icon_pixmap(Base::icon_pixmap)
//     .with_item_is_menu(false)
//     .with_category(Category::ApplicationStatus)
//     .with_menu_status(Menu::status)
//     .with_on_clicked(Menu::on_clicked)
//     .run()
//     .await?;
//     Ok(async move {
//         let _ = connection;
//     })
// }
