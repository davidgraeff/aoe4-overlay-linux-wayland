use anyhow::Result;
use aoe4_overlay::{
    consts::*,
    image_analyzer::{DetectedIcon, DetectedText, ImageAnalyzer},
};
use opencv::{
    core::{Mat, Point, Rect, Scalar, Vector},
    imgcodecs::{self, IMREAD_COLOR},
    imgproc::{self, FONT_HERSHEY_SIMPLEX, LINE_8},
    prelude::*,
};

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

/// Draw detection results on the image
fn visualize_results(img: &Mat, texts: &[DetectedText], icons: &[DetectedIcon]) -> Result<Mat> {
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

    // Draw text bounding boxes in green
    for detected_text in texts {
        imgproc::rectangle(
            &mut output,
            detected_text.bbox,
            Scalar::new(0.0, 255.0, 0.0, 0.0),
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
            Scalar::new(0.0, 255.0, 0.0, 0.0),
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

    // Create image analyzer
    println!("\n--- Initializing Image Analyzer ---");
    let analyzer = ImageAnalyzer::new()?;
    println!("Image analyzer initialized successfully");

    // Analyze the image
    println!("\n--- Analyzing Image (OCR + Template Matching) ---");
    let start_timestamp = std::time::Instant::now();
    let analysis_result = analyzer.analyze(img.clone())?;
    let duration_for_all = std::time::Instant::now() - start_timestamp;
    println!("Full time: {} ms", duration_for_all.as_millis(),);

    println!(
        "Total texts detected: {}",
        analysis_result.detected_texts.len()
    );
    println!(
        "Total icons detected: {}",
        analysis_result.detected_icons.len()
    );

    // Visualize results
    println!("\n--- Creating Visualization ---");
    let output = visualize_results(
        &img,
        &analysis_result.detected_texts,
        &analysis_result.detected_icons,
    )?;

    // Save the output image
    let output_path = "target/image_analysis_result.jpg";
    imgcodecs::imwrite(output_path, &output, &Vector::default())?;
    println!("Saved visualization to: {}", output_path);

    // Print summary
    println!("\n========================================");
    println!("Analysis Summary");
    println!("========================================");
    println!("Image size: {}x{}", img.cols(), img.rows());
    println!("Detected texts: {}", analysis_result.detected_texts.len());
    println!("Detected icons: {}", analysis_result.detected_icons.len());
    println!("\nDetected text items:");
    for text in analysis_result.detected_texts.iter() {
        println!("  {}. '{}'", text.stat_name, text.text);
    }
    println!("\nDetected icon locations:");
    for (i, icon) in analysis_result.detected_icons.iter().enumerate() {
        println!(
            "  {}. {} at ({}, {}) confidence: {:.2}",
            i + 1,
            icon.name,
            icon.bbox.x,
            icon.bbox.y,
            icon.confidence
        );
    }

    println!("\n========================================\n");

    Ok(())
}
