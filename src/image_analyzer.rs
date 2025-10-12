use crate::consts::{AOE4_STATS_POS, AREA_Y_OFFSET, STAT_RECT, VILLAGER_ICON_AREA};
use crate::ocr::{
    OcrEngine,
    OcrEngineWrapper,
    // fallback_ocr::FallbackOcrEngine,
    onnx_ocr::OnnxOcrEngine,
    onnx_parallel_ocr::OnnxParallelOcrEngine,
    paddle_ocr::PaddleOcrEngine,
    template_matching_ocr::{TemplateMatchingOcrEngine, TemplateMatchingConfig},
};
use anyhow::Result;
use image::RgbImage;
use opencv::{
    core::{self, AlgorithmHint, Mat, Point, Rect},
    imgcodecs::{self, IMREAD_COLOR},
    imgproc::{self},
    prelude::*,
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};


#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub detected_texts: [fixedstr::str8; AOE4_STATS_POS.len()],
    pub has_villager_icon: bool,
    pub detect_villager_time: Duration,
    pub convert_color_time: Duration,
    pub ocr_time: Duration,
}

pub struct ImageAnalyzer {
    inner: Arc<Mutex<Option<ImageAnalyzerInner>>>,
}

impl ImageAnalyzer {
    pub fn new(ocrmodel: OCRModel) -> Result<Self> {
        let inner = ImageAnalyzerInner::new(ocrmodel)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Some(inner))),
        })
    }
    pub fn into_inner(self) -> Option<ImageAnalyzerInner> {
        Arc::try_unwrap(self.inner)
            .ok()
            .and_then(|mutex| mutex.into_inner().ok())
            .and_then(|opt| opt)
    }
}

pub struct ImageAnalyzerInner {
    ocr_engine: OcrEngineWrapper,
    villager_icon_template: Mat,
}

#[derive(Debug)]
pub enum OCRModel {
    #[allow(dead_code)]
    PP,
    #[allow(dead_code)]
    ONNX,
    #[allow(dead_code)]
    OnnxPar,
    #[allow(dead_code)]
    TemplateMatching,
    // TemplateMatchingWithFallback,
}

impl ImageAnalyzerInner {
    pub fn new(ocrmodel: OCRModel) -> Result<Self> {
        // Create OCR engine based on selected model
        let ocr_engine = match ocrmodel {
            OCRModel::PP => OcrEngineWrapper::Paddle(PaddleOcrEngine::new()?),
            OCRModel::ONNX => OcrEngineWrapper::Onnx(OnnxOcrEngine::new()?),
            OCRModel::OnnxPar => OcrEngineWrapper::OnnxParallel(OnnxParallelOcrEngine::new()?),
            OCRModel::TemplateMatching => {
                let config = TemplateMatchingConfig::default();
                OcrEngineWrapper::TemplateMatching(TemplateMatchingOcrEngine::new(config)?)
            }
            // OCRModel::TemplateMatchingWithFallback => {
            //     let config = TemplateMatchingConfig::default();
            //     let primary = OcrEngineWrapper::TemplateMatching(TemplateMatchingOcrEngine::new(config)?);
            //     let fallback = OcrEngineWrapper::Onnx(OnnxOcrEngine::new()?);
            //     OcrEngineWrapper::Fallback(FallbackOcrEngine::new(primary, fallback, 0.75))
            // }
        };

        // Load villager icon template
        let template_path = "src_images/villager_icon.png";
        let villager_icon_template = imgcodecs::imread(template_path, IMREAD_COLOR)?;

        if villager_icon_template.empty() {
            anyhow::bail!("Failed to load template image from {}", template_path);
        }

        Ok(Self {
            ocr_engine,
            villager_icon_template,
        })
    }

