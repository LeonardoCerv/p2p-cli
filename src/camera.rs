use nokhwa::{
    pixel_format::RgbFormat,
    utils::{CameraIndex, RequestedFormat, RequestedFormatType, Resolution, FrameFormat, CameraFormat},
    Camera
};
use anyhow::Result;

#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

pub struct CameraCapture {
    camera: Camera,
    buffer: Vec<u8>,
    frame_skip_counter: u32,
    last_successful_frame: Option<Vec<u8>>,
}

impl CameraCapture {
    pub fn new() -> Result<Self> {
        // Initialize COM on Windows for MediaFoundation
        #[cfg(windows)]
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }

        let formats = vec![
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(320, 240),
                FrameFormat::YUYV,
                15
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(320, 240),
                FrameFormat::RAWRGB,
                15
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(640, 480),
                FrameFormat::YUYV,
                15
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(320, 240),
                FrameFormat::MJPEG,
                15
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::HighestResolution(Resolution::new(640, 480))),

            RequestedFormat::new::<RgbFormat>(RequestedFormatType::HighestResolution(Resolution::new(160, 120))),
        ];
        
        for (format_idx, format) in formats.iter().enumerate() {
            println!("Trying camera format {}: {:?}", format_idx, format);
            
            for camera_index in [0, 1] {
                match Self::try_create_camera(camera_index, format.clone()) {
                    Ok(camera_capture) => {
                        println!("Successfully initialized camera {} with format {}", camera_index, format_idx);
                        return Ok(camera_capture);
                    }
                    Err(e) => {
                        eprintln!("Failed to initialize camera {} with format {}: {}", camera_index, format_idx, e);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
        }
        
        Err(anyhow::anyhow!("Failed to initialize any camera with any format. This might be due to camera permissions or hardware resource limitations on Windows. Try:\n1. Running as administrator\n2. Checking camera permissions in Windows Settings\n3. Ensuring no other application is using the camera\n4. Restarting the computer to free up hardware resources"))
    }
    
    fn try_create_camera(camera_index: u32, format: RequestedFormat) -> Result<Self> {
        std::thread::sleep(std::time::Duration::from_millis(50));
        
        let mut camera = Camera::new(CameraIndex::Index(camera_index), format)?;
        
        let mut attempts = 0;
        let max_attempts = 5;
        
        while attempts < max_attempts {
            match camera.open_stream() {
                Ok(_) => break,
                Err(e) => {
                    attempts += 1;
                    if attempts >= max_attempts {
                        return Err(anyhow::anyhow!("Failed to open camera stream after {} attempts: {}", max_attempts, e));
                    }
                    eprintln!("Camera stream open attempt {} failed, retrying in 300ms...", attempts);
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            }
        }
        
        std::thread::sleep(std::time::Duration::from_millis(500));
        
        let res = camera.resolution();
        let buffer_size = (res.width() * res.height() * 3) as usize;
        
        Ok(Self { 
            camera,
            buffer: Vec::with_capacity(buffer_size),
            frame_skip_counter: 0,
            last_successful_frame: None,
        })
    }
    
    pub fn get_frame(&mut self) -> Result<&[u8]> {
        let mut attempts = 0;
        let max_attempts = 3;
        
        while attempts < max_attempts {
            match self.try_get_frame() {
                Ok(_) => {
                    self.last_successful_frame = Some(self.buffer.clone());
                    return Ok(&self.buffer);
                }
                Err(e) => {
                    attempts += 1;
                    
                    let error_msg = e.to_string();
                    let is_hardware_issue = error_msg.contains("0xC00D3704") || 
                                          error_msg.contains("hardware") || 
                                          error_msg.contains("MFT") ||
                                          error_msg.contains("Hardware MFT failed to start streaming");
                    
                    if is_hardware_issue {
                        
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        
                        self.frame_skip_counter += 1;
                        if self.frame_skip_counter < 3 {
                            if let Some(ref last_frame) = self.last_successful_frame {
                                self.buffer.clear();
                                self.buffer.extend_from_slice(last_frame);
                                return Ok(&self.buffer);
                            }
                        } else {
                            self.frame_skip_counter = 0;
                        }
                    } else {
                        eprintln!("Frame capture attempt {} failed: {}", attempts, e);
                    }
                    
                    if attempts >= max_attempts {
                        if let Some(ref last_frame) = self.last_successful_frame {
                            self.buffer.clear();
                            self.buffer.extend_from_slice(last_frame);
                            return Ok(&self.buffer);
                        }
                        
                        if !is_hardware_issue {
                            return Err(e);
                        } else {
                            let (width, height) = self.dimensions();
                            let frame_size = (width * height * 3) as usize;
                            self.buffer.clear();
                            self.buffer.resize(frame_size, 0);
                            return Ok(&self.buffer);
                        }
                    }
                    
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
        
        Err(anyhow::anyhow!("Failed to capture frame after {} attempts", max_attempts))
    }
    
    fn try_get_frame(&mut self) -> Result<()> {
        let frame = self.camera.frame()?;
        let img = frame.decode_image::<RgbFormat>()?;
        let raw_data = img.as_raw();
        
        let expected = raw_data.len();
        if self.buffer.capacity() < expected {
            self.buffer.reserve(expected - self.buffer.capacity());
        }
        
        self.buffer.clear();
        self.buffer.extend_from_slice(raw_data);
        
        Ok(())
    }
    
    pub fn dimensions(&self) -> (u32, u32) {
        let res = self.camera.resolution();
        (res.width(), res.height())
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        let _ = self.camera.stop_stream();
        
        // Cleanup COM on Windows
        #[cfg(windows)]
        unsafe {
            CoUninitialize();
        }
    }
}