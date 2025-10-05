use anyhow::Result;
use aoe4_overlay::consts::*;
use image::{GenericImageView, Rgb, RgbImage, buffer::ConvertBuffer, imageops::ColorMap};
use oar_ocr::{
    core::config::onnx::{OrtExecutionProvider, OrtSessionConfig},
    pipeline::OAROCRBuilder,
};
use opencv::{
    core::{self, Mat, Point, Rect, Scalar, Vector},
    imgcodecs::{self, IMREAD_COLOR},
    imgproc::{self, FONT_HERSHEY_SIMPLEX, LINE_8},
    prelude::*,
};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DetectedText {
    pub label: String,
    pub text: String,
    pub confidence: f32,
    pub bbox: Rect,
}

#[derive(Debug, Clone)]
pub struct NotificationBox {
    pub bbox: Rect,
    pub icon_area: Option<Rect>,
    pub text_area: Option<Rect>,
}

#[derive(Debug, Clone)]
pub struct DetectedIcon {
    pub name: String,
    pub bbox: Rect,
    pub confidence: f64,
}

/// Load the test image
fn load_test_image() -> Result<Mat> {
    let image_path = "src_images/menu_cut1.jpg";
    let img = imgcodecs::imread(image_path, IMREAD_COLOR)?;

    if img.empty() {
        anyhow::bail!("Failed to load image from {}", image_path);
    }

    log::info!("Loaded image: {}x{}", img.cols(), img.rows());
    Ok(img)
}

#[derive(Clone, Copy)]
pub struct BiLevelRGB;

impl ColorMap for BiLevelRGB {
    type Color = Rgb<u8>;

    #[inline(always)]
    fn index_of(&self, color: &Rgb<u8>) -> usize {
        let luma = color.0;
        if luma[0] > 127 && luma[1] > 127 && luma[2] > 127 {
            1
        } else {
            0
        }
    }

    #[inline(always)]
    fn lookup(&self, idx: usize) -> Option<Self::Color> {
        match idx {
            0 => Some([0, 0, 0].into()),
            1 => Some([255, 255, 255].into()),
            _ => None,
        }
    }

    /// Indicate `NeuQuant` implements `lookup`.
    fn has_lookup(&self) -> bool {
        true
    }

    #[inline(always)]
    fn map_color(&self, color: &mut Rgb<u8>) {
        let new_color = 0xFF * self.index_of(color) as u8;
        let luma = &mut color.0;
        luma[0] = new_color;
    }
}

