use std::path::PathBuf;
use log::info;

pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").unwrap_or_default() == "wayland"
}

pub fn data_directory() -> anyhow::Result<PathBuf> {
    let mut app_data_dir = PathBuf::new();
    if !app_data_dir.join("src_images").exists() {
        use std::fs::read_link;
        let base_path = read_link("/proc/self/exe")?;
        app_data_dir = std::fs::canonicalize(
            base_path
                .parent()
                .ok_or_else(||anyhow::anyhow!("/proc/self/exe gave strange result"))?
                .join("../.."),
        )?;
        info!("data dir \"{}\"", app_data_dir.display());
    }
    if !app_data_dir.exists() {
        anyhow::bail!(
            "Data directory with src_images and models not found at {}",
            app_data_dir.to_string_lossy()
        );
    }
    Ok(app_data_dir)
}
