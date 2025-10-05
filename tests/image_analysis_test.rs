use anyhow::Result;
use aoe4_overlay::consts::*;
use image::GenericImageView;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use opencv::{
    core::{self, Mat, Point, Rect, Scalar, Vector},
    imgcodecs::{self, IMREAD_COLOR},
    imgproc::{self, FONT_HERSHEY_SIMPLEX, LINE_8},
    prelude::*,
};
use rten::Model;
use std::{path::PathBuf};

#[derive(Debug, Clone)]
pub struct DetectedText {
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

/// Load the test image
fn load_test_image() -> Result<Mat> {
    let image_path = "src_images/menu_cut1.jpg";
    let img = imgcodecs::imread(image_path, IMREAD_COLOR)?;

    if img.empty() {
        anyhow::bail!("Failed to load image from {}", image_path);
    }

    println!("Loaded image: {}x{}", img.cols(), img.rows());
    Ok(img)
}

fn file_path(path: &str) -> PathBuf {
    let mut abs_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    abs_path.push(path);
    abs_path
}

/// Perform OCR on the entire image and return detected texts with their locations
fn extract_text_with_ocr(image_path: PathBuf) -> Result<Vec<DetectedText>> {
    let mut detected_texts = Vec::new();

    let detection_model_path = file_path("models/text-detection.rten");
    let rec_model_path = file_path("models/text-recognition.rten");

    let detection_model = Model::load_file(detection_model_path)?;
    let recognition_model = Model::load_file(rec_model_path)?;

    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(detection_model),
        recognition_model: Some(recognition_model),
        ..Default::default()
    })?;

    let img = image::open(image_path)?.into_rgb8();
    let image_height = img.height() as f32;

    for mut stat_pos in AOE4_STATS_POS {
        stat_pos.y = image_height + stat_pos.y;
        let subview = img
            .view(
                stat_pos.x as u32,
                stat_pos.y as u32,
                STAT_RECT.width,
                STAT_RECT.height,
            )
            .to_image();
        let img_source = ImageSource::from_bytes(subview.as_raw(), subview.dimensions())?;
        let ocr_input = engine.prepare_input(img_source)?;

        let text = engine.get_text(&ocr_input)?;
        detected_texts.push(DetectedText {
            text: text,
            confidence: 1.0, // Placeholder as confidence is not provided
            bbox: Rect {
                x: stat_pos.x as i32,
                y: stat_pos.y as i32,
                width: STAT_RECT.width as i32,
                height: STAT_RECT.height as i32,
            },
        });
    }

    Ok(detected_texts)
}

/// Detect notification boxes using color-based detection
fn detect_notification_boxes(img: &Mat) -> Result<Vec<NotificationBox>> {
    let mut notification_boxes = Vec::new();

    // Convert to HSV for better color detection
    let mut hsv = Mat::default();
    imgproc::cvt_color(
        img,
        &mut hsv,
        imgproc::COLOR_BGR2HSV,
        0,
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )?;

    // Define color ranges for the dark blue/gray UI elements in AoE4
    // These values might need tuning based on the actual game UI colors
    let lower_blue = Scalar::new(90.0, 30.0, 30.0, 0.0);
    let upper_blue = Scalar::new(130.0, 255.0, 255.0, 0.0);

    let mut mask = Mat::default();
    core::in_range(&hsv, &lower_blue, &upper_blue, &mut mask)?;

    // Find contours
    let mut contours = Vector::<Vector<Point>>::new();
    imgproc::find_contours(
        &mask,
        &mut contours,
        imgproc::RETR_EXTERNAL,
        imgproc::CHAIN_APPROX_SIMPLE,
        Point::new(0, 0),
    )?;

    println!("\nFound {} contours", contours.len());

    // Filter contours by size to find notification boxes
    for i in 0..contours.len() {
        let contour = contours.get(i)?;
        let area = imgproc::contour_area(&contour, false)?;

        // Only consider contours with reasonable size (adjust thresholds as needed)
        if area > 500.0 && area < 50000.0 {
            let bbox = imgproc::bounding_rect(&contour)?;

            // Check aspect ratio to filter out non-rectangular shapes
            let aspect_ratio = bbox.width as f32 / bbox.height as f32;
            if aspect_ratio > 0.5 && aspect_ratio < 10.0 {
                println!(
                    "Notification box candidate: ({}, {}) {}x{} area: {:.0}",
                    bbox.x, bbox.y, bbox.width, bbox.height, area
                );

                notification_boxes.push(NotificationBox {
                    bbox,
                    icon_area: None,
                    text_area: None,
                });
            }
        }
    }

    Ok(notification_boxes)
}

/// Draw detection results on the image
fn visualize_results(img: &Mat, texts: &[DetectedText], boxes: &[NotificationBox]) -> Result<Mat> {
    let mut output = img.clone();

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
    println!("\n========================================");
    println!("Starting Image Analysis Test");
    println!("========================================\n");

    // Load the test image
    let img = load_test_image()?;

    // Extract text using OCR
    println!("\n--- Running OCR ---");
    let detected_texts = extract_text_with_ocr("src_images/menu_cut1.jpg".parse()?)?;
    println!("Total texts detected: {}", detected_texts.len());

    // Detect notification boxes
    println!("\n--- Detecting Notification Boxes ---");
    let notification_boxes = detect_notification_boxes(&img)?;
    println!(
        "Total notification boxes detected: {}",
        notification_boxes.len()
    );

    // Visualize results
    println!("\n--- Creating Visualization ---");
    let output = visualize_results(&img, &detected_texts, &notification_boxes)?;

    // Save the output image
    let output_path = "target/image_analysis_result.jpg";
    imgcodecs::imwrite(output_path, &output, &Vector::default())?;
    println!("Saved visualization to: {}", output_path);

    // Print summary
    println!("\n========================================");
    println!("Analysis Summary");
    println!("========================================");
    println!("Image size: {}x{}", img.cols(), img.rows());
    println!("Detected texts: {}", detected_texts.len());
    println!("Notification boxes: {}", notification_boxes.len());
    println!("\nDetected text items:");
    for (i, text) in detected_texts.iter().enumerate() {
        println!("  {}. '{}'", i + 1, text.text);
    }
    println!("\nNotification box locations:");
    for (i, bbox) in notification_boxes.iter().enumerate() {
        println!(
            "  {}. Position: ({}, {}) Size: {}x{}",
            i + 1,
            bbox.bbox.x,
            bbox.bbox.y,
            bbox.bbox.width,
            bbox.bbox.height
        );
    }

    println!("\n========================================\n");

    Ok(())
}
