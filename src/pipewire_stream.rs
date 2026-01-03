use crate::pixelbuf_wrapper::PixelBufWrapperWithDroppedFramesTS;
use anyhow::Result;
use pipewire::{
    context::Context,
    main_loop::MainLoop,
    spa,
    spa::{
        pod::{ChoiceValue, serialize::PodSerializer},
        sys::{SPA_PARAM_EnumFormat, SPA_TYPE_OBJECT_Format},
        utils,
        utils::{ChoiceEnum, ChoiceFlags, Direction},
    },
    stream::{Stream, StreamListener, StreamState},
};
use spa::{
    param::{
        ParamType,
        format::{MediaSubtype, MediaType},
    },
    pod::{Object, Pod, Property, Value},
};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use pipewire::spa::sys::{spa_format_parse, spa_format_video_raw_parse, spa_video_info_raw};

pub struct UserData {
    last_time: u64,
    mainloop: MainLoop,
}

pub struct PipeWireStreamState {
    pub stream: Stream,
    pub listener: StreamListener<UserData>,
}

/// Manages a PipeWire stream for screen capturing and sends images via a channel.
#[derive(Clone)]
pub struct PipeWireStream {
    context: Context,
    state: Arc<Mutex<Option<PipeWireStreamState>>>,
    image_sender: mpsc::SyncSender<bool>,
    image_sender_content: PixelBufWrapperWithDroppedFramesTS,
    pub mainloop: MainLoop,
}

impl PipeWireStream {

    pub fn new_communication() -> (
        pipewire::channel::Receiver<PipewireMessage>,
        PipeWireStopHandler,
    ) {
        let (pw_sender, pw_receiver) = pipewire::channel::channel::<PipewireMessage>();
        let stop_handler = PipeWireStopHandler { pw_sender };

        (pw_receiver, stop_handler)
    }

    /// Creates a new PipeWireStream instance.
    pub fn new(
        image_sender: mpsc::SyncSender<bool>,
        image_sender_content: PixelBufWrapperWithDroppedFramesTS,
    ) -> Result<Self> {
        pipewire::init();

        let mainloop = MainLoop::new(None)?;

        let mainloop_clone = mainloop.clone();
        let pipewire_stream = Self {
            context: Context::new(&mainloop_clone)?,
            state: Arc::new(Mutex::new(None)),
            image_sender,
            image_sender_content,
            mainloop,
        };

        Ok(pipewire_stream)
    }

    pub fn handle(&self, m: PipewireMessage) {
        match m {
            PipewireMessage::Stop => {
                log::info!("PipeWire main loop: Received stop message, quitting...");
                self.mainloop.quit()
            }
            PipewireMessage::Connect(stream_node_id) => {
                self.connect_to_node(stream_node_id).unwrap();
            }
        }
    }

    pub fn connect_to_node(&self, node_id: u32) -> Result<()> {
        let core = self.context.connect(None)?;

        // Create stream properties
        let props = pipewire::properties::properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Screen",
        };

        let stream = Stream::new(&core, "screen-capture", props)?;

        log::info!("Recording Wayland screen cast: {node_id}");

        // Clone sender for the callback
        let sender = self.image_sender.clone();
        let image_sender_content = self.image_sender_content.clone();

