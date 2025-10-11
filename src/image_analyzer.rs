use crate::consts::{AOE4_STATS_POS, AREA_Y_OFFSET, STAT_RECT, VILLAGER_ICON_AREA};
use anyhow::Result;
use image::{DynamicImage, GenericImageView, RgbImage};
use oar_ocr::{
    core::{
        StandardPredictor,
        config::{OrtExecutionProvider, OrtSessionConfig},
    },
    predictor::{TextRecPredictor, TextRecPredictorBuilder},
};
use opencv::{
    core::{self, AlgorithmHint, Mat, Point, Rect},
    imgcodecs::{self, IMREAD_COLOR},
    imgproc::{self},
    prelude::*,
};
use rayon::{
    iter::{IntoParallelRefMutIterator, ParallelIterator},
    prelude::IndexedParallelIterator,
};
use rust_paddle_ocr::Rec as PPRec;
use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

// #[derive(Debug, Clone, Default)]
// pub struct DetectedText {
//     pub text: String,
//     pub confidence: f32,
//     #[allow(dead_code)]
//     pub bbox: Rect,
//     #[allow(dead_code)]
//     pub stat_name: &'static str,
//     pub text_type: TextType,
// }

// #[derive(Debug, Clone, PartialEq)]
// pub enum IconType {
//     Villager,
// }

#[derive(Debug, Clone)]
pub struct AnalysisResult {
    pub detected_texts: [String; AOE4_STATS_POS.len()],
    pub has_villager_icon: bool,
    pub detect_villager_time: Duration,
    pub convert_color_time: Duration,
    pub ocr_time: Duration,
}

pub struct ImageAnalyzer {
    inner: Arc<Mutex<Option<ImageAnalyzerInner>>>,
}

impl ImageAnalyzer {
    pub fn new() -> Result<Self> {
        let inner = ImageAnalyzerInner::new()?;
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
    rec: PPRec,
    ocr_engine: Arc<TextRecPredictor>,
    villager_icon_template: Mat,
}

#[derive(Debug)]
pub enum OCRModel {
    PP,
    ONNX,
    OnnxPar,
}

impl ImageAnalyzerInner {
    pub fn new() -> Result<Self> {
        let ort_config = OrtSessionConfig::new().with_execution_providers(vec![
            OrtExecutionProvider::ROCm { device_id: None },
            //OrtExecutionProvider::CPU,
        ]);

        let rec = PPRec::from_file(
            "./models/PP-OCRv5_mobile_rec_fp16.mnn",
            "./models/ppocr_keys_v5.txt",
        )?
        .with_min_score(0.6)
        .with_punct_min_score(0.1);

        let predictor = TextRecPredictorBuilder::new()
            .model_input_shape([3, 48, 320]) // Model input shape for image resizing
            .batch_size(8) // Process 8 images at a time
            .session_pool_size(8)
            .character_dict(
                fs::read_to_string("models/numbers_only_dict.txt")?
                    .lines()
                    .map(|l| l.to_string())
                    .collect(),
            ) // Character dictionary for recognition
            .model_name("PP-OCRv5_mobile_rec".to_string()) // Model name
            .ort_session(ort_config) // Set device configuration
            .build(Path::new("models/latin_ppocrv5_mobile_rec.onnx"))?;

        // Load villager icon template
        let template_path = "src_images/villager_icon.png";
        let villager_icon_template = imgcodecs::imread(template_path, IMREAD_COLOR)?;

        if villager_icon_template.empty() {
            anyhow::bail!("Failed to load template image from {}", template_path);
        }

        Ok(Self {
            rec: rec,
            ocr_engine: Arc::new(predictor),
            villager_icon_template,
        })
    }

    pub fn analyze(&mut self, mut cv_mat: Mat, ocrmodel: OCRModel) -> Result<AnalysisResult> {
        let width = cv_mat.cols() as u32;
        let height = cv_mat.rows() as u32;
        // let stride = cv_mat.step1(0)? as i32;

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
        // img.save("target/processed_image.jpg")?;

        let convert_color_time = now.elapsed() - detect_villager_time;

        // Perform OCR
        let detected_texts = match ocrmodel {
            OCRModel::PP => self.extract_text_with_ocr_pp(img)?,
            OCRModel::ONNX => self.extract_text_with_ocr(img)?,
            OCRModel::OnnxPar => self.extract_text_with_ocr_par(img)?,
        };
        // let detected_texts = self.extract_text_with_tesseract(img)?;

        let ocr_time = now.elapsed() - convert_color_time - detect_villager_time;
        log::info!("OCR time: {:?}", ocr_time);

        Ok(AnalysisResult {
            detected_texts,
            has_villager_icon,
            detect_villager_time,
            convert_color_time,
            ocr_time,
        })
    }

    fn extract_text_with_tesseract(&self, img: RgbImage) -> Result<[String; AOE4_STATS_POS.len()]> {
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

        let mut buf = Vec::new();

        let mut detected_texts: [String; AOE4_STATS_POS.len()] = Default::default();
        for i in 0..detected_texts.len() {
            let stat_pos = &AOE4_STATS_POS[i];
            buf.clear();
            let writer = &mut std::io::Cursor::new(&mut buf);
            subviews[i].write_to(writer, image::ImageFormat::Tiff)?;
            let tess = tesseract::Tesseract::new(None, Some("eng"))?;
            let mut tess = tess
                .set_variable("tessedit_char_whitelist", "0123456789/")?
                .set_image_from_mem(&buf)?
                .set_source_resolution(70);
            let ocr_result = tess.get_text()?;
            if ocr_result.len() == 0 {
                detected_texts[i] = String::new();
                continue;
            }

            // If not OCR result is a number, set to "0"
            if !ocr_result.is_empty() && ocr_result.chars().all(|c| c.is_ascii_digit() || c == '/')
            {
                detected_texts[i] = ocr_result.trim().to_string();
            };
        }

        Ok(detected_texts)
    }

