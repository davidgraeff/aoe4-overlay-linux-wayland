// ONNX-based OCR implementation

use super::OcrEngine;
use anyhow::Result;
use image::{GenericImageView, RgbImage};
use oar_ocr::{
    core::{
        config::{OrtExecutionProvider, OrtSessionConfig},
    },
    predictor::{TextRecPredictor, TextRecPredictorBuilder},
};
use std::{fs, path::Path, sync::Arc};
use oar_ocr::core::StandardPredictor;

/// ONNX Runtime-based text recognition engine
pub struct OnnxOcrEngine {
    predictor: Arc<TextRecPredictor>,
}

impl OnnxOcrEngine {
    pub fn new() -> Result<Self> {
        let ort_config = OrtSessionConfig::new().with_execution_providers(vec![
            OrtExecutionProvider::ROCm { device_id: None },
            // OrtExecutionProvider::CPU,
        ]);

        let predictor = TextRecPredictorBuilder::new()
            .model_input_shape([3, 48, 320])
            .batch_size(8)
            .session_pool_size(8)
            .character_dict(
                fs::read_to_string("models/numbers_only_dict.txt")?
                    .lines()
                    .map(|l| l.to_string())
                    .collect(),
            )
            .model_name("PP-OCRv5_mobile_rec".to_string())
            .ort_session(ort_config)
            .build(Path::new("models/latin_ppocrv5_mobile_rec.onnx"))?;

        Ok(Self {
            predictor: Arc::new(predictor),
        })
    }
}

impl OcrEngine for OnnxOcrEngine {
    fn recognize_text<const N: usize>(
        &mut self,
        img: &RgbImage,
        regions: &[(u32, u32, u32, u32)],
    ) -> Result<[fixedstr::str8; N]> {
        let subviews = regions
            .iter()
            .map(|(x, y, width, height)| img.view(*x, *y, *width, *height).to_image())
            .collect::<Vec<_>>();

        let ocr_results = self.predictor.predict(subviews, None)?;

        let mut detected_texts: [fixedstr::str8; N] = [fixedstr::str8::new(); N];
        for i in 0..detected_texts.len() {
            let ocr_result = &ocr_results.rec_text[i];

            if ocr_result.is_empty() {
                continue;
            }

            // Only accept numeric results with '/' character
            if ocr_result.chars().all(|c| c.is_ascii_digit() || c == '/') {
                detected_texts[i] = ocr_result.as_str().into();
            }
        }

        Ok(detected_texts)
    }
}