    pub fn analyze(&mut self, mut cv_mat: Mat) -> Result<AnalysisResult> {
        let width = cv_mat.cols() as u32;
        let height = cv_mat.rows() as u32;

        let now = std::time::Instant::now();

        // OpenCV Mat is in BGR format, convert to grayscale and then to RGB
        let mut rgb_mat = Mat::default();
        let has_villager_icon = if cv_mat.channels() == 4 {
            imgproc::cvt_color(
                &cv_mat,
                &mut rgb_mat,
                imgproc::COLOR_BGRA2BGR,
                0,
                AlgorithmHint::ALGO_HINT_DEFAULT,
            )?;
            self.detect_icon(&rgb_mat, &self.villager_icon_template)?
        } else {
            self.detect_icon(&cv_mat, &self.villager_icon_template)?
        };
        let detect_villager_time = now.elapsed();

        if cv_mat.channels() == 4 {
            imgproc::cvt_color(
                &cv_mat,
                &mut rgb_mat,
                imgproc::COLOR_BGRA2GRAY,
                0,
                AlgorithmHint::ALGO_HINT_DEFAULT,
            )?;
        } else {
            imgproc::cvt_color(
                &cv_mat,
                &mut rgb_mat,
                imgproc::COLOR_BGR2GRAY,
                0,
                AlgorithmHint::ALGO_HINT_DEFAULT,
            )?;
        }
        imgproc::cvt_color(
            &rgb_mat,
            &mut cv_mat,
            imgproc::COLOR_GRAY2RGB,
            0,
            AlgorithmHint::ALGO_HINT_DEFAULT,
        )?;

        let rgb_buffer = cv_mat.data_bytes()?;
        let mut img = RgbImage::from_raw(width, height, rgb_buffer.to_vec()).unwrap();
        image::imageops::colorops::brighten_in_place(&mut img, 30);

        let convert_color_time = now.elapsed() - detect_villager_time;

        // Prepare regions for OCR
        let image_height = img.height() as f32;
        let regions: Vec<(u32, u32, u32, u32)> = AOE4_STATS_POS
            .iter()
            .map(|stat_pos| {
                let y = (image_height + stat_pos.y) as u32;
                (stat_pos.x as u32, y, STAT_RECT.width, STAT_RECT.height)
            })
            .collect();

        // Perform OCR using the selected engine
        let detected_texts = self.ocr_engine.recognize_text::<{AOE4_STATS_POS.len()}>(&img, &regions)?;

        let ocr_time = now.elapsed() - convert_color_time - detect_villager_time;
        if ocr_time > Duration::from_millis(100) {
            log::warn!("OCR took too long: {:?}", ocr_time);
        }

        Ok(AnalysisResult {
            detected_texts,
            has_villager_icon,
            detect_villager_time,
            convert_color_time,
            ocr_time,
        })
    }

    /// Detect villager icon using template matching
    ///
    /// # Arguments
    ///
    /// * `img`: &Mat - Input image in BGR format
    ///
    /// returns: Result<bool, Error>
    fn detect_icon(&self, img: &Mat, detect_icon: &Mat) -> Result<bool> {
        let img_height = img.rows() as f32;

        // Calculate search area
        let search_x = VILLAGER_ICON_AREA.x as i32;
        let search_y = (img_height + AREA_Y_OFFSET) as i32 + VILLAGER_ICON_AREA.y as i32;
        let search_width = VILLAGER_ICON_AREA.width as i32;
        let search_height = VILLAGER_ICON_AREA.height as i32;

        // Ensure bounds
        let search_x = search_x.max(0);
        let search_y = search_y.max(0);
        let search_width = search_width.min(img.cols() - search_x);
        let search_height = search_height.min(img.rows() - search_y);

        if search_width <= 0 || search_height <= 0 {
            return Ok(false);
        }

        // Extract ROI
        let roi = Mat::roi(
            img,
            Rect::new(search_x, search_y, search_width, search_height),
        )?;

        // Perform template matching
        let mut result = Mat::default();
        imgproc::match_template(
            &roi,
            detect_icon,
            &mut result,
            imgproc::TM_CCOEFF_NORMED,
            &Mat::default(),
        )?;

        // Find best match
        let mut _min_val = 0.0;
        let mut max_val = 0.0;
        let mut _min_loc = Point::default();
        let mut max_loc = Point::default();

        core::min_max_loc(
            &result,
            Some(&mut _min_val),
            Some(&mut max_val),
            Some(&mut _min_loc),
            Some(&mut max_loc),
            &Mat::default(),
        )?;

        let threshold = 0.6;
        Ok(max_val >= threshold)
    }
}
