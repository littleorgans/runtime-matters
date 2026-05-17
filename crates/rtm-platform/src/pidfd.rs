#[cfg(target_os = "linux")]
pub fn enabled() -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn enabled() -> bool {
    false
}