        let user_data = UserData {
            last_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::from_secs(0))
                .as_millis() as u64,
            mainloop: self.mainloop.clone(),
        };

        // Set up stream listener
        let listener = stream
            .add_local_listener_with_user_data(user_data)
            .state_changed(
                |_stream,
                 user_data: &mut UserData,
                 old_state: StreamState,
                 new_state: StreamState| {
                    if let StreamState::Error(err) = new_state {
                        log::error!("Stream state to error: {}", err);
                        user_data.mainloop.quit();
                    } else {
                        log::info!(
                            "Stream state changed from {:?} to {:?}",
                            old_state,
                            new_state
                        );
                    }
                },
            )
            .param_changed(|_stream, _user_data: &mut UserData, id, param| {
                if let Some(param) = param {
                    if id == ParamType::Format.as_raw() {

                        let mut media_type: u32 = 0;
                        let mut media_subtype: u32 = 0;
                        let mut uninit: ::std::mem::MaybeUninit<spa_video_info_raw> =
                            ::std::mem::MaybeUninit::uninit();
                        let video_info = uninit.as_mut_ptr();
                        unsafe {
                            spa_format_parse(
                                param.as_raw_ptr(),
                                &mut media_type,
                                &mut media_subtype,
                            );
                            if spa_format_video_raw_parse(param.as_raw_ptr(), video_info) != 0 {
                                log::info!("Stream format changed: width={}, height={}, max_framerate={}", unsafe { (*video_info).size.width }, unsafe { (*video_info).size.height }, unsafe { (*video_info).max_framerate.num });
                                // println!("Stream unknown param changed: {} {:?}", id,
                                //          *video_info);     } else {
                                // println!("Stream unknown param changed: {} (non-video)", id);
                            }
                        }
                    } else if id == ParamType::Latency.as_raw() {
                        log::info!("Stream latency params changed");
                    } else if id == ParamType::Props.as_raw() {
                        log::info!("Stream props changed");
                    } else {
                        log::info!("Stream unknown params changed");
                    }
                }
            })
            .process(move |stream, user_data: &mut UserData| {
                // Parse metadata of pipewire buffer
                //let props = stream.properties();
                //log::info!("Stream properties: {:?}", props);

                let mut buffer = match stream.dequeue_buffer() {
                    None => {
                        log::error!("Failed to dequeue buffer");
                        return;
                    }
                    Some(buffer) => buffer,
                };
                // Reduce framerate to every 100ms (10fps) by comparing timestamps
                // {
                //     use std::time::{Duration, SystemTime, UNIX_EPOCH};
                //     let last = user_data.last_time;
                //     let now = SystemTime::now()
                //         .duration_since(UNIX_EPOCH)
                //         .unwrap_or(Duration::from_secs(0))
                //         .as_millis() as u64;
                //     user_data.last_time = now;
                //     if now.saturating_sub(last) < 50 {
                //         return;
                //     }
                // }

                let data = buffer.datas_mut();
                if data.is_empty() {
                    return;
                }
                let data = &mut data[0];

                let chunk = data.chunk();
                let stride = chunk.stride();
                let size = chunk.size() as usize;
                //log::info!("Buffer received, size: {}, stride: {}", size, stride);

                if data.data().is_none() {
                    return;
                }
                let slice = data.data().unwrap();
                let width = stride / 4; // For BGRx, 4 bytes per pixel
                let height = slice.len() as i32 / stride;

                //log::info!("Buffer received, dimensions: {}x{}", width, height);

                if width <= 0 || height <= 0 || size <= 0 || slice.len() < size {
                    log::error!("Invalid image dimensions: {}x{}", width, height);
                    return;
                }
                //
                // let pixbuf_wrapper = PixbufWrapper {
                //     bgr_buffer: Vec::from(&slice[..size]),
                //     width,
                //     height,
                //     stride,
                // };

                if let Ok(mut content) = image_sender_content.lock() {
                    content
                        .pixbuf
                        .copy_from_slice(&slice[..size], width, height, stride);
                    content.frames_written += 1;
                }

                let _ = sender.try_send(true);
            })
            .register()?;

        // Create video format parameters
        let format = Object {
            type_: SPA_TYPE_OBJECT_Format,
            id: SPA_PARAM_EnumFormat,
            properties: vec![
                Property::new(
                    spa::param::format::FormatProperties::MediaType.as_raw(),
                    Value::Id(spa::utils::Id(MediaType::Video.as_raw())),
                ),
                Property::new(
                    spa::param::format::FormatProperties::MediaSubtype.as_raw(),
                    Value::Id(spa::utils::Id(MediaSubtype::Raw.as_raw())),
                ),
                Property::new(
                    spa::param::format::FormatProperties::VideoFormat.as_raw(),
                    Value::Choice(ChoiceValue::Id(utils::Choice {
                        0: ChoiceFlags::empty(),
                        1: ChoiceEnum::Enum {
                            default: utils::Id(spa::param::video::VideoFormat::BGRx.as_raw()),
                            alternatives: vec![
                                utils::Id(spa::param::video::VideoFormat::BGRx.as_raw()),
                                utils::Id(spa::param::video::VideoFormat::BGRA.as_raw()),
                                utils::Id(spa::param::video::VideoFormat::BGR.as_raw()),
                            ],
                        },
                    })),
                ),
                // Property::new(
                //     spa::param::format::FormatProperties::VideoSize.as_raw(),
                //     Value::Choice(ChoiceValue::Rectangle(utils::Choice {
                //         0: ChoiceFlags::empty(),
                //         1: ChoiceEnum::Enum {
                //             default: utils::Rectangle{width: 320, height: 240},
                //             alternatives: vec![
                //                 utils::Rectangle{width: 320, height: 240},
                //                 utils::Rectangle{width: 1, height: 1},
                //                 utils::Rectangle{width: 4096, height: 4096},
                //             ],
                //         },
                //     })),
                // ),
                Property::new(
                    spa::param::format::FormatProperties::VideoFramerate.as_raw(),
                    Value::Choice(ChoiceValue::Fraction(utils::Choice {
                        0: ChoiceFlags::empty(),
                        1: ChoiceEnum::Enum {
                            default: utils::Fraction { num: 25, denom: 1 },
                            alternatives: vec![
                                utils::Fraction { num: 0, denom: 1 },
                                utils::Fraction { num: 25, denom: 1 },
                                utils::Fraction {
                                    num: 1000,
                                    denom: 1,
                                },
                            ],
                        },
                    })),
                ),
            ],
        };
        let format = Value::Object(format);
        let values: Vec<u8> = PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &format)?
            .0
            .into_inner();
        let mut params = [Pod::from_bytes(&values)
            .ok_or_else(|| anyhow::anyhow!("Failed to create Pod from bytes"))?];

        log::info!("Connecting PipeWire stream...");
        // Connect stream to the node
        stream.connect(
            Direction::Input,
            Some(node_id),
            pipewire::stream::StreamFlags::AUTOCONNECT | pipewire::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        stream.set_active(true)?;
        // if stream.state() != StreamState::Connecting
        //     && stream.state() != StreamState::Streaming
        // {
        //     log::error!("Stream failed to start, state: {:?}", stream.state());
        //     return Err(anyhow::anyhow!("Stream failed to start"));
        // }

        let mut state = self.state.lock().expect("Lock poisoned");
        *state = Some(PipeWireStreamState { stream, listener });
        Ok(())
    }
}

impl Drop for PipeWireStream {
    fn drop(&mut self) {
        let state = self.state.lock().expect("Lock poisoned");
        if let Some(state) = state.as_ref() {
            let _ = state.stream.disconnect();
        }
    }
}

pub enum PipewireMessage {
    Stop,
    Connect(u32),
}

#[derive(Clone)]
pub struct PipeWireStopHandler {
    pw_sender: pipewire::channel::Sender<PipewireMessage>,
}

impl PipeWireStopHandler {
    pub fn stop(&self) {
        let _ = self.pw_sender.send(PipewireMessage::Stop);
    }
    pub fn get_frame_sender(&self) -> pipewire::channel::Sender<PipewireMessage> {
        self.pw_sender.clone()
    }
}
