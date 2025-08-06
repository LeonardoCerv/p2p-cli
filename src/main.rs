mod camera;
mod display;

use camera::CameraCapture;
use display::TerminalDisplay;
use anyhow::Result;
use std::time::{Duration, Instant};

const FPS: u64 = 30;

fn main() -> Result<()> {
    let mut camera = CameraCapture::new()?;
    let (w, h) = camera.dimensions();
    let mut display = TerminalDisplay::new(w, h);
    
    let frame_time = Duration::from_millis(1000 / FPS);
    
    loop {
        let start = Instant::now();
        
        match camera.get_frame() {
            Ok(pixels) => {
                if display.show_frame(pixels).is_err() {
                    continue;
                }
            }
            Err(_) => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        }
        
        if let Some(sleep_time) = frame_time.checked_sub(start.elapsed()) {
            std::thread::sleep(sleep_time);
        }
    }
}