//! System bus / MMU: routes CPU memory accesses to all peripherals and
//! advances them in lock-step with CPU cycles.

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::interrupts::Interrupts;
use crate::joypad::Joypad;
use crate::ppu::Ppu;
use crate::timer::Timer;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Bus {
    pub cart: Cartridge,
    pub ppu: Ppu,
    pub apu: Apu,
    pub timer: Timer,
    pub joypad: Joypad,
    pub ints: Interrupts,
    /// 8 banks of 4 KiB (CGB); DMG uses banks 0-1.
    #[serde(with = "serde_big_array::BigArray")]
    pub wram: [u8; 0x8000],
    #[serde(with = "serde_big_array::BigArray")]
    pub hram: [u8; 0x7F],
    serial_data: u8,
    serial_ctrl: u8,
    /// Bytes written to the serial port (handy for test ROMs like Blargg's).
    /// A debug capture, not machine state — excluded from save states.
    #[serde(skip)]
    pub serial_out: Vec<u8>,
    dma_src: u8,
    dma_countdown: u16, // remaining T-cycles of an active OAM DMA
    /// True for CGB ROMs: enables WRAM/VRAM banking and colour rendering.
    pub cgb: bool,
    /// WRAM bank select (SVBK); bank 0 maps to 1.
    svbk: u8,
    /// KEY1 double-speed "armed" bit (speed switching itself is not yet done).
    key1: u8,
    /// CGB VRAM DMA source/destination, latched from HDMA1-4.
    hdma_src: u16,
    hdma_dst: u16,
}

impl Bus {
    pub fn new(cart: Cartridge) -> Self {
        let cgb = cart.read(0x0143) & 0x80 != 0;
        let mut ppu = Ppu::default();
        ppu.set_cgb(cgb);
        Bus {
            cart,
            ppu,
            apu: Apu::default(),
            timer: Timer::default(),
            joypad: Joypad::default(),
            ints: Interrupts::default(),
            wram: [0; 0x8000],
            hram: [0; 0x7F],
            serial_data: 0,
            serial_ctrl: 0,
            serial_out: Vec::new(),
            dma_src: 0,
            dma_countdown: 0,
            cgb,
            svbk: 1,
            key1: 0,
            hdma_src: 0,
            hdma_dst: 0,
        }
    }

    /// Map a 0xC000-0xDFFF (or echo) address to a flat WRAM index, honouring
    /// the CGB bank select for the 0xD000-0xDFFF window.
    fn wram_index(&self, addr: u16) -> usize {
        let off = (addr & 0x1FFF) as usize; // 0..0x2000 within C000-DFFF/echo
        if off < 0x1000 {
            off
        } else {
            let bank = (self.svbk as usize & 0x07).max(1);
            bank * 0x1000 + (off - 0x1000)
        }
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cart.read(addr),
            0x8000..=0x9FFF => self.ppu.read(addr),
            0xC000..=0xDFFF => self.wram[self.wram_index(addr)],
            0xE000..=0xFDFF => self.wram[self.wram_index(addr)], // echo RAM
            0xFE00..=0xFE9F => self.ppu.read(addr),
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00 => self.joypad.read(),
            0xFF01 => self.serial_data,
            0xFF02 => self.serial_ctrl | 0x7E,
            0xFF04..=0xFF07 => self.timer.read(addr),
            0xFF0F => self.ints.request | 0xE0,
            0xFF10..=0xFF3F => self.apu.read(addr),
            0xFF46 => self.dma_src,
            0xFF4D => 0x7E | (self.key1 & 0x01), // KEY1: current speed 0 for now
            0xFF40..=0xFF4B => self.ppu.read(addr),
            0xFF4F => self.ppu.read(addr),                 // VBK
            0xFF55 => 0xFF,                                // VRAM DMA idle/complete
            0xFF68..=0xFF6B => self.ppu.read(addr),        // BG/OBJ palettes
            0xFF70 => 0xF8 | (self.svbk & 0x07),           // SVBK
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ints.enable,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        match addr {
            0x0000..=0x7FFF | 0xA000..=0xBFFF => self.cart.write(addr, val),
            0x8000..=0x9FFF => self.ppu.write(addr, val, &mut self.ints),
            0xC000..=0xDFFF => self.wram[self.wram_index(addr)] = val,
            0xE000..=0xFDFF => self.wram[self.wram_index(addr)] = val,
            0xFE00..=0xFE9F => self.ppu.write(addr, val, &mut self.ints),
            0xFEA0..=0xFEFF => {}
            0xFF00 => self.joypad.write(val),
            0xFF01 => self.serial_data = val,
            0xFF02 => {
                self.serial_ctrl = val;
                if val & 0x80 != 0 {
                    // Instant transfer, no external device: record the byte.
                    // Cap the log — games poll the link port forever.
                    if self.serial_out.len() < 0x10000 {
                        self.serial_out.push(self.serial_data);
                    }
                    self.serial_ctrl &= 0x7F;
                }
            }
            0xFF04..=0xFF07 => self.timer.write(addr, val, &mut self.ints),
            0xFF0F => self.ints.request = val & 0x1F,
            0xFF10..=0xFF3F => self.apu.write(addr, val),
            0xFF46 => self.start_dma(val),
            0xFF4D => self.key1 = (self.key1 & 0x80) | (val & 0x01),
            0xFF40..=0xFF4B => self.ppu.write(addr, val, &mut self.ints),
            0xFF4F => self.ppu.write(addr, val, &mut self.ints), // VBK
            0xFF51 => self.hdma_src = (self.hdma_src & 0x00FF) | ((val as u16) << 8),
            0xFF52 => self.hdma_src = (self.hdma_src & 0xFF00) | (val as u16 & 0xF0),
            0xFF53 => self.hdma_dst = (self.hdma_dst & 0x00FF) | (((val as u16) & 0x1F) << 8),
            0xFF54 => self.hdma_dst = (self.hdma_dst & 0xFF00) | (val as u16 & 0xF0),
            0xFF55 => self.vram_dma(val),
            0xFF68..=0xFF6B => self.ppu.write(addr, val, &mut self.ints), // palettes
            0xFF70 => self.svbk = val & 0x07,
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = val,
            0xFFFF => self.ints.enable = val,
            _ => {}
        }
    }

