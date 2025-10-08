use crate::{
    image_analyzer::{AnalysisResult, ImageAnalyzer},
    overlay_window_gtk::PixbufWrapper,
};
use anyhow::{anyhow, Result};
use aoe4_overlay::consts::{AREA_HEIGHT, AREA_WIDTH};
use log::{debug, error, info};
use opencv::core::{Mat, MatTraitConst, Rect};
use crate::image_analyzer::OCRModel;

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
        let analyzer = ImageAnalyzer::new()?;
        Ok(Self {
            analyzer,
        })
    }

    /// Start processing frames from input channel and send results to output channel
    pub fn start_processing(
        self,
        frame_rx: std::sync::mpsc::Receiver<PixbufWrapper>,
        processed_tx: std::sync::mpsc::SyncSender<ProcessedFrame>,
    )  -> Result<()>  {
        info!("Frame processor started");
        let mut analyzer = self.analyzer.into_inner().ok_or_else(|| anyhow!(""))?;

        let mut frame_count = 0u64;
        let mut processed_count = 0u64;
        let mut dropped_count = 0u64;

        while let Ok(frame) = frame_rx.recv() {
            if frame.bgr_buffer.is_empty() {
                info!("Received quit signal, stopping frame processor");
                break;
            }
            frame_count += 1;

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

            match analyzer.analyze(cv_mat, OCRModel::ONNX) {
                Ok(analysis) => {
                    processed_count += 1;

                    let processed_frame = ProcessedFrame {
                        original: frame,
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
                    if let Err(_) = processed_tx.try_send(processed_frame) {
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
