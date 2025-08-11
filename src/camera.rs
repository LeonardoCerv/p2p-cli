use nokhwa::{
    pixel_format::RgbFormat,
    utils::{CameraIndex, RequestedFormat, RequestedFormatType, Resolution, FrameFormat, CameraFormat},
    Camera
};
use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[cfg(windows)]
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

pub struct CameraCapture {
    camera: Camera,
    buffer: Vec<u8>,
    backup_buffer: Vec<u8>,
    consecutive_failures: u32,
    is_healthy: Arc<AtomicBool>,
    frame_pool: Vec<Vec<u8>>,
    current_pool_index: usize,
}

impl CameraCapture {
    pub fn new() -> Result<Self> {
        #[cfg(windows)]
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let formats = vec![
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(640, 480),
                FrameFormat::MJPEG,
                30
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(320, 240),
                FrameFormat::MJPEG,
                60
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(640, 480),
                FrameFormat::YUYV,
                30
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(320, 240),
                FrameFormat::YUYV,
                60
            ))),
            RequestedFormat::new::<RgbFormat>(RequestedFormatType::Exact(CameraFormat::new(
                Resolution::new(160, 120),
                FrameFormat::MJPEG,
                30
            ))),
        ];
        
        for (format_idx, format) in formats.iter().enumerate() {
            println!("Trying high-performance camera format {}: {:?}", format_idx, format);
            
            for camera_index in [0, 1, 2] {
                match Self::try_create_camera(camera_index, format.clone()) {
                    Ok(camera_capture) => {
                        println!("Successfully initialized camera {} with format {} for high-performance capture", camera_index, format_idx);
                        return Ok(camera_capture);
                    }
                    Err(e) => {
                        eprintln!("Camera {} with format {} failed: {}", camera_index, format_idx, e);
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
            }
        }
        
        Err(anyhow::anyhow!("Failed to initialize camera with any high-performance format. Windows troubleshooting:\n1. Close all camera applications (Skype, Teams, OBS, etc.)\n2. Run as administrator\n3. Check Windows Privacy Settings > Camera\n4. Restart Windows if issues persist"))
    }
    
    fn try_create_camera(camera_index: u32, format: RequestedFormat) -> Result<Self> {
        std::thread::sleep(std::time::Duration::from_millis(25));
        
        let mut camera = Camera::new(CameraIndex::Index(camera_index), format)?;
        
        let mut attempts = 0;
        let max_attempts = 3;
        
        while attempts < max_attempts {
            match camera.open_stream() {
                Ok(_) => break,
                Err(e) => {
                    attempts += 1;
                    if attempts >= max_attempts {
                        return Err(anyhow::anyhow!("Failed to open camera stream after {} attempts: {}", max_attempts, e));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        
        std::thread::sleep(std::time::Duration::from_millis(200));
        
        let res = camera.resolution();
        let buffer_size = (res.width() * res.height() * 3) as usize;
        
        let mut frame_pool = Vec::with_capacity(3);
        for _ in 0..3 {
            frame_pool.push(vec![0u8; buffer_size]);
        }
        
        Ok(Self { 
            camera,
            buffer: Vec::with_capacity(buffer_size),
            backup_buffer: vec![0u8; buffer_size],
            consecutive_failures: 0,
            is_healthy: Arc::new(AtomicBool::new(true)),
            frame_pool,
            current_pool_index: 0,
        })
    }
    
    pub fn get_frame(&mut self) -> Result<&[u8]> {
        match self.try_get_frame_fast() {
            Ok(_) => {
                self.consecutive_failures = 0;
                self.is_healthy.store(true, Ordering::Relaxed);
                return Ok(&self.buffer);
            }
            Err(e) => {
                self.consecutive_failures += 1;
                
                if self.consecutive_failures > 5 {
                    self.is_healthy.store(false, Ordering::Relaxed);
                }
                
                let error_msg = e.to_string();
                if error_msg.contains("0xC00D3704") || 
                   error_msg.contains("MFT") ||
                   error_msg.contains("hardware") ||
                   self.consecutive_failures > 3 {
                    
                    self.buffer.clear();
                    self.buffer.extend_from_slice(&self.backup_buffer);
                    return Ok(&self.buffer);
                }
                
                match self.try_get_frame_fast() {
                    Ok(_) => {
                        self.consecutive_failures = 0;
                        return Ok(&self.buffer);
                    }
                    Err(_) => {
                        self.buffer.clear();
                        self.buffer.extend_from_slice(&self.backup_buffer);
                        return Ok(&self.buffer);
                    }
                }
            }
        }
    }
    
    fn try_get_frame_fast(&mut self) -> Result<()> {
        let frame = self.camera.frame()?;
        let img = frame.decode_image::<RgbFormat>()?;
        let raw_data = img.as_raw();
        
        let target_buffer = &mut self.frame_pool[self.current_pool_index];
        
        if target_buffer.len() >= raw_data.len() {
            target_buffer[..raw_data.len()].copy_from_slice(raw_data);
            
            std::mem::swap(&mut self.buffer, target_buffer);
            self.buffer.truncate(raw_data.len());
            
            if self.buffer.len() <= self.backup_buffer.len() {
                self.backup_buffer[..self.buffer.len()].copy_from_slice(&self.buffer);
            }
        } else {
            self.buffer.clear();
            self.buffer.extend_from_slice(raw_data);
            
            self.backup_buffer.clear();
            self.backup_buffer.extend_from_slice(&self.buffer);
        }
        
        self.current_pool_index = (self.current_pool_index + 1) % self.frame_pool.len();
        
        Ok(())
    }
    
    pub fn is_healthy(&self) -> bool {
        self.is_healthy.load(Ordering::Relaxed)
    }
    
    pub fn dimensions(&self) -> (u32, u32) {
        let res = self.camera.resolution();
        (res.width(), res.height())
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        let _ = self.camera.stop_stream();
        std::thread::sleep(std::time::Duration::from_millis(50));
        
        #[cfg(windows)]
        unsafe {
            CoUninitialize();
        }
    }
}