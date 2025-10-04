pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default() == "wayland"
}