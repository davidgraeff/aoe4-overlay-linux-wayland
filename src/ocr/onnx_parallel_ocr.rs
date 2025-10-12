// ONNX-based OCR implementation with parallel processing

use super::OcrEngine;
use anyhow::Result;
use image::{GenericImageView, RgbImage};
use oar_ocr::{
    core::{
        config::{OrtExecutionProvider, OrtSessionConfig},
    },
    predictor::{TextRecPredictor, TextRecPredictorBuilder},
};
use rayon::prelude::*;
use std::{fs, path::Path, sync::Arc};
use oar_ocr::core::StandardPredictor;

/// ONNX Runtime-based text recognition engine with parallel processing
pub struct OnnxParallelOcrEngine {
    predictor: Arc<TextRecPredictor>,
}

impl OnnxParallelOcrEngine {
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

impl OcrEngine for OnnxParallelOcrEngine {
    fn recognize_text<const N: usize>(
        &mut self,
        img: &RgbImage,
        regions: &[(u32, u32, u32, u32)],
    ) -> Result<[fixedstr::str8; N]> {
        let mut detected_texts: [fixedstr::str8; N] = [fixedstr::str8::new(); N];

        let predictor = self.predictor.clone();

        detected_texts
            .par_iter_mut()
            .zip(regions.par_iter())
            .for_each(|(entry, (x, y, width, height))| {
                let subview = img.view(*x, *y, *width, *height).to_image();

                let ocr_results = predictor.predict(vec![subview], None);
                if let Ok(results) = ocr_results {
                    let ocr_result = &results.rec_text[0];

                    // Only accept numeric results with '/' character and good confidence
                    if !ocr_result.is_empty()
                        && ocr_result.chars().all(|c| c.is_ascii_digit() || c == '/')
                        && results.rec_score[0] > 0.5
                    {
                        *entry = ocr_result.as_str().into();
                    }
                }
            });

        Ok(detected_texts)
    }
}

