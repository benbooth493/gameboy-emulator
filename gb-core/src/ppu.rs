//! Pixel processing unit: scanline renderer with mode timing, STAT
//! interrupts, background, window and sprites.
//!
//! The framebuffer stores one shade index (0..=3, 0 = lightest) per pixel;
//! colour mapping is left to the frontend (the shader applies the DMG green).

use crate::interrupts::{Interrupts, INT_STAT, INT_VBLANK};
use serde_big_array::BigArray;

pub const SCREEN_W: usize = 160;
pub const SCREEN_H: usize = 144;

// LCDC bits
const LCDC_ENABLE: u8 = 0x80;
const LCDC_WIN_MAP: u8 = 0x40;
const LCDC_WIN_ENABLE: u8 = 0x20;
const LCDC_TILE_DATA: u8 = 0x10;
const LCDC_BG_MAP: u8 = 0x08;
const LCDC_OBJ_SIZE: u8 = 0x04;
const LCDC_OBJ_ENABLE: u8 = 0x02;
const LCDC_BG_ENABLE: u8 = 0x01;

// STAT bits
const STAT_LYC_INT: u8 = 0x40;
const STAT_OAM_INT: u8 = 0x20;
const STAT_VBLANK_INT: u8 = 0x10;
const STAT_HBLANK_INT: u8 = 0x08;
const STAT_LYC_EQ: u8 = 0x04;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    HBlank = 0,
    VBlank = 1,
    OamScan = 2,
    Drawing = 3,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Ppu {
    #[serde(with = "BigArray")]
    pub vram: [u8; 0x2000],
    #[serde(with = "BigArray")]
    pub oam: [u8; 0xA0],
    pub lcdc: u8,
    pub stat: u8,
    pub scy: u8,
    pub scx: u8,
    pub ly: u8,
    pub lyc: u8,
    pub bgp: u8,
    pub obp0: u8,
    pub obp1: u8,
    pub wy: u8,
    pub wx: u8,
    mode: Mode,
    dot: u32,
    window_line: u8,
    /// Shade indices 0..=3 per pixel.
    #[serde(with = "BigArray")]
    pub framebuffer: [u8; SCREEN_W * SCREEN_H],
    /// Set when a full frame has just been completed; caller clears it.
    pub frame_ready: bool,
}

impl Default for Ppu {
    fn default() -> Self {
        Ppu {
            vram: [0; 0x2000],
            oam: [0; 0xA0],
            lcdc: 0x91,
            stat: 0x04, // LY==LYC at boot; mode bits are never stored
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            mode: Mode::OamScan,
            dot: 0,
            window_line: 0,
            framebuffer: [0; SCREEN_W * SCREEN_H],
            frame_ready: false,
        }
    }
}

