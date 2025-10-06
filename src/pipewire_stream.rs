use crate::overlay_window_gtk::PixbufWrapper;
use anyhow::Result;
use pipewire::{
    context::Context,
    main_loop::MainLoop,
    spa,
    spa::{
        pod::{ChoiceValue, serialize::PodSerializer},
        sys::{
            SPA_PARAM_EnumFormat, SPA_TYPE_OBJECT_Format,
        },
        utils,
        utils::{ChoiceEnum, ChoiceFlags, Direction},
    },
    stream::{Stream, StreamListener},
};
use spa::{
    param::{
        ParamType,
        format::{MediaSubtype, MediaType},
    },
    pod::{Object, Pod, Property, Value},
};
use std::sync::mpsc;

/// Manages a PipeWire stream for screen capturing and sends images via a channel.
pub struct PipeWireStream {
    pub(crate) main_loop: MainLoop,
    context: Context,
    stream: Option<Stream>,
    listener: Option<StreamListener<()>>,
    image_sender: mpsc::SyncSender<PixbufWrapper>,
}

impl PipeWireStream {
    /// Creates a new PipeWireStream instance.
    pub fn new(image_sender: mpsc::SyncSender<PixbufWrapper>) -> Result<Self> {
        pipewire::init();

        let main_loop = MainLoop::new(None)?;
        let context = Context::new(&main_loop)?;

        Ok(Self {
            main_loop,
            context,
            stream: None,
            listener: None,
            image_sender,
        })
    }

    pub fn connect_to_node(&mut self, node_id: u32) -> Result<()> {
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

        // Set up stream listener
        let listener = stream
            .add_local_listener()
            .state_changed(|_stream, _user_data: &mut (), old_state, new_state| {
                log::info!(
                    "Stream state changed from {:?} to {:?}",
                    old_state,
                    new_state
                );
            })
            .param_changed(|_stream, _user_data, id, param| {
                if let Some(_param) = param {
                    if id == ParamType::Format.as_raw() {
                        log::info!("Stream format changed");
                    } else if id == ParamType::Latency.as_raw() {
                        log::info!("Stream latency params changed");
                    } else if id == ParamType::Props.as_raw() {
                        log::info!("Stream props changed");
                    } else {
                        log::info!("Stream unknown params changed");
                        // let mut media_type: u32 = 0;
                        // let mut media_subtype: u32 = 0;
                        // let mut uninit: ::std::mem::MaybeUninit<spa_video_info_raw> =
                        //     ::std::mem::MaybeUninit::uninit();
                        // let video_info = uninit.as_mut_ptr();
                        // unsafe {
                        //     spa_format_parse(
                        //         param.as_raw_ptr(),
                        //         &mut media_type,
                        //         &mut media_subtype,
                        //     );
                        //     if !spa_format_video_raw_parse(param.as_raw_ptr(), video_info) {
                        //         println!("Stream unknown param changed: {} {:?}", id,
                        // *video_info);     } else {
                        //         println!("Stream unknown param changed: {} (non-video)", id);
                        //     }
                        // }
                    }
                }
            })
            .process(move |stream, _user_data| {
                let mut buffer = match stream.dequeue_buffer() {
                    None => {
                        log::error!("Failed to dequeue buffer");
                        return;
                    }
                    Some(buffer) => buffer,
                };

                let data = buffer.datas_mut();
                if data.is_empty() {
                    return;
                }
                let data = &mut data[0];
                let chunk = data.chunk();
                let stride = chunk.stride();
                let size = chunk.size() as usize;
                // log::info!("Buffer received, size: {}, stride: {}", size, stride);

                if data.data().is_none() {
                    return;
                }
                let slice = data.data().unwrap();
                let width = stride / 4; // For BGRx, 4 bytes per pixel
                let height = slice.len() as i32 / stride;

                // log::info!("Buffer received, dimensions: {}x{}", width, height);

                if width <= 0 || height <= 0 || size <= 0 || slice.len() < size {
                    log::error!("Invalid image dimensions: {}x{}", width, height);
                    return;
                }

                let pixbuf_wrapper = PixbufWrapper {
                    bgr_buffer: Vec::from(&slice[..size]),
                    width,
                    height,
                    stride,
                };

                if let Err(e) = sender.try_send(pixbuf_wrapper) {
                    log::error!("Pipeline thread: Buffer full: {}", e);
                }
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
                                // utils::Id(spa::param::video::VideoFormat::BGR.as_raw()),
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

        // Connect stream to the node
        stream.connect(
            Direction::Input,
            Some(node_id),
            pipewire::stream::StreamFlags::AUTOCONNECT | pipewire::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        stream.set_active(true)?;

        self.stream = Some(stream);
        self.listener = Some(listener);
        Ok(())
    }
}

impl Drop for PipeWireStream {
    fn drop(&mut self) {
        if let Some(stream) = &self.stream {
            let _ = stream.disconnect();
        }
    }
}
