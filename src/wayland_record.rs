//use gst::prelude::*;
use crate::{overlay_window_gtk::PixbufWrapper, pipewire_stream::PipeWireStream};
use anyhow::Result;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc::SyncSender},
    thread,
    thread::JoinHandle,
};
use std::future::AsyncDrop;
use std::pin::Pin;
use zbus::{
    Connection, MessageStream,
    export::ordered_stream::OrderedStreamExt,
    message,
    zvariant::{Dict, ObjectPath, OwnedValue, Structure, Value},
};

#[derive(Clone, Copy)]
pub enum RecordTypes {
    Monitor,
    Window,
}

#[derive(Clone, Copy)]
pub enum CursorModeTypes {
    Hidden,
    Show,
}

use crate::dbus_portal_screen_cast::ScreenCastProxy;

enum PipewireMessage {
    Stop,
    Connect(u32),
}

pub struct WaylandRecorder {
    connection: Connection,
    screen_cast_proxy: ScreenCastProxy<'static>,
    session_path: String,
    id: OwnedValue,
    stream_node_id: Arc<Mutex<Option<u32>>>,
    pw_thread: Option<JoinHandle<()>>,
    pw_sender: pipewire::channel::Sender<PipewireMessage>,
}

impl WaylandRecorder {
    pub async fn new(id: &str) -> Result<Self> {
        let connection = Connection::session()
            .await
            .expect("failed to connect to session bus");
        let screen_cast_proxy = ScreenCastProxy::new(&connection)
            .await
            .expect("failed to create dbus proxy for screen-cast");

        let (pw_sender, _pw_receiver) = pipewire::channel::channel::<PipewireMessage>();

        Ok(WaylandRecorder {
            connection,
            screen_cast_proxy,
            session_path: String::new(),
            id: Value::from(id).try_to_owned().unwrap(),
            stream_node_id: Arc::new(Mutex::new(None)),
            pw_thread: None,
            pw_sender,
        })
    }

    pub async fn start(
        &mut self,
        record_type: RecordTypes,
        cursor_mode_type: CursorModeTypes,
        sender: SyncSender<PixbufWrapper>
    ) -> bool {
        let (pw_sender, pw_receiver) = pipewire::channel::channel::<PipewireMessage>();
        self.pw_sender = pw_sender;

        let pw_thread = thread::spawn(move || {
            let pipewire_stream = PipeWireStream::new(sender).unwrap();
            let mainloop = pipewire_stream.main_loop.clone();
            let mainloop_clone = pipewire_stream.main_loop.clone();
            let pipewire_stream_arc = Arc::new(Mutex::new(pipewire_stream));
            let _receiver = pw_receiver.attach(mainloop.loop_(), {
                move |m| match m {
                    PipewireMessage::Stop => {
                        log::info!("PipeWire main loop: Received stop message, quitting...");
                        mainloop_clone.quit()
                    }
                    PipewireMessage::Connect(stream_node_id) => {
                        let mut pipewire_stream = pipewire_stream_arc.lock().unwrap();
                        pipewire_stream.connect_to_node(stream_node_id).unwrap();
                    }
                }
            });
            log::info!("Starting PipeWire main loop");
            mainloop.run();
            log::info!("Finished PipeWire main loop");
        });
        self.pw_thread = Some(pw_thread);

        let id_value = Value::from(self.id.clone());
        let option_map: HashMap<&str, &Value> = [
            ("handle_token", &id_value),
            ("session_handle_token", &id_value),
        ]
        .into();
        self.screen_cast_proxy
            .create_session(option_map)
            .await
            .expect("failed to create session");

        let mut message_stream: MessageStream = self.connection.clone().into();

        while let Some(Ok(msg)) = message_stream.next().await {
            match msg.message_type() {
                message::Type::Signal => {
                    let body = msg.body();
                    let (response_num, response) = body
                        .deserialize::<(u32, HashMap<&str, Value>)>()
                        .expect("failed to handle session");

                    if response_num > 0 {
                        return false;
                    }

                    if response.len() == 0 {
                        continue;
                    }

                    if response.contains_key("session_handle") {
                        self.handle_session(
                            self.screen_cast_proxy.clone(),
                            response.clone(),
                            record_type,
                            cursor_mode_type,
                        )
                        .await
                        .expect("failed to handle session");
                        continue;
                    }

                    if response.contains_key("streams") {
                        self.record_screen_cast(response.clone())
                            .await
                            .expect("failed to record screen cast");
                        break;
                    }
                }
                _ => {
                    log::warn!("Unkown message: {:?}", msg);
                }
            }
        }

        true
    }

    pub async fn stop(&mut self) {
        log::info!("Stopping Wayland Recorder");
        let _ = self.pw_sender.send(PipewireMessage::Stop);

        if let Some(pw_thread) = self.pw_thread.take() {
            let _ = pw_thread.join();
        }

        if self.session_path.len() > 0 {
            println!(
                "Closing session...: {:?}",
                self.session_path.replace("request", "session")
            );
            self.connection
                .clone()
                .call_method(
                    Some("org.freedesktop.portal.Desktop"),
                    self.session_path.clone().replace("request", "session"),
                    Some("org.freedesktop.portal.Session"),
                    "Close",
                    &(),
                )
                .await
                .expect("failed to close session");
            self.session_path = String::new();
        }
    }