/// Perform OCR on the entire image and return detected texts with their locations
fn extract_text_with_ocr(image_path: PathBuf) -> Result<Vec<DetectedText>> {
    let ort_config = OrtSessionConfig::new().with_execution_providers(vec![
        OrtExecutionProvider::CPU,
    ]);

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

    let mut img = image::open(image_path)?.into_rgb8();
    // Grayscale and Increase brightness
    let img = image::imageops::grayscale(&mut img);
    let mut img: RgbImage = img.convert();
    image::imageops::colorops::brighten_in_place(&mut img, 30);

    img.save("target/processed_image.jpg")?;

    let image_height = img.height() as f32;
    log::info!("OCR models and image loaded.");

    let start_timestamp = std::time::Instant::now();
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

    let ocr_results = ocr.predict(&subviews)?;
    let duration_for_all = std::time::Instant::now() - start_timestamp;

    let mut detected_texts = Vec::new();
    for stat_pos in AOE4_STATS_POS.iter() {
        let mut result = DetectedText {
            label: stat_pos.name.to_string(),
            text: "0".to_string(),
            confidence: 1.0, // Placeholder as confidence is not provided
            bbox: Rect {
                x: stat_pos.x as i32,
                y: stat_pos.y as i32,
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
        if !ocr1_result.is_empty() {
            result.text = ocr1_result;
            result.confidence = ocr_result.text_regions[0].confidence.unwrap_or(0.0);
        }
        detected_texts.push(result);
    }
    log::info!("Full time: {} ms", duration_for_all.as_millis(),);

    Ok(detected_texts)
}

/// Perform template matching to find the villager icon
fn detect_villager_icon(img: &Mat) -> Result<Vec<DetectedIcon>> {
    let mut detected_icons = Vec::new();

    // Load the villager icon template
    let template_path = "src_images/villager_icon.png";
    let template = imgcodecs::imread(template_path, IMREAD_COLOR)?;

    if template.empty() {
        anyhow::bail!("Failed to load template image from {}", template_path);
    }

    log::info!("Loaded template: {}x{}", template.cols(), template.rows());

    // Get image dimensions
    let img_height = img.rows() as f32;

    // Calculate the search area based on VILLAGER_ICON_AREA
    let search_x = VILLAGER_ICON_AREA.x as i32;
    let search_y = (img_height + AREA_Y_OFFSET) as i32 + VILLAGER_ICON_AREA.y as i32;
    let search_width = VILLAGER_ICON_AREA.width as i32;
    let search_height = VILLAGER_ICON_AREA.height as i32;

    // Ensure the search area is within image bounds
    let search_x = search_x.max(0);
    let search_y = search_y.max(0);
    let search_width = search_width.min(img.cols() - search_x);
    let search_height = search_height.min(img.rows() - search_y);

    if search_width <= 0 || search_height <= 0 {
        log::info!("Warning: Search area is outside image bounds");
        return Ok(detected_icons);
    }

    log::info!(
        "Search area: ({}, {}) {}x{}",
        search_x,
        search_y,
        search_width,
        search_height
    );

    // Extract the region of interest
    let roi = Mat::roi(
        img,
        Rect::new(search_x, search_y, search_width, search_height),
    )?;

    // Perform template matching with multiple methods for comparison
    let methods = vec![
        //        (imgproc::TM_CCOEFF_NORMED, "TM_CCOEFF_NORMED"),
        (imgproc::TM_CCORR_NORMED, "TM_CCORR_NORMED"),
        //(imgproc::TM_SQDIFF_NORMED, "TM_SQDIFF_NORMED"),
    ];

    let mut best_matches: Vec<(Point, f64, &str)> = Vec::new();

    for (method, method_name) in methods {
        let mut result = Mat::default();
        imgproc::match_template(&roi, &template, &mut result, method, &Mat::default())?;

        // Find the best match
        let mut min_val = 0.0;
        let mut max_val = 0.0;
        let mut min_loc = Point::default();
        let mut max_loc = Point::default();

        core::min_max_loc(
            &result,
            Some(&mut min_val),
            Some(&mut max_val),
            Some(&mut min_loc),
            Some(&mut max_loc),
            &Mat::default(),
        )?;

        // For TM_SQDIFF_NORMED, lower values are better matches
        let (match_loc, match_val) = if method == imgproc::TM_SQDIFF_NORMED {
            (min_loc, 1.0 - min_val) // Invert for consistency
        } else {
            (max_loc, max_val)
        };

        log::info!(
            "  Method {}: confidence = {:.4}, location = ({}, {})",
            method_name,
            match_val,
            match_loc.x,
            match_loc.y
        );

        best_matches.push((match_loc, match_val, method_name));
    }

    // Use the best match from TM_CCOEFF_NORMED (typically most reliable)
    if let Some((match_loc, confidence, method_name)) = best_matches.first() {
        let threshold = 0.6; // Confidence threshold

        if *confidence >= threshold {
            // Adjust coordinates back to full image space
            let icon_x = search_x + match_loc.x;
            let icon_y = search_y + match_loc.y;

            detected_icons.push(DetectedIcon {
                name: "Villager".to_string(),
                bbox: Rect::new(icon_x, icon_y, template.cols(), template.rows()),
                confidence: *confidence,
            });

            log::info!(
                "  Villager icon detected using {} with confidence {:.4}",
                method_name,
                confidence
            );
            log::info!("  Position in full image: ({}, {})", icon_x, icon_y);
        } else {
            log::warn!(
                "  No confident match found (best confidence: {:.4}, threshold: {})",
                confidence,
                threshold
            );
        }
    }

    Ok(detected_icons)
}

/// Draw detection results on the image
fn visualize_results(
    img: &Mat,
    texts: &[DetectedText],
    boxes: &[NotificationBox],
    icons: &[DetectedIcon],
) -> Result<Mat> {
    let mut output = img.clone();

    // Draw the search area for villager icon in yellow
    let img_height = img.rows() as f32;
    let search_x = VILLAGER_ICON_AREA.x as i32;
    let search_y = (img_height + AREA_Y_OFFSET) as i32 + VILLAGER_ICON_AREA.y as i32;
    let search_rect = Rect::new(
        search_x,
        search_y,
        VILLAGER_ICON_AREA.width as i32,
        VILLAGER_ICON_AREA.height as i32,
    );

    imgproc::rectangle(
        &mut output,
        search_rect,
        Scalar::new(0.0, 255.0, 255.0, 0.0), // Yellow
        1,
        LINE_8,
        0,
    )?;

    imgproc::put_text(
        &mut output,
        "Search Area",
        Point::new(search_x, search_y - 5),
        FONT_HERSHEY_SIMPLEX,
        0.5,
        Scalar::new(0.0, 255.0, 255.0, 0.0),
        1,
        LINE_8,
        false,
    )?;

    // Draw notification boxes in green
    for notification_box in boxes {
        imgproc::rectangle(
            &mut output,
            notification_box.bbox,
            Scalar::new(0.0, 255.0, 0.0, 0.0),
            2,
            LINE_8,
            0,
        )?;

        // Add label
        let label = format!(
            "Box: {}x{}",
            notification_box.bbox.width, notification_box.bbox.height
        );
        imgproc::put_text(
            &mut output,
            &label,
            Point::new(notification_box.bbox.x, notification_box.bbox.y - 5),
            FONT_HERSHEY_SIMPLEX,
            0.5,
            Scalar::new(0.0, 255.0, 0.0, 0.0),
            1,
            LINE_8,
            false,
        )?;
    }

    // Draw detected icons in blue
    for icon in icons {
        imgproc::rectangle(
            &mut output,
            icon.bbox,
            Scalar::new(255.0, 0.0, 0.0, 0.0), // Blue
            2,
            LINE_8,
            0,
        )?;

        let label = format!("{}: {:.2}", icon.name, icon.confidence);
        imgproc::put_text(
            &mut output,
            &label,
            Point::new(icon.bbox.x, icon.bbox.y - 5),
            FONT_HERSHEY_SIMPLEX,
            0.6,
            Scalar::new(255.0, 0.0, 0.0, 0.0),
            2,
            LINE_8,
            false,
        )?;
    }

    // Draw text bounding boxes in red
    for detected_text in texts {
        imgproc::rectangle(
            &mut output,
            detected_text.bbox,
            Scalar::new(0.0, 0.0, 255.0, 0.0),
            1,
            LINE_8,
            0,
        )?;

        // Add text label
        imgproc::put_text(
            &mut output,
            &detected_text.text,
            Point::new(detected_text.bbox.x, detected_text.bbox.y - 3),
            FONT_HERSHEY_SIMPLEX,
            0.4,
            Scalar::new(0.0, 0.0, 255.0, 0.0),
            1,
            LINE_8,
            false,
        )?;
    }

    Ok(output)
}

#[test]
fn test_image_analysis() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default())
        .filter(None, log::LevelFilter::Info)
        .format_timestamp_millis()
        .init();
    log::info!("========================================");
    log::info!("Starting Image Analysis Test");
    log::info!("========================================\n");

    // Load the test image
    let img = load_test_image()?;

    // Extract text using OCR
    log::info!("--- Running OCR ---");
    let detected_texts = extract_text_with_ocr("src_images/menu_cut1.jpg".parse()?)?;
    log::info!("Total texts detected: {}", detected_texts.len());

    // Detect notification boxes
    let notification_boxes = Vec::new();

    // Perform villager icon detection
    log::info!("--- Detecting Villager Icon ---");
    let detected_icons = detect_villager_icon(&img)?;
    log::info!("Total icons detected: {}", detected_icons.len());

    // Visualize results
    log::info!("--- Creating Visualization ---");
    let output = visualize_results(&img, &detected_texts, &notification_boxes, &detected_icons)?;

    // Save the output image
    let output_path = "target/image_analysis_result.jpg";
    imgcodecs::imwrite(output_path, &output, &Vector::default())?;
    log::info!("Saved visualization to: {}", output_path);

    log::info!("--- Analysis Summary ---");
    log::info!("Image size: {}x{}", img.cols(), img.rows());
    log::info!("Detected texts: {}", detected_texts.len());
    log::info!("Notification boxes: {}", notification_boxes.len());
    log::info!("Detected icons: {}", detected_icons.len());
    log::info!("Detected text items:");
    for (i, text) in detected_texts.iter().enumerate() {
        log::info!("  {}. '{}'", text.label, text.text);
    }
    log::info!("Notification box locations:");
    for (i, bbox) in notification_boxes.iter().enumerate() {
        log::info!(
            "  {}. Position: ({}, {}) Size: {}x{}",
            i + 1,
            bbox.bbox.x,
            bbox.bbox.y,
            bbox.bbox.width,
            bbox.bbox.height
        );
    }
    log::info!("Detected icon locations:");
    for (i, icon) in detected_icons.iter().enumerate() {
        log::info!(
            "  {}. {} at ({}, {}) confidence: {:.2}",
            i + 1,
            icon.name,
            icon.bbox.x,
            icon.bbox.y,
            icon.confidence
        );
    }

    log::info!("========================================\n");

    Ok(())
}
