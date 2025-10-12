// OCR Engine Trait and Implementations

use anyhow::Result;
use image::RgbImage;

pub mod paddle_ocr;
pub mod onnx_ocr;
pub mod onnx_parallel_ocr;
pub mod template_matching_ocr;
// pub mod fallback_ocr;

/// Trait for OCR engines that can recognize text from images
pub trait OcrEngine {
    /// Extract text from multiple regions of an image
    ///
    /// # Arguments
    ///
    /// * `img` - The input image in RGB format
    /// * `regions` - Array of image regions to process
    ///
    /// # Returns
    ///
    /// Array of detected text strings, one per region
    fn recognize_text<const N: usize>(
        &mut self,
        img: &RgbImage,
        regions: &[(u32, u32, u32, u32)], // (x, y, width, height)
    ) -> Result<[fixedstr::str8; N]>;
}

/// Wrapper enum for different OCR engine implementations
/// This allows using OCR engines polymorphically without dyn trait issues
pub enum OcrEngineWrapper {
    Paddle(paddle_ocr::PaddleOcrEngine),
    Onnx(onnx_ocr::OnnxOcrEngine),
    OnnxParallel(onnx_parallel_ocr::OnnxParallelOcrEngine),
    TemplateMatching(template_matching_ocr::TemplateMatchingOcrEngine),
    // Fallback(fallback_ocr::FallbackOcrEngine),
}

impl OcrEngine for OcrEngineWrapper {
    fn recognize_text<const N: usize>(
        &mut self,
        img: &RgbImage,
        regions: &[(u32, u32, u32, u32)],
    ) -> Result<[fixedstr::str8; N]> {
        match self {
            OcrEngineWrapper::Paddle(engine) => engine.recognize_text(img, regions),
            OcrEngineWrapper::Onnx(engine) => engine.recognize_text(img, regions),
            OcrEngineWrapper::OnnxParallel(engine) => engine.recognize_text(img, regions),
            OcrEngineWrapper::TemplateMatching(engine) => engine.recognize_text(img, regions),
            // OcrEngineWrapper::Fallback(engine) => engine.recognize_text(img, regions),
        }
    }
}
