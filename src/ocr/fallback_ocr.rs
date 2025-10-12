// Fallback OCR engine wrapper

use super::OcrEngine;
use anyhow::Result;
use image::RgbImage;
use crate::ocr::OcrEngineWrapper;

/// Wrapper that combines a primary OCR engine with a fallback
pub struct FallbackOcrEngine {
    primary: OcrEngineWrapper,
    fallback: OcrEngineWrapper,
    min_confidence_threshold: f64,
}

impl FallbackOcrEngine {
    pub fn new(
        primary: OcrEngineWrapper,
        fallback: OcrEngineWrapper,
        min_confidence_threshold: f64,
    ) -> Self {
        Self {
            primary,
            fallback,
            min_confidence_threshold,
        }
    }
}

impl OcrEngine for FallbackOcrEngine {
    fn recognize_text<const N: usize>(
        &mut self,
        img: &RgbImage,
        regions: &[(u32, u32, u32, u32)],
    ) -> Result<[String; N]> {
        // Try primary engine first
        let primary_results = self.primary.recognize_text::<N>(img, regions)?;

        // Check which regions need fallback
        let mut final_results = primary_results.clone();
        let mut needs_fallback = Vec::new();

        for (i, text) in primary_results.iter().enumerate() {
            if text.is_empty() {
                needs_fallback.push(i);
            }
        }

        // If some regions failed, try fallback for those specific regions
        if !needs_fallback.is_empty() {
            log::debug!("Using fallback OCR for {} regions", needs_fallback.len());
            let fallback_results = self.fallback.recognize_text::<N>(img, regions)?;

            for i in needs_fallback {
                if !fallback_results[i].is_empty() {
                    final_results[i] = fallback_results[i].clone();
                    log::debug!("Fallback succeeded for region {}: '{}'", i, final_results[i]);
                }
            }
        }

        Ok(final_results)
    }
}

