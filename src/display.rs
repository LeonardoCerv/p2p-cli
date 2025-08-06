use std::io::{self, Write, BufWriter};
use anyhow::Result;

pub struct TerminalDisplay {
    cam_w: u32,
    cam_h: u32,
    term_w: usize,
    term_h: usize,
    disp_w: usize,
    disp_h: usize,
    scale: u32,
    h_pad: usize,
    v_pad: usize,
    buf: String,
    writer: BufWriter<std::io::Stdout>,
    redraw: bool,
}

impl TerminalDisplay {
    pub fn new(cam_w: u32, cam_h: u32) -> Self {
        let (term_w, term_h) = term_size();
        
        let max_w = term_w.saturating_sub(2);
        let max_h = term_h.saturating_sub(3);
        
        let scale_x = (cam_w as f32 / max_w as f32).ceil() as u32;
        let scale_y = (cam_h as f32 / (max_h * 2) as f32).ceil() as u32;
        let scale = scale_x.max(scale_y).max(2);
        
        let disp_w = (cam_w / scale).max(1) as usize;
        let disp_h = (cam_h / (scale * 2)).max(1) as usize;
        
        let h_pad = (term_w.saturating_sub(disp_w)) / 2;
        let v_pad = (term_h.saturating_sub(disp_h).saturating_sub(2)) / 2;
        
        let buf_size = (disp_w * disp_h * 50) + 1000;
        
        print!("\x1B[?25l");
        io::stdout().flush().unwrap();
        
        Self {
            cam_w,
            cam_h,
            term_w,
            term_h,
            disp_w,
            disp_h,
            scale,
            h_pad,
            v_pad,
            buf: String::with_capacity(buf_size),
            writer: BufWriter::with_capacity(32768, io::stdout()),
            redraw: true,
        }
    }

    pub fn show_frame(&mut self, pixels: &[(u8, u8, u8)]) -> Result<()> {
        let (new_w, new_h) = term_size();
        if new_w != self.term_w || new_h != self.term_h {
            self.term_w = new_w;
            self.term_h = new_h;
            self.calc_layout();
            self.redraw = true;
        }
        
        self.render_blocks(pixels)
    }
    
    fn calc_layout(&mut self) {
        let max_w = self.term_w.saturating_sub(2);
        let max_h = self.term_h.saturating_sub(3);
        
        let scale_x = (self.cam_w as f32 / max_w as f32).ceil() as u32;
        let scale_y = (self.cam_h as f32 / (max_h * 2) as f32).ceil() as u32;
        self.scale = scale_x.max(scale_y).max(2);
        
        self.disp_w = (self.cam_w / self.scale).max(1) as usize;
        self.disp_h = (self.cam_h / (self.scale * 2)).max(1) as usize;
        
        self.h_pad = (self.term_w.saturating_sub(self.disp_w)) / 2;
        self.v_pad = (self.term_h.saturating_sub(self.disp_h).saturating_sub(2)) / 2;
    }

    fn render_blocks(&mut self, pixels: &[(u8, u8, u8)]) -> Result<()> {
        self.buf.clear();
        
        if self.redraw {
            self.buf.push_str("\x1B[2J\x1B[H");
            self.redraw = false;
        } else {
            self.buf.push_str("\x1B[H");
        }
        
        for _ in 0..self.v_pad {
            self.buf.push('\n');
        }
        
        let mut last_top = (255u8, 255u8, 255u8);
        let mut last_bot = (255u8, 255u8, 255u8);
        
        for y in 0..self.disp_h {
            for _ in 0..self.h_pad {
                self.buf.push(' ');
            }
            
            for x in 0..self.disp_w {
                let src_x = ((x as u32 * self.scale) as usize).min(self.cam_w as usize - 1);
                let src_y_top = ((y as u32 * self.scale * 2) as usize).min(self.cam_h as usize - 1);
                let src_y_bot = (((y as u32 * self.scale * 2) + self.scale) as usize).min(self.cam_h as usize - 1);
                
                let top_idx = src_y_top * self.cam_w as usize + src_x;
                let bot_idx = src_y_bot * self.cam_w as usize + src_x;
                
                if top_idx < pixels.len() && bot_idx < pixels.len() {
                    let (r1, g1, b1) = pixels[top_idx];
                    let (r2, g2, b2) = pixels[bot_idx];
                    
                    if (r1, g1, b1) != last_top || (r2, g2, b2) != last_bot {
                        self.buf.push_str(&format!("\x1B[38;2;{};{};{}m\x1B[48;2;{};{};{}m", r1, g1, b1, r2, g2, b2));
                        last_top = (r1, g1, b1);
                        last_bot = (r2, g2, b2);
                    }
                    self.buf.push('▀');
                } else {
                    self.buf.push(' ');
                }
            }
            
            self.buf.push_str("\x1B[0m\n");
            last_top = (255, 255, 255);
            last_bot = (255, 255, 255);
        }
        
        self.writer.write_all(self.buf.as_bytes())?;
        self.writer.flush()?;
        Ok(())
    }
}

fn term_size() -> (usize, usize) {
    terminal_size::terminal_size()
        .map(|(terminal_size::Width(w), terminal_size::Height(h))| (w as usize, h as usize))
        .unwrap_or((120, 40))
}

impl Drop for TerminalDisplay {
    fn drop(&mut self) {
        print!("\x1B[?25h\x1B[0m");
        let _ = io::stdout().flush();
    }
}