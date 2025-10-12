use crate::pipewire_stream::{PipeWireStream, PipewireMessage};
use anyhow::{Result, anyhow};
use log::info;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc::SyncSender},
    thread,
};
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
    #[allow(dead_code)]
    Show,
}

use crate::{
    dbus_portal_screen_cast::ScreenCastProxy, pixelbuf_wrapper::PixelBufWrapperWithDroppedFramesTS,
};

pub struct WaylandRecorder {
    connection: Connection,
    screen_cast_proxy: ScreenCastProxy<'static>,
    session_path: String,
    restore_token: Option<String>,
    id: OwnedValue,
    stream_node_id: Arc<Mutex<Option<u32>>>,
}

pub struct WaylandStopHandler {
    pub connection: Connection,
    pub session_path: String,
}


pub async fn close_session(session_path: &str, connection: &Connection) {
    if session_path.len() == 0 {
        return;
    }
    log::info!(
        "Closing session...: {:?}",
        session_path.replace("request", "session")
    );
    if let Err(err) = connection
        .clone()
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            session_path.replace("request", "session"),
            Some("org.freedesktop.portal.Session"),
            "Close",
            &(),
        )
        .await
    {
        log::error!("Failed to close session: {}", err);
    }
}

impl WaylandStopHandler {
    pub async fn stop(&self) {
        close_session(&self.session_path, &self.connection).await;
        let _ = self.connection.clone().close().await;
    }
}

impl WaylandRecorder {
    pub async fn new(id: &str) -> Result<Self> {
        let connection = Connection::session()
            .await
            .expect("failed to connect to session bus");
        let screen_cast_proxy = ScreenCastProxy::new(&connection)
            .await
            .expect("failed to create dbus proxy for screen-cast");

        Ok(WaylandRecorder {
            connection,
            screen_cast_proxy,
            session_path: String::new(),
            id: Value::from(id).try_to_owned().unwrap(),
            stream_node_id: Arc::new(Mutex::new(None)),
            restore_token: None,
        })
    }

    pub fn get_stop_handler(&self) -> WaylandStopHandler {
        WaylandStopHandler {
            connection: self.connection.clone(),
            session_path: self.session_path.clone(),
        }
    }

    pub async fn run(
        &mut self,
        record_type: RecordTypes,
        cursor_mode_type: CursorModeTypes,
        pw_sender: pipewire::channel::Sender<PipewireMessage>,
    ) -> Result<()> {
        info!("Starting...");

        if let Ok(restore_token) = std::fs::read_to_string("restore_token.txt") {
            self.restore_token = Some(restore_token);
            info!("Loaded restore token from file");
        }

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
                message::Type::Error => {
                    let body = msg.body();
                    let s = body.deserialize::<String>()?;
                    log::error!(
                        "Error message received: {:?}. Session handle: {}",
                        s,
                        self.session_path
                    );
                }
                message::Type::Signal => {
                    let body = msg.body();
                    let (_response_num, response) =
                        body.deserialize::<(u32, HashMap<&str, Value>)>()?;

                    // if response_num > 0 {
                    //     return Ok(false);
                    // }

                    if response.len() == 0 {
                        continue;
                    }

                    if response.contains_key("session_handle") {
                        let session_path = response
                            .get("session_handle")
                            .ok_or(anyhow!("No session_handle in response"))?
                            .clone()
                            .downcast::<String>()?;
                        self.session_path = session_path.clone();

                        self.handle_session(record_type, cursor_mode_type)
                            .await
                            .map_err(|e| {
                                anyhow!("{}. Session handle: {}", e, &self.session_path)
                            })?;
                        continue;
                    }

                    if response.contains_key("streams") {
                        if let Some(restore_token) = response.get("restore_token") {
                            let restore_token = restore_token.downcast_ref::<&str>()?;
                            self.restore_token = Some(restore_token.to_string());
                            log::info!("Got restore token: {}", restore_token);
                            std::fs::write("restore_token.txt", restore_token)?;
                        }
                        let node_id = self.parse_stream_response(response.clone()).await?;
                        let _ = pw_sender.send(PipewireMessage::Connect(node_id));
                        break;
                    }
                }
                _ => {
                    log::warn!("Unknown message: {:?}", msg);
                }
            }
        }

        log::info!("No more messages. Session path: {}", self.session_path);
        Ok(())
    }

    pub async fn close_session(&mut self) {
        if self.session_path.len() == 0 {
            return;
        }
        log::info!(
            "Closing session...: {:?}",
            self.session_path.replace("request", "session")
        );
        if let Err(err) = self
            .connection
            .clone()
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                self.session_path.clone().replace("request", "session"),
                Some("org.freedesktop.portal.Session"),
                "Close",
                &(),
            )
            .await
        {
            log::error!("Failed to close session: {}", err);
        }
        self.session_path = String::new();
    }

    async fn handle_session(
        &mut self,
        record_type: RecordTypes,
        cursor_mode_type: CursorModeTypes,
    ) -> Result<()> {
        let types_value: Value = match record_type {
            RecordTypes::Monitor => Value::from(1u32),
            RecordTypes::Window => Value::from(2u32),
        };
        let cursor_mode_value: Value = match cursor_mode_type {
            CursorModeTypes::Hidden => Value::from(1u32),
            CursorModeTypes::Show => Value::from(2u32),
        };
        let multiple_value: Value = Value::from(false);
        let persist_mode_value: Value = Value::from(0u32); // Value::from(2u32);
        let id_value = Value::from(self.id.clone());
        let mut option_map: HashMap<&str, &Value> = HashMap::from([
            ("handle_token", &id_value),
            ("types", &types_value),
            ("cursor_mode", &cursor_mode_value),
            ("multiple", &multiple_value),
            ("persist_mode", &persist_mode_value),
        ]);

        // let (restore_token, has_restore_token) = if let Some(restore_token) = &self.restore_token
        // {     (Value::from(restore_token.clone()), true)
        // } else {
        //     (Value::from(""), false)
        // };
        //
        // if has_restore_token {
        //     option_map.insert("restore_token", &restore_token);
        // }

        self.screen_cast_proxy
            .select_sources(
                &ObjectPath::try_from(self.session_path.clone())?,
                option_map,
            )
            .await?;

        let id_value = Value::from(self.id.clone());
        let _response = self
            .screen_cast_proxy
            .start(
                &ObjectPath::try_from(self.session_path.clone())?,
                "parent_window",
                HashMap::from([("handle_token", &id_value)]),
            )
            .await?;
        Ok(())
    }

    async fn parse_stream_response(&mut self, response: HashMap<&str, Value<'_>>) -> Result<u32> {
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

        Ok(stream_node_id)
    }
}

impl Drop for WaylandRecorder {
    fn drop(&mut self) {
        let _ = self.connection.clone().close();
    }
}