impl Ppu {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize],
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize],
            0xFF40 => self.lcdc,
            // Bits 3-6 are the stored interrupt enables, bit 2 the LYC flag;
            // the mode bits always come live from the PPU.
            0xFF41 => 0x80 | (self.stat & 0x7C) | self.mode as u8,
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8, ints: &mut Interrupts) {
        match addr {
            0x8000..=0x9FFF => self.vram[(addr - 0x8000) as usize] = val,
            0xFE00..=0xFE9F => self.oam[(addr - 0xFE00) as usize] = val,
            0xFF40 => {
                let was_on = self.lcdc & LCDC_ENABLE != 0;
                self.lcdc = val;
                let now_on = self.lcdc & LCDC_ENABLE != 0;
                if was_on && !now_on {
                    self.ly = 0;
                    self.dot = 0;
                    self.mode = Mode::HBlank;
                    self.window_line = 0;
                } else if !was_on && now_on {
                    self.mode = Mode::OamScan;
                    self.check_lyc(ints);
                }
            }
            0xFF41 => self.stat = (val & 0x78) | (self.stat & 0x04),
            0xFF42 => self.scy = val,
            0xFF43 => self.scx = val,
            0xFF44 => {} // LY read-only
            0xFF45 => {
                self.lyc = val;
                self.check_lyc(ints);
            }
            0xFF47 => self.bgp = val,
            0xFF48 => self.obp0 = val,
            0xFF49 => self.obp1 = val,
            0xFF4A => self.wy = val,
            0xFF4B => self.wx = val,
            _ => {}
        }
    }

    fn check_lyc(&mut self, ints: &mut Interrupts) {
        if self.ly == self.lyc {
            let was = self.stat & STAT_LYC_EQ != 0;
            self.stat |= STAT_LYC_EQ;
            if !was && self.stat & STAT_LYC_INT != 0 {
                ints.raise(INT_STAT);
            }
        } else {
            self.stat &= !STAT_LYC_EQ;
        }
    }

    fn set_mode(&mut self, mode: Mode, ints: &mut Interrupts) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        let int_bit = match mode {
            Mode::HBlank => STAT_HBLANK_INT,
            Mode::VBlank => STAT_VBLANK_INT,
            Mode::OamScan => STAT_OAM_INT,
            Mode::Drawing => 0,
        };
        if int_bit != 0 && self.stat & int_bit != 0 {
            ints.raise(INT_STAT);
        }
    }

    /// Advance by `cycles` T-cycles (dots).
    pub fn tick(&mut self, cycles: u32, ints: &mut Interrupts) {
        if self.lcdc & LCDC_ENABLE == 0 {
            return;
        }
        for _ in 0..cycles {
            self.dot += 1;
            match self.mode {
                Mode::OamScan => {
                    if self.dot >= 80 {
                        self.set_mode(Mode::Drawing, ints);
                    }
                }
                Mode::Drawing => {
                    if self.dot >= 80 + 172 {
                        self.render_scanline();
                        self.set_mode(Mode::HBlank, ints);
                    }
                }
                Mode::HBlank => {
                    if self.dot >= 456 {
                        self.dot = 0;
                        self.ly += 1;
                        self.check_lyc(ints);
                        if self.ly == 144 {
                            self.set_mode(Mode::VBlank, ints);
                            ints.raise(INT_VBLANK);
                            self.frame_ready = true;
                        } else {
                            self.set_mode(Mode::OamScan, ints);
                        }
                    }
                }
                Mode::VBlank => {
                    if self.dot >= 456 {
                        self.dot = 0;
                        self.ly += 1;
                        if self.ly > 153 {
                            self.ly = 0;
                            self.window_line = 0;
                            self.set_mode(Mode::OamScan, ints);
                        }
                        self.check_lyc(ints);
                    }
                }
            }
        }
    }

    fn tile_row(&self, tile_idx: u8, row: usize, signed_addressing: bool) -> (u8, u8) {
        let base = if signed_addressing {
            (0x1000i32 + (tile_idx as i8 as i32) * 16) as usize
        } else {
            tile_idx as usize * 16
        };
        (self.vram[base + row * 2], self.vram[base + row * 2 + 1])
    }

    #[allow(clippy::needless_range_loop)] // indexed pixel loops are clearer here
    fn render_scanline(&mut self) {
        let y = self.ly as usize;
        if y >= SCREEN_H {
            return;
        }
        // Raw BG colour index (pre-palette) per pixel, needed for sprite priority.
        let mut bg_index = [0u8; SCREEN_W];

        if self.lcdc & LCDC_BG_ENABLE != 0 {
            let signed = self.lcdc & LCDC_TILE_DATA == 0;
            // Background
            let map_base: usize = if self.lcdc & LCDC_BG_MAP != 0 { 0x1C00 } else { 0x1800 };
            let bg_y = (y as u8).wrapping_add(self.scy) as usize;
            for x in 0..SCREEN_W {
                let bg_x = (x as u8).wrapping_add(self.scx) as usize;
                let tile = self.vram[map_base + (bg_y / 8) * 32 + bg_x / 8];
                let (lo, hi) = self.tile_row(tile, bg_y % 8, signed);
                let bit = 7 - (bg_x % 8);
                let idx = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
                bg_index[x] = idx;
            }
            // Window
            if self.lcdc & LCDC_WIN_ENABLE != 0 && self.wy as usize <= y && self.wx < 167 {
                let map_base: usize =
                    if self.lcdc & LCDC_WIN_MAP != 0 { 0x1C00 } else { 0x1800 };
                let win_y = self.window_line as usize;
                let start_x = self.wx.saturating_sub(7) as usize;
                let mut drew = false;
                for x in start_x..SCREEN_W {
                    let win_x = x + 7 - self.wx as usize;
                    let tile = self.vram[map_base + (win_y / 8) * 32 + win_x / 8];
                    let (lo, hi) = self.tile_row(tile, win_y % 8, signed);
                    let bit = 7 - (win_x % 8);
                    let idx = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
                    bg_index[x] = idx;
                    drew = true;
                }
                if drew {
                    self.window_line += 1;
                }
            }
        }

        let mut line = [0u8; SCREEN_W];
        for x in 0..SCREEN_W {
            line[x] = (self.bgp >> (bg_index[x] * 2)) & 0x03;
        }

        // Sprites
        if self.lcdc & LCDC_OBJ_ENABLE != 0 {
            let tall = self.lcdc & LCDC_OBJ_SIZE != 0;
            let height = if tall { 16i32 } else { 8 };
            // Collect up to 10 sprites on this line, in OAM order.
            let mut sprites: Vec<(i32, usize)> = Vec::with_capacity(10);
            for i in 0..40 {
                let sy = self.oam[i * 4] as i32 - 16;
                if (y as i32) >= sy && (y as i32) < sy + height {
                    sprites.push((self.oam[i * 4 + 1] as i32 - 8, i));
                    if sprites.len() == 10 {
                        break;
                    }
                }
            }
            // DMG priority: lower X wins; ties broken by OAM order.
            // Draw in reverse priority so higher-priority sprites overwrite.
            sprites.sort_by_key(|&(x, i)| (x, i));
            for &(sx, i) in sprites.iter().rev() {
                let sy = self.oam[i * 4] as i32 - 16;
                let mut tile = self.oam[i * 4 + 2];
                let attr = self.oam[i * 4 + 3];
                let mut row = y as i32 - sy;
                if attr & 0x40 != 0 {
                    row = height - 1 - row; // Y flip
                }
                if tall {
                    tile &= 0xFE;
                    if row >= 8 {
                        tile |= 1;
                        row -= 8;
                    }
                }
                let (lo, hi) = self.tile_row(tile, row as usize, false);
                let palette = if attr & 0x10 != 0 { self.obp1 } else { self.obp0 };
                for px in 0..8i32 {
                    let x = sx + px;
                    if !(0..SCREEN_W as i32).contains(&x) {
                        continue;
                    }
                    let bit = if attr & 0x20 != 0 { px } else { 7 - px }; // X flip
                    let idx = ((hi >> bit) & 1) << 1 | ((lo >> bit) & 1);
                    if idx == 0 {
                        continue; // transparent
                    }
                    if attr & 0x80 != 0 && bg_index[x as usize] != 0 {
                        continue; // behind non-zero BG
                    }
                    line[x as usize] = (palette >> (idx * 2)) & 0x03;
                }
            }
        }

        self.framebuffer[y * SCREEN_W..(y + 1) * SCREEN_W].copy_from_slice(&line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_line(ppu: &mut Ppu, ints: &mut Interrupts) {
        ppu.tick(456, ints);
    }

    #[test]
    fn mode_sequence_and_frame_timing() {
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        assert_eq!(ppu.mode(), Mode::OamScan);
        ppu.tick(80, &mut ints);
        assert_eq!(ppu.mode(), Mode::Drawing);
        ppu.tick(172, &mut ints);
        assert_eq!(ppu.mode(), Mode::HBlank);
        ppu.tick(456 - 252, &mut ints);
        assert_eq!(ppu.ly, 1);
        assert_eq!(ppu.mode(), Mode::OamScan);
        // Complete the frame: 154 lines total.
        for _ in 1..154 {
            run_line(&mut ppu, &mut ints);
        }
        assert_eq!(ppu.ly, 0);
        assert!(ppu.frame_ready);
    }

    #[test]
    fn vblank_interrupt_at_line_144() {
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        ints.enable = 0xFF;
        for _ in 0..144 {
            run_line(&mut ppu, &mut ints);
        }
        assert_eq!(ppu.mode(), Mode::VBlank);
        assert!(ints.pending() & INT_VBLANK != 0);
    }

    #[test]
    fn lyc_coincidence_raises_stat() {
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        ints.enable = 0xFF;
        ppu.write(0xFF45, 5, &mut ints); // LYC = 5
        ppu.write(0xFF41, STAT_LYC_INT, &mut ints);
        for _ in 0..5 {
            run_line(&mut ppu, &mut ints);
        }
        assert!(ints.pending() & INT_STAT != 0);
        assert!(ppu.read(0xFF41) & STAT_LYC_EQ != 0);
    }

    #[test]
    fn renders_solid_bg_tile() {
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        // Tile 0: all pixels colour index 3.
        for i in 0..16 {
            ppu.vram[i] = 0xFF;
        }
        // BG map already zeroed -> tile 0 everywhere. BGP identity: 11 10 01 00.
        ppu.bgp = 0b11100100;
        run_line(&mut ppu, &mut ints);
        assert!(ppu.framebuffer[..SCREEN_W].iter().all(|&p| p == 3));
    }

    #[test]
    fn bgp_remaps_shades() {
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        for i in 0..16 {
            ppu.vram[i] = 0xFF; // index 3
        }
        ppu.bgp = 0b00_11_11_11; // index 3 -> shade 0
        run_line(&mut ppu, &mut ints);
        assert!(ppu.framebuffer[..SCREEN_W].iter().all(|&p| p == 0));
    }

    #[test]
    fn sprite_renders_over_bg() {
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        ppu.lcdc |= LCDC_OBJ_ENABLE;
        // Sprite tile 1: solid colour index 1.
        for i in 0..8 {
            ppu.vram[16 + i * 2] = 0xFF;
            ppu.vram[16 + i * 2 + 1] = 0x00;
        }
        ppu.obp0 = 0b11100100;
        // Sprite 0 at top-left: y=16, x=8 -> screen (0,0)
        ppu.oam[0] = 16;
        ppu.oam[1] = 8;
        ppu.oam[2] = 1;
        ppu.oam[3] = 0;
        run_line(&mut ppu, &mut ints);
        assert_eq!(ppu.framebuffer[0], 1);
        assert_eq!(ppu.framebuffer[7], 1);
        assert_eq!(ppu.framebuffer[8], 0);
    }

    #[test]
    fn stat_mode_bits_track_live_mode() {
        // Regression: Tetris polls STAT & 3 == 0 to write VRAM during
        // HBlank; stale mode bits in the stored STAT byte must never leak
        // into reads.
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        ppu.tick(80, &mut ints); // OAM scan done
        assert_eq!(ppu.read(0xFF41) & 0x03, Mode::Drawing as u8);
        ppu.tick(172, &mut ints);
        assert_eq!(ppu.read(0xFF41) & 0x03, 0, "HBlank must read mode 0");
        // Writes only affect the interrupt-enable bits.
        ppu.write(0xFF41, 0xFF, &mut ints);
        assert_eq!(ppu.read(0xFF41) & 0x03, 0);
        // And VBlank reads mode 1.
        for _ in 0..144 {
            ppu.tick(456, &mut ints);
        }
        assert_eq!(ppu.read(0xFF41) & 0x03, 1);
    }

    #[test]
    fn lcd_off_freezes_ly() {
        let mut ppu = Ppu::default();
        let mut ints = Interrupts::default();
        ppu.write(0xFF40, 0x00, &mut ints);
        ppu.tick(456 * 10, &mut ints);
        assert_eq!(ppu.ly, 0);
    }
}
