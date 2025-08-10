use nokhwa::{
    pixel_format::RgbFormat,
    utils::{CameraIndex, RequestedFormat, RequestedFormatType, Resolution},
    Camera
};
use anyhow::Result;

pub struct CameraCapture {
    camera: Camera,
    buffer: Vec<u8>,
}

impl CameraCapture {
    pub fn new() -> Result<Self> {
        let format = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
        let mut camera = Camera::new(CameraIndex::Index(0), format)?;
        
        if camera.resolution().width() > 640 {
            let _ = camera.set_resolution(Resolution::new(640, 480));
        }
        
        camera.open_stream()?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        
        let res = camera.resolution();
        let buffer_size = (res.width() * res.height() * 3) as usize;
        
        Ok(Self { 
            camera,
            buffer: Vec::with_capacity(buffer_size),
        })
    }
    
    pub fn get_frame(&mut self) -> Result<&[u8]> {
        let frame = self.camera.frame()?;
        let img = frame.decode_image::<RgbFormat>()?;
        let raw_data = img.as_raw();
        
        let expected = raw_data.len();
        if self.buffer.capacity() < expected {
            self.buffer.reserve(expected - self.buffer.capacity());
        }
        
        self.buffer.clear();
        self.buffer.extend_from_slice(raw_data);
        
        Ok(&self.buffer)
    }
    
    pub fn dimensions(&self) -> (u32, u32) {
        let res = self.camera.resolution();
        (res.width(), res.height())
    }
}

impl Drop for CameraCapture {
    fn drop(&mut self) {
        let _ = self.camera.stop_stream();
    }
}