    fn extract_text_with_ocr_pp(
        &mut self,
        img: RgbImage,
    ) -> Result<[String; AOE4_STATS_POS.len()]> {
        let image_height = img.height() as f32;
        //let now = std::time::Instant::now();

        let subviews = AOE4_STATS_POS
            .iter()
            .map(|stat_pos| {
                let y = image_height + stat_pos.y;
                DynamicImage::ImageRgb8(
                    img.view(
                        stat_pos.x as u32,
                        y as u32,
                        STAT_RECT.width,
                        STAT_RECT.height,
                    )
                    .to_image(),
                )
            })
            .collect::<Vec<_>>();

        // let ocr_results = self.ocr_engine.predict(subviews, None)?;
        //let ocr_time = now.elapsed() - subview_time;

        let mut detected_texts: [String; AOE4_STATS_POS.len()] = Default::default();
        (0..detected_texts.len()).into_iter().for_each(|i| {
            let (text, confidence) = self.rec.predict_with_confidence(&subviews[i]).unwrap();
            if text.len() == 0 {
                return;
            }

            // If not OCR result is a number, set to "0"
            if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit() || c == '/') {
                if confidence > 0.5 {
                    detected_texts[i] = text;
                }
            };
        });

        // let parse_time = now.elapsed() - subview_time - ocr_time;
        // log::info!(
        //     "Subview extraction time: {:?}, OCR time: {:?}, Parse time: {:?}",
        //     subview_time,
        //     ocr_time,
        //     parse_time
        // );

        Ok(detected_texts)
    }

    fn extract_text_with_ocr(&self, img: RgbImage) -> Result<[String; AOE4_STATS_POS.len()]> {
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

        let ocr_results = self.ocr_engine.predict(subviews, None)?;

        let mut detected_texts: [String; AOE4_STATS_POS.len()] = Default::default();
        for i in 0..detected_texts.len() {
            let ocr_result = ocr_results.rec_text[i].clone();
            if ocr_result.len() == 0 {
                detected_texts[i] = String::new();
                continue;
            }

            // let ocr1_result = ocr_result.to_string();
            // If not OCR result is a number, set to "0"
            if !ocr_result.is_empty() && ocr_result.chars().all(|c| c.is_ascii_digit() || c == '/')
            {
                detected_texts[i] = ocr_result.to_string();
                // result.text = ocr1_result;
                // result.confidence = ocr_results.rec_score[i];
            };
        }

        Ok(detected_texts)
    }

    fn extract_text_with_ocr_par(&self, img: RgbImage) -> Result<[String; AOE4_STATS_POS.len()]> {
        let image_height = img.height() as f32;
        let now = std::time::Instant::now();

        let mut detected_texts: [String; AOE4_STATS_POS.len()] = Default::default();

        let engine = self.ocr_engine.clone();
        detected_texts
            .par_iter_mut()
            .zip(&AOE4_STATS_POS)
            .for_each(|(mut entry, stat_pos)| {
                let y = image_height + stat_pos.y;
                let subview = img
                    .view(
                        stat_pos.x as u32,
                        y as u32,
                        STAT_RECT.width,
                        STAT_RECT.height,
                    )
                    .to_image();

                let ocr_results = engine.predict(vec![subview], None);
                if ocr_results.is_err() {
                    entry.clear();
                    return;
                }
                let ocr_results = ocr_results.unwrap();

                let mut ocr1_result = ocr_results.rec_text[0].to_string();
                // If not OCR result is a number, set to "0"
                if !ocr1_result.is_empty()
                    && ocr1_result.chars().all(|c| c.is_ascii_digit() || c == '/')
                {
                    if ocr_results.rec_score[0] > 0.5 {
                        entry = &mut ocr1_result;
                    }
                    //entry = ocr1_result;
                    //entry.confidence = ocr_results.rec_score[0];
                };
            });

        let parse_time = now.elapsed();
        log::info!("OCR time: {:?}", parse_time);

        Ok(detected_texts)
    }

    /// Detect villager icon using template matching
    ///
    /// # Arguments
    ///
    /// * `img`: &Mat - Input image in BGR format
    ///
    /// returns: Result<Vec<DetectedIcon, Global>, Error>
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
            imgproc::TM_CCOEFF_NORMED, // TM_CCORR_NORMED,
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
        // if max_val >= threshold {
        //     let icon_x = search_x + max_loc.x;
        //     let icon_y = search_y + max_loc.y;
        //
        //     detected_icons.push(DetectedIcon {
        //         icon_type: IconType::Villager,
        //         name: "Villager",
        //         bbox: Rect::new(
        //             icon_x,
        //             icon_y,
        //             self.villager_icon_template.cols(),
        //             self.villager_icon_template.rows(),
        //         ),
        //         confidence: max_val,
        //     });
        // }
        //
        // Ok(detected_icons)
    }
}