    async fn handle_session(
        &mut self,
        screen_cast_proxy: ScreenCastProxy<'_>,
        response: HashMap<&str, Value<'_>>,
        record_type: RecordTypes,
        cursor_mode_type: CursorModeTypes,
    ) -> Result<()> {
        let response_session_handle = response
            .get("session_handle")
            .expect("cannot get session_handle")
            .clone()
            .downcast::<String>()
            .expect("cannot down cast session_handle");

        self.session_path = response_session_handle.clone();

        let types_value: Value = match record_type {
            RecordTypes::Monitor => Value::from(1u32),
            RecordTypes::Window => Value::from(2u32),
        };
        let cursor_mode_value: Value = match cursor_mode_type {
            CursorModeTypes::Hidden => Value::from(1u32),
            CursorModeTypes::Show => Value::from(2u32),
        };
        let id_value = Value::from(self.id.clone());
        let option_map: HashMap<&str, &Value> = HashMap::from([
            ("handle_token", &id_value),
            ("types", &types_value),
            ("cursor_mode", &cursor_mode_value),
        ]);

        screen_cast_proxy
            .select_sources(
                &ObjectPath::try_from(response_session_handle.clone())?,
                option_map,
            )
            .await?;

        let id_value = Value::from(self.id.clone());
        screen_cast_proxy
            .start(
                &ObjectPath::try_from(response_session_handle.clone())?,
                "parent_window",
                HashMap::from([("handle_token", &id_value)]),
            )
            .await?;
        Ok(())
    }

    async fn record_screen_cast(&mut self, response: HashMap<&str, Value<'_>>) -> Result<()> {
        let streams: &Value<'_> = response.get("streams").expect("cannot get streams");

        // get fields from nested structure inside elements
        let streams = streams
            .clone()
            .downcast::<Vec<Value>>()
            .expect("cannot down cast streams to vec array");

        let first_stream = streams
            .first()
            .expect("cannot get first object from streams array")
            .clone()
            .downcast::<Structure>()
            .expect("cannot down cast first object to structure");

        let stream_node_id: u32 = first_stream.fields()[0]
            .downcast_ref::<u32>()
            .expect("cannot down cast first field to u32");

        let meta = first_stream.fields()[1]
            .downcast_ref::<Dict>()
            .expect("cannot down cast meta to dict");

        // log::info!("Meta: {:?}", meta);
        // Meta: Dict { map: {Str("id"): Value(Str("0")), Str("position"): Value(Structure(Structure
        // { fields: [I32(0), I32(0)], signature: Structure(Dynamic { fields: [I32, I32] }) })),
        // Str("size"): Value(Structure(Structure { fields: [I32(2560), I32(1440)], signature:
        // Structure(Dynamic { fields: [I32, I32] }) })), Str("source_type"): Value(U32(1))},
        // signature: Dict { key: Dynamic { child: Str }, value: Dynamic { child: Variant } } }

        let key = zbus::zvariant::Str::from_static("id");
        let id_struct: Option<Value> = meta.get(&key)?;
        let key = zbus::zvariant::Str::from_static("position");
        let position_struct: Option<Value> = meta.get(&key)?;
        let key = zbus::zvariant::Str::from_static("size");
        let size_struct: Option<Value> = meta.get(&key)?;
        let key = zbus::zvariant::Str::from_static("source_type");
        let source_type_struct: Option<Value> = meta.get(&key)?;

        log::info!("Stream Node ID: {}", stream_node_id);
        if let Some(id_struct) = id_struct {
            let id: &str = id_struct
                .downcast_ref::<&str>()
                .expect("cannot down cast id to &str");
            log::info!("Stream ID: {}", id);
        }
        if let Some(position_struct) = position_struct {
            let position = position_struct
                .clone()
                .downcast::<Structure>()
                .expect("cannot down cast position to structure");
            let x = position.fields()[0]
                .downcast_ref::<i32>()
                .expect("cannot down cast x to i32");
            let y = position.fields()[1]
                .downcast_ref::<i32>()
                .expect("cannot down cast y to i32");
            log::info!("Position: x={}, y={}", x, y);
        }
        if let Some(size_struct) = size_struct {
            let size = size_struct
                .clone()
                .downcast::<Structure>()
                .expect("cannot down cast size to structure");
            let width = size.fields()[0]
                .downcast_ref::<i32>()
                .expect("cannot down cast width to i32");
            let height = size.fields()[1]
                .downcast_ref::<i32>()
                .expect("cannot down cast height to i32");
            log::info!("Size: width={}, height={}", width, height);
        }
        if let Some(source_type_struct) = source_type_struct {
            let source_type = source_type_struct
                .downcast_ref::<u32>()
                .expect("cannot down cast source_type to u32");
            log::info!("Source Type: {}", source_type);
        }

        // Store the stream node ID
        let mut stream_node_id_lock = self
            .stream_node_id
            .lock()
            .expect("cannot lock stream_node_id");
        *stream_node_id_lock = Some(stream_node_id);

        let _ = self
            .pw_sender
            .send(PipewireMessage::Connect(stream_node_id));

        Ok(())
    }
}

impl AsyncDrop for WaylandRecorder {
    async fn drop(mut self: Pin<&mut Self>) {
        log::info!("Dropping Wayland Recorder");
        self.stop().await;
    }
}

impl Drop for WaylandRecorder {
    fn drop(&mut self) {
        log::info!("Dropping Wayland Recorder");
        let _ = self.pw_sender.send(PipewireMessage::Stop);

        if let Some(pw_thread) = self.pw_thread.take() {
            let _ = pw_thread.join();
        }
    }
}