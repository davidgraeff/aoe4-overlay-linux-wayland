// PaddleOCR implementation

use super::OcrEngine;
use anyhow::Result;
use image::{DynamicImage, GenericImageView, RgbImage};
use rust_paddle_ocr::Rec as PPRec;

/// PaddleOCR-based text recognition engine
pub struct PaddleOcrEngine {
    rec: PPRec,
}

impl PaddleOcrEngine {
    pub fn new() -> Result<Self> {
        let rec = PPRec::from_file(
            "./models/PP-OCRv5_mobile_rec_fp16.mnn",
            "./models/ppocr_keys_v5.txt",
        )?
        .with_min_score(0.6)
        .with_punct_min_score(0.1);

        Ok(Self { rec })
    }
}

impl OcrEngine for PaddleOcrEngine {
    fn recognize_text<const N: usize>(
        &mut self,
        img: &RgbImage,
        regions: &[(u32, u32, u32, u32)],
    ) -> Result<[fixedstr::str8; N]> {
        let subviews = regions
            .iter()
            .map(|(x, y, width, height)| {
                DynamicImage::ImageRgb8(img.view(*x, *y, *width, *height).to_image())
            })
            .collect::<Vec<_>>();

        let mut detected_texts: [fixedstr::str8; N] = [fixedstr::str8::new(); N];

        for (i, subview) in subviews.iter().enumerate() {
            let (text, confidence) = self.rec.predict_with_confidence(subview)?;

            if text.is_empty() {
                continue;
            }

            // Only accept numeric results with '/' character
            if text.chars().all(|c| c.is_ascii_digit() || c == '/') && confidence > 0.5 {
                detected_texts[i] = text.into();
            }
        }

        Ok(detected_texts)
    }
}