    /// CGB VRAM DMA (HDMA5). Both general-purpose (bit 7 clear) and HBlank
    /// (bit 7 set) transfers are performed immediately here; sub-frame HBlank
    /// pacing is a later refinement.
    fn vram_dma(&mut self, ctrl: u8) {
        let len = ((ctrl as u16 & 0x7F) + 1) * 0x10;
        let src = self.hdma_src & 0xFFF0;
        let dst = 0x8000 | (self.hdma_dst & 0x1FF0);
        for i in 0..len {
            let b = self.read(src.wrapping_add(i));
            self.ppu.write(dst.wrapping_add(i), b, &mut self.ints);
        }
    }

    fn start_dma(&mut self, src: u8) {
        self.dma_src = src;
        // Copy immediately (games wait in HRAM for the 160 M-cycle duration).
        let base = (src as u16) << 8;
        for i in 0..0xA0u16 {
            let b = self.read(base + i);
            self.ppu.oam[i as usize] = b;
        }
        self.dma_countdown = 640;
    }

    pub fn dma_active(&self) -> bool {
        self.dma_countdown > 0
    }

    /// Advance all peripherals by `cycles` T-cycles.
    pub fn tick(&mut self, cycles: u32) {
        self.timer.tick(cycles, &mut self.ints);
        self.ppu.tick(cycles, &mut self.ints);
        self.apu.tick(cycles);
        self.dma_countdown = self.dma_countdown.saturating_sub(cycles as u16);
    }
}

/// The production adapter for the CPU's [`Memory`](crate::memory::Memory) seam.
/// Forwards to the inherent routing methods so existing callers stay unchanged.
impl crate::memory::Memory for Bus {
    fn read(&self, addr: u16) -> u8 {
        Bus::read(self, addr)
    }
    fn write(&mut self, addr: u16, val: u8) {
        Bus::write(self, addr, val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bus() -> Bus {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0;
        Bus::new(Cartridge::from_rom(rom).unwrap())
    }

    #[test]
    fn wram_and_echo() {
        let mut b = bus();
        b.write(0xC123, 0xAB);
        assert_eq!(b.read(0xC123), 0xAB);
        assert_eq!(b.read(0xE123), 0xAB);
        b.write(0xE200, 0x55);
        assert_eq!(b.read(0xC200), 0x55);
    }

    #[test]
    fn hram_roundtrip() {
        let mut b = bus();
        b.write(0xFF80, 0x11);
        b.write(0xFFFE, 0x22);
        assert_eq!(b.read(0xFF80), 0x11);
        assert_eq!(b.read(0xFFFE), 0x22);
    }

    #[test]
    fn ie_and_if_registers() {
        let mut b = bus();
        b.write(0xFFFF, 0x1F);
        b.write(0xFF0F, 0x05);
        assert_eq!(b.read(0xFFFF), 0x1F);
        assert_eq!(b.read(0xFF0F), 0xE5);
    }

    #[test]
    fn oam_dma_copies_160_bytes() {
        let mut b = bus();
        for i in 0..0xA0u16 {
            b.write(0xC000 + i, i as u8);
        }
        b.write(0xFF46, 0xC0);
        assert!(b.dma_active());
        assert_eq!(b.ppu.oam[0], 0);
        assert_eq!(b.ppu.oam[0x9F], 0x9F);
        b.tick(640);
        assert!(!b.dma_active());
    }

    #[test]
    fn serial_capture() {
        let mut b = bus();
        b.write(0xFF01, b'H');
        b.write(0xFF02, 0x81);
        b.write(0xFF01, b'i');
        b.write(0xFF02, 0x81);
        assert_eq!(b.serial_out, b"Hi");
    }
}
