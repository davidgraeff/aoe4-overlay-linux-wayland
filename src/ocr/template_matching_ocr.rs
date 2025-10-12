// Template matching-based OCR implementation for fast digit recognition

use super::{OcrEngine, onnx_ocr};
use anyhow::Result;
use image::{GenericImageView, RgbImage};
use include_directory::{Dir, include_directory};
use opencv::{
    core::Mat,
    imgcodecs::{self, IMREAD_GRAYSCALE},
    imgproc::{self},
    prelude::*,
};
use std::collections::HashMap;

/// Configuration for template matching OCR
#[derive(Debug, Clone)]
pub struct TemplateMatchingConfig {
    pub match_threshold: f64,
    pub min_confidence: f64,
}

impl Default for TemplateMatchingConfig {
    fn default() -> Self {
        Self {
            match_threshold: 0.7,
            min_confidence: 0.75,
        }
    }
}

static PROJECT_DIR: Dir<'_> = include_directory!("$CARGO_MANIFEST_DIR/src_images/digits");

/// Template matching-based OCR engine for fast digit recognition
pub struct TemplateMatchingOcrEngine {
    digit_templates: HashMap<char, Vec<Mat>>,
    config: TemplateMatchingConfig,
    fallback_engine: Option<onnx_ocr::OnnxOcrEngine>,
}

#[derive(Debug, Clone)]
struct DigitMatch {
    digit: char,
    x: i32,
    confidence: f64,
}

impl TemplateMatchingOcrEngine {
    /// Create a new template matching OCR engine
    pub fn new(config: TemplateMatchingConfig) -> Result<Self> {
        let digit_templates = Self::load_templates()?;

        Ok(Self {
            digit_templates,
            config,
            fallback_engine: None,
        })
    }

    /// Create with a fallback OCR engine
    pub fn with_fallback(
        config: TemplateMatchingConfig,
        fallback: onnx_ocr::OnnxOcrEngine,
    ) -> Result<Self> {
        let mut engine = Self::new(config)?;
        engine.fallback_engine = Some(fallback);
        Ok(engine)
    }

    /// Load digit templates from directory
    fn load_templates() -> Result<HashMap<char, Vec<Mat>>> {
        let mut templates: HashMap<char, Vec<Mat>> = HashMap::new();

        for file in PROJECT_DIR.entries() {
            let file_path = file.path();
            let file_name = file_path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if file_name == "slash" {
                let data = file.as_file().unwrap().contents();
                let mat = imgcodecs::imdecode(&Mat::from_slice(data)?, IMREAD_GRAYSCALE)?;
                if !mat.empty() {
                    templates.entry('/').or_default().push(mat);
                    //log::info!("Loaded template for '/'");
                }
                continue;
            }

            let (digit_char, _variant) = if let Some((d, v)) = file_name.split_once('-') {
                (d, v)
            } else {
                log::warn!("Ignoring file: '{}'", file_path.display());
                continue;
            };
            let data = file.as_file().unwrap().contents();
            let mat = imgcodecs::imdecode(&Mat::from_slice(data)?, IMREAD_GRAYSCALE)?;
            if !mat.empty() {
                templates
                    .entry(digit_char.chars().next().unwrap())
                    .or_default()
                    .push(mat);
                //log::info!("Loaded template for digit '{}'", digit_char);
            }
        }

        if templates.is_empty() {
            anyhow::bail!("Some digit templates could not be loaded");
        }

        Ok(templates)
    }

