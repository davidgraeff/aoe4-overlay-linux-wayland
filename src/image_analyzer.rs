use crate::consts::{AOE4_STATS_POS, AREA_Y_OFFSET, STAT_RECT, VILLAGER_ICON_AREA};
use anyhow::Result;
use image::{GenericImageView, RgbImage};
use oar_ocr::{
    core::config::{OrtExecutionProvider, OrtSessionConfig},
    pipeline::{OAROCR, OAROCRBuilder},
};
use opencv::{
    core::{self, AlgorithmHint, Mat, Point, Rect},
    imgcodecs::{self, IMREAD_COLOR},
    imgproc::{self},
    prelude::*,
};
use std::{sync::Arc};

#[derive(Debug, Clone)]
pub struct DetectedText {
    pub text: String,
    pub confidence: f32,
    #[allow(dead_code)]
    pub bbox: Rect,
    #[allow(dead_code)]
    pub stat_name: &'static str,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct DetectedIcon {
    pub name: &'static str,
    pub bbox: Rect,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub detected_texts: Vec<DetectedText>,
    pub detected_icons: Vec<DetectedIcon>,
}

pub struct ImageAnalyzer {
    ocr_engine: Arc<OAROCR>,
    villager_icon_template: Mat,
}

impl ImageAnalyzer {
    pub fn new() -> Result<Self> {
        let ort_config =
            OrtSessionConfig::new().with_execution_providers(vec![OrtExecutionProvider::CPU]);

        // Build OCR pipeline with CUDA support
        let ocr = OAROCRBuilder::new(
            "models/ppocrv5_mobile_det.onnx".to_string(),
            "models/latin_ppocrv5_mobile_rec.onnx".to_string(),
            "models/numbers_only_dict.txt".to_string(),
        )
        .global_ort_session(ort_config) // Apply CUDA config to all components
        .text_det_box_threshold(0.2)
        .text_det_threshold(0.2)
        .build()?;

        // Load villager icon template
        let template_path = "src_images/villager_icon.png";
        let villager_icon_template = imgcodecs::imread(template_path, IMREAD_COLOR)?;

        if villager_icon_template.empty() {
            anyhow::bail!("Failed to load template image from {}", template_path);
        }

        Ok(Self {
            ocr_engine: Arc::new(ocr),
            villager_icon_template,
        })
    }

    pub fn analyze(&self, mut cv_mat: Mat) -> Result<AnalysisResult> {
        let width = cv_mat.cols() as u32;
        let height = cv_mat.rows() as u32;
        // let stride = cv_mat.step1(0)? as i32;

        // OpenCV Mat is in BGR format, convert to grayscale and then to RGB
        let mut rgb_mat = Mat::default();
        let detected_icons = if cv_mat.channels() == 4 {
            imgproc::cvt_color(
                &cv_mat,
                &mut rgb_mat,
                imgproc::COLOR_BGRA2BGR,
                0,
                AlgorithmHint::ALGO_HINT_DEFAULT,
            )?;
            self.detect_villager_icon(&rgb_mat)?
        } else {
            self.detect_villager_icon(&cv_mat)?
        };

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

        // Perform OCR
        let img = RgbImage::from_raw(width, height, rgb_buffer.to_vec()).unwrap();
        let detected_texts = self.extract_text_with_ocr(img)?;

        Ok(AnalysisResult {
            detected_texts,
            detected_icons,
        })
    }

    fn extract_text_with_ocr(&self, mut img: RgbImage) -> Result<Vec<DetectedText>> {
        image::imageops::colorops::brighten_in_place(&mut img, 30);

        // img.save("target/processed_image.jpg")?;

        let image_height = img.height() as f32;

        let subviews = AOE4_STATS_POS
            .iter()
            .map(|stat_pos| {
                let y = image_height + stat_pos.y;
                img.view(
                    stat_pos.x as u32,
                    y as u32,
                    STAT_RECT.width,
                    STAT_RECT.height,
                )
                .to_image()
            })
            .collect::<Vec<_>>();

        let ocr_results = self.ocr_engine.predict(&subviews)?;

        let mut detected_texts = Vec::new();
        for (i, stat_pos) in AOE4_STATS_POS.iter().enumerate() {
            let y = image_height + stat_pos.y;
            let mut result = DetectedText {
                stat_name: stat_pos.name,
                text: "0".to_string(),
                confidence: 0.0,
                bbox: Rect {
                    x: stat_pos.x as i32,
                    y: y as i32,
                    width: STAT_RECT.width as i32,
                    height: STAT_RECT.height as i32,
                },
            };
            let ocr_result = &ocr_results[i];
            if ocr_result.text_regions.len() == 0 {
                detected_texts.push(result);
                continue;
            }
            let first_region = ocr_result.text_regions[0].text.as_ref();
            if first_region.is_none() {
                detected_texts.push(result);
                continue;
            }

            let ocr1_result = first_region
                .and_then(|f| Some(f.clone()))
                .unwrap_or_default()
                .to_string();
            // If not OCR result is a number, set to "0"
            if !ocr1_result.is_empty() && ocr1_result.chars().all(|c| c.is_ascii_digit() || c == '/') {
                result.text = ocr1_result;
                result.confidence = ocr_result.text_regions[0].confidence.unwrap_or(0.0);
            };
            detected_texts.push(result);
        }

        Ok(detected_texts)
    }

    /// Detect villager icon using template matching
    ///
    /// # Arguments
    ///
    /// * `img`: &Mat - Input image in BGR format
    ///
    /// returns: Result<Vec<DetectedIcon, Global>, Error>
    fn detect_villager_icon(&self, img: &Mat) -> Result<Vec<DetectedIcon>> {
        let mut detected_icons = Vec::new();

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
            return Ok(detected_icons);
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
            &self.villager_icon_template,
            &mut result,
            imgproc::TM_CCORR_NORMED,
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
        if max_val >= threshold {
            let icon_x = search_x + max_loc.x;
            let icon_y = search_y + max_loc.y;

            detected_icons.push(DetectedIcon {
                name: "Villager",
                bbox: Rect::new(
                    icon_x,
                    icon_y,
                    self.villager_icon_template.cols(),
                    self.villager_icon_template.rows(),
                ),
                confidence: max_val,
            });
        }

        Ok(detected_icons)
    }
}
