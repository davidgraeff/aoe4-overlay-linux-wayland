use crate::pipewire_stream::PipewireMessage;
use anyhow::{Result, anyhow};
use ashpd::desktop::{PersistMode, Session};
pub use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};
use ashpd::enumflags2::BitFlags;

pub struct WaylandRecorder {
    restore_token: Option<String>,
    proxy: Screencast<'static>,
    session: Option<Session<'static, Screencast<'static>>>

}

#[derive(Clone)]
pub struct WaylandStopHandler {
}

impl WaylandStopHandler {
    pub async fn stop(&self) {

    }
}

impl WaylandRecorder {
    pub async fn new() -> Result<(Self, WaylandStopHandler)> {
        let proxy =Screencast::new().await?;
        let stop_handler = WaylandStopHandler {
        };
        Ok((
            WaylandRecorder {
                restore_token: None,
                proxy,
                session: None,
            },
            stop_handler,
        ))
    }

    pub fn is_running(&self) -> bool {
        self.session.is_some()
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(session) = &self.session {
            session.close().await?;
            self.session = None;
            log::info!("Closed screen cast session");
        }
        Ok(())
    }

    pub async fn start(
        &mut self,
        types: BitFlags<SourceType>,
        pw_sender: pipewire::channel::Sender<PipewireMessage>,
    ) -> Result<()> {
        log::info!("Starting...");

        if let Ok(restore_token) = std::fs::read_to_string("restore_token.txt") {
            self.restore_token = Some(restore_token);
            log::info!("Loaded restore token from file");
        }

        let session = self.proxy.create_session().await?;
        self.proxy
            .select_sources(
                &session,
                CursorMode::Hidden,
                types,
                false,
                self.restore_token.as_ref().map(String::as_str),
                PersistMode::ExplicitlyRevoked,
            )
            .await?;

        let response = self.proxy.start(&session, None).await?.response()?;
        if let Some(stream) = response.streams().first() {
            if let Some(token) = response.restore_token() {
                std::fs::write("restore_token.txt", token).map_err(|e| anyhow!("Failed to write restore token: {}", e))?;
                log::info!("Saved restore token to file");
            }
            self.session = Some(session);
            let _ = pw_sender.send(PipewireMessage::Connect(stream.pipe_wire_node_id()));
            return Ok(());
        }

        log::info!("Screen cast portal closed");
        Ok(())
    }
}
