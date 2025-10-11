use std::sync::{Arc, Mutex};
use gdk::gdk_pixbuf::Pixbuf;
use gtk::gdk_pixbuf;

#[derive(Clone, Default)]
pub struct PixbufWrapper {
    pub bgr_buffer: Vec<u8>,
    pub width: i32,
    pub height: i32,
    pub stride: i32,
}

impl PixbufWrapper {
    pub fn copy_from_slice(&mut self, other: &[u8], width: i32, height: i32, stride: i32) {
        self.bgr_buffer.clear();
        self.bgr_buffer.extend_from_slice(other);
        self.width = width;
        self.height = height;
        self.stride = stride;
    }
    pub fn copy_from_pixbuf(&mut self, other: &PixbufWrapper) {
        self.bgr_buffer.clear();
        self.bgr_buffer.extend_from_slice(other.bgr_buffer.as_slice());
        self.width = other.width;
        self.height = other.height;
        self.stride = other.stride;
    }
}

#[derive(Clone, Default)]
pub struct PixelBufWrapperWithDroppedFrames {
    pub pixbuf: PixbufWrapper,
    pub frames_written: u32,
}

pub type PixelBufWrapperWithDroppedFramesTS = Arc<Mutex<PixelBufWrapperWithDroppedFrames>>;

impl PixbufWrapper {
    pub fn to_pixbuf(self) -> Pixbuf {
        Pixbuf::from_bytes(
            &gtk::glib::Bytes::from(&self.bgr_buffer),
            gdk_pixbuf::Colorspace::Rgb,
            true, // no alpha
            8,    // bits per sample
            self.width,
            self.height,
            self.stride,
        )
    }
}