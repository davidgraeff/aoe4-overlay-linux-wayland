use crate::{
    image_analyzer::{AnalysisResult, ImageAnalyzer, OCRModel},
    pixelbuf_wrapper::{PixbufWrapper, PixelBufWrapperWithDroppedFramesTS},
};
use anyhow::{Result, anyhow};
use aoe4_overlay::consts::{AREA_HEIGHT, AREA_WIDTH};
use log::{debug, error, info};
use opencv::core::{Mat, MatTraitConst, Rect};
use crate::overlay_window_gtk::GuiCommand;

/// Frame data with original image and analysis results
#[derive(Clone)]
pub struct ProcessedFrame {
    pub original: PixbufWrapper,
    pub analysis: AnalysisResult,
}

/// Frame processor that runs in a separate task
pub struct FrameProcessor {
    analyzer: ImageAnalyzer,
}

unsafe impl Send for FrameProcessor {}

impl FrameProcessor {
    pub fn new() -> Result<Self> {
        let analyzer = ImageAnalyzer::new(OCRModel::TemplateMatching)?;
        Ok(Self { analyzer })
    }

    /// Start processing frames from input channel and send results to output channel
    pub fn run(
        self,
        frame_rx: std::sync::mpsc::Receiver<bool>,
        frame_rx_content: PixelBufWrapperWithDroppedFramesTS,
        processed_tx: tokio::sync::mpsc::Sender<GuiCommand>,
    ) -> Result<()> {
        info!("Frame processor started");
        let mut analyzer = self.analyzer.into_inner().ok_or_else(|| anyhow!(""))?;

        let mut frame_count = 0u64;
        let mut processed_count = 0u64;
        let mut dropped_count = 0u32;
        let mut frame = PixbufWrapper::default();

        while let Ok(has_data) = frame_rx.recv() {
            if !has_data {
                info!("Received quit signal, stopping frame processor");
                break;
            }
            let mut content = frame_rx_content.lock().unwrap();
            if content.pixbuf.bgr_buffer.is_empty() || content.frames_written == 0 {
                debug!("No frame available, skipping");
                continue;
            }
            frame_count += 1;
            let dropped_frames = content.frames_written - 1;
            content.frames_written = 0; // reset counter
            frame.copy_from_pixbuf(&content.pixbuf);
            drop(content);

            dropped_count += dropped_frames;

            let cv_type = opencv::core::CV_MAKETYPE(8, 4);
            let r = unsafe {
                Mat::new_nd_with_data_unsafe(
                    &[frame.height, frame.width],
                    cv_type,
                    frame.bgr_buffer.as_ptr() as *mut _,
                    None,
                )
            };
            let cv_mat = match r {
                Ok(mat) => mat,
                Err(e) => {
                    error!("Failed to create Mat from frame data: {}", e);
                    continue;
                }
            };

            // Crop to the area of interest if needed
            let roi = Rect::new(0, frame.height - AREA_HEIGHT, AREA_WIDTH, AREA_HEIGHT);
            let cv_mat = Mat::roi(&cv_mat, roi).unwrap().try_clone()?;

            match analyzer.analyze(cv_mat) {
                Ok(analysis) => {
                    processed_count += 1;

                    let processed_frame = ProcessedFrame {
                        original: frame.clone(),
                        analysis,
                    };

                    if processed_count % 100 == 0 {
                        info!(
                            "Processed {} frames (received: {}, dropped: {}). Villager/Convert/OCR time: {}/{}/{}",
                            processed_count,
                            frame_count,
                            dropped_count,
                            processed_frame.analysis.detect_villager_time.as_millis(),
                            processed_frame.analysis.convert_color_time.as_millis(),
                            processed_frame.analysis.ocr_time.as_millis()
                        );
                    }
                    // Try to send, drop if channel is full
                    if let Err(_) = processed_tx.try_send(GuiCommand::ProcessedFrame(processed_frame)) {
                        dropped_count += 1;
                        debug!(
                            "Dropped processed frame (channel full). Total dropped: {}",
                            dropped_count
                        );
                    }
                }
                Err(e) => {
                    error!("Frame processing task error: {}", e);
                    return Err(e);
                }
            }
        }

        info!(
            "Frame processor stopped. Processed {} frames (received: {}, dropped: {})",
            processed_count, frame_count, dropped_count
        );

        Ok(())
    }
}
