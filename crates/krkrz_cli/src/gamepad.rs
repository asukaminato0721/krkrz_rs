use krkrz_runtime::InputEvent;

pub struct Delivery {
    pub window: usize,
    pub event: InputEvent,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
#[path = "gamepad/native.rs"]
mod platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform {
    use super::Delivery;

    pub struct Gamepads;
    impl Gamepads {
        pub fn new() -> Self {
            eprintln!("Gamepad input is not available on this platform.");
            Self
        }
        pub fn poll(&mut self, _target: Option<usize>, _now: u64) -> Vec<Delivery> {
            Vec::new()
        }
        pub fn release_window(&mut self, _window: usize) -> Vec<Delivery> {
            Vec::new()
        }
    }
}

pub use platform::Gamepads;