    /// Recognize digits in a grayscale image region using template matching
    fn recognize_digits(&self, img: &Mat) -> Result<(fixedstr::str8, f64)> {
        let mut matches: Vec<DigitMatch> = Vec::new();

        // Try matching each digit template
        for (&digit, templates) in &self.digit_templates {
            for template in templates {
                let digit_matches = self.match_template(img, template, digit)?;
                matches.extend(digit_matches);
            }
        }

        // Sort matches by x-coordinate (left to right)
        matches.sort_by_key(|m| m.x);

        // Remove overlapping matches (keep highest confidence)
        let filtered_matches = self.filter_overlapping_matches(matches);

        if filtered_matches.is_empty() {
            return Ok((Default::default(), 0.0));
        }

        // Build the recognized number string
        let mut text: fixedstr::str8 = Default::default();
        if filtered_matches.len() > 8 {
            log::warn!(
                "Recognized {} digits, but maximum supported is 8. Truncating.",
                filtered_matches.len()
            );
        }
        let mut tmp = [0u8; 4];
        let max_len = filtered_matches.len().min(8);
        for i in 0..max_len {
            text.push(filtered_matches[i].digit.encode_utf8(&mut tmp));
        }

        // Calculate average confidence
        let avg_confidence = filtered_matches.iter().map(|m| m.confidence).sum::<f64>()
            / filtered_matches.len() as f64;

        Ok((text, avg_confidence))
    }

    /// Match a single template in the image
    fn match_template(&self, img: &Mat, template: &Mat, digit: char) -> Result<Vec<DigitMatch>> {
        let mut result = Mat::default();
        imgproc::match_template(
            img,
            template,
            &mut result,
            imgproc::TM_CCOEFF_NORMED,
            &Mat::default(),
        )?;

        let mut matches = Vec::new();

        // Find all matches above threshold
        for y in 0..result.rows() {
            for x in 0..result.cols() {
                let confidence = *result.at_2d::<f32>(y, x)?;

                if confidence as f64 >= self.config.match_threshold {
                    matches.push(DigitMatch {
                        digit,
                        x,
                        confidence: confidence as f64,
                    });
                }
            }
        }

        Ok(matches)
    }

    /// Filter overlapping matches, keeping only the one with highest confidence
    fn filter_overlapping_matches(&self, matches: Vec<DigitMatch>) -> Vec<DigitMatch> {
        if matches.is_empty() {
            return matches;
        }

        let mut filtered = Vec::new();
        let mut sorted_matches = matches;
        sorted_matches.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());

        for current in sorted_matches {
            let overlaps = filtered.iter().any(|existing: &DigitMatch| {
                let distance = (current.x - existing.x).abs();
                distance < 10 // Assume digits are at least 15 pixels apart
            });

            if !overlaps {
                filtered.push(current);
            }
        }

        // Re-sort by x position
        filtered.sort_by_key(|m| m.x);
        filtered
    }

    /// Convert RGB image to OpenCV Mat in grayscale
    fn rgb_to_gray_mat(
        &self,
        img: &RgbImage,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Mat> {
        let subview = img.view(x, y, width, height).to_image();

        // Convert to grayscale
        let gray_img = image::imageops::grayscale(&subview);

        // Create OpenCV Mat from grayscale data - using from_slice and reshape
        let data = gray_img.into_raw();
        let mat = Mat::from_slice(&data)?;
        let mat = mat.reshape(1, height as i32)?;
        let mat = mat.try_clone()?;

        Ok(mat)
    }
}

impl OcrEngine for TemplateMatchingOcrEngine {
    fn recognize_text<const N: usize>(
        &mut self,
        img: &RgbImage,
        regions: &[(u32, u32, u32, u32)],
    ) -> Result<[fixedstr::str8; N]> {
        let mut detected_texts: [fixedstr::str8; N] = [fixedstr::str8::new(); N];

        for (i, &(x, y, width, height)) in regions.iter().enumerate() {
            // Convert region to grayscale Mat
            let gray_mat = self.rgb_to_gray_mat(img, x, y, width, height)?;

            // Recognize digits using template matching
            let (text, confidence) = self.recognize_digits(&gray_mat)?;

            // Check if we should use fallback
            let should_use_fallback = text.is_empty()
                || confidence < self.config.min_confidence
                || !text.chars().all(|c| c.is_ascii_digit() || c == '/');

            if should_use_fallback && self.fallback_engine.is_some() {
                // We need to call fallback with just this region
                // For now, skip fallback in this implementation - can be enhanced later
                detected_texts[i] = Default::default();
            } else if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit() || c == '/') {
                detected_texts[i] = text.into();
                // log::debug!(
                //     "Region {}: detected '{}' with confidence {:.2}",
                //     i,
                //     detected_texts[i],
                //     confidence
                // );
            }
        }

        Ok(detected_texts)
    }
}
