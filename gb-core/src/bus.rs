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
    /// CGB VRAM DMA source/destination; also the running pointers during a
    /// transfer.
    hdma_src: u16,
    hdma_dst: u16,
    /// An HBlank VRAM DMA is in progress, with this many 0x10-byte blocks left.
    hdma_active: bool,
    hdma_blocks: u16,
    /// CGB double-speed mode (KEY1 bit 7).
    double_speed: bool,
    /// Total T-cycles the machine has advanced (for CPU cycle accounting).
    elapsed: u64,
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
            hdma_active: false,
            hdma_blocks: 0,
            double_speed: false,
            elapsed: 0,
        }
    }

    /// Advance the whole machine by one CPU M-cycle. The timer runs on the CPU
    /// clock (4 T, doubled in double-speed), while the PPU/APU run on the base
    /// clock (2 T per CPU M-cycle in double-speed). `elapsed` counts CPU-T.
    fn advance_m(&mut self) {
        let base = if self.double_speed { 2 } else { 4 };
        self.timer.tick(4, &mut self.ints);
        self.ppu.tick(base, &mut self.ints);
        self.apu.tick(base);
        self.dma_countdown = self.dma_countdown.saturating_sub(4);
        self.elapsed += 4;
        // Advance an HBlank VRAM DMA by one block each time a visible-line
        // HBlank begins.
        if self.hdma_active && self.ppu.take_entered_hblank() {
            self.hdma_copy(1);
            self.hdma_blocks -= 1;
            if self.hdma_blocks == 0 {
                self.hdma_active = false;
            }
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
            0xFF4D => 0x7E | (self.key1 & 0x01) | if self.double_speed { 0x80 } else { 0 },
            0xFF40..=0xFF4B => self.ppu.read(addr),
            0xFF4F => self.ppu.read(addr),                 // VBK
            // FF55: bit 7 clear + remaining-1 blocks while active; 0xFF idle.
            0xFF55 => {
                if self.hdma_active {
                    (self.hdma_blocks - 1) as u8 & 0x7F
                } else {
                    0xFF
                }
            }
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

    /// CGB VRAM DMA (HDMA5). Bit 7 set starts an HBlank transfer (one block per
    /// HBlank); writing bit 7 clear during one cancels it. Bit 7 clear with no
    /// transfer active is a general-purpose DMA, copied immediately.
    fn vram_dma(&mut self, ctrl: u8) {
        let blocks = (ctrl as u16 & 0x7F) + 1;
        if ctrl & 0x80 != 0 {
            self.hdma_active = true;
            self.hdma_blocks = blocks;
        } else if self.hdma_active {
            self.hdma_active = false; // cancel in-progress HBlank transfer
        } else {
            self.hdma_copy(blocks);
        }
    }

    /// Copy `blocks` x 0x10 bytes from the source pointer to VRAM, advancing
    /// both running pointers (HDMA1-4 reflect the live position on hardware).
    fn hdma_copy(&mut self, blocks: u16) {
        for _ in 0..blocks {
            let dst = 0x8000 | (self.hdma_dst & 0x1FF0);
            let src = self.hdma_src & 0xFFF0;
            for i in 0..0x10 {
                let b = self.read(src.wrapping_add(i));
                self.ppu.write(dst.wrapping_add(i), b, &mut self.ints);
            }
            self.hdma_src = self.hdma_src.wrapping_add(0x10);
            self.hdma_dst = self.hdma_dst.wrapping_add(0x10);
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

/// The production adapter for the CPU's clocked [`CpuBus`](crate::memory::CpuBus)
/// seam. Each access advances the machine one M-cycle; the inherent
/// `read`/`write` (used by tooling and DMA) stay non-clocking.
impl crate::memory::CpuBus for Bus {
    fn read(&mut self, addr: u16) -> u8 {
        self.advance_m();
        Bus::read(self, addr)
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.advance_m();
        Bus::write(self, addr, val)
    }
    fn idle(&mut self) {
        self.advance_m();
    }
    fn peek(&self, addr: u16) -> u8 {
        Bus::read(self, addr)
    }
    fn poke(&mut self, addr: u16, val: u8) {
        Bus::write(self, addr, val)
    }
    fn elapsed(&self) -> u64 {
        self.elapsed
    }
    fn stop(&mut self) {
        // STOP performs a CGB speed switch only when KEY1 is armed.
        if self.key1 & 0x01 != 0 {
            self.double_speed = !self.double_speed;
            self.key1 &= !0x01;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::CpuBus;

    fn bus() -> Bus {
        let mut rom = vec![0u8; 0x8000];
        rom[0x147] = 0;
        Bus::new(Cartridge::from_rom(rom).unwrap())
    }

    #[test]
    fn key1_stop_switches_double_speed() {
        let mut b = bus();
        assert_eq!(b.read(0xFF4D) & 0x80, 0); // starts single speed
        b.write(0xFF4D, 0x01); // arm the switch
        CpuBus::stop(&mut b);
        assert_eq!(b.read(0xFF4D) & 0x80, 0x80); // now double speed
        assert_eq!(b.read(0xFF4D) & 0x01, 0); // armed bit cleared
        // A STOP without arming does not switch.
        CpuBus::stop(&mut b);
        assert_eq!(b.read(0xFF4D) & 0x80, 0x80);
    }

    #[test]
    fn double_speed_halves_ppu_stepping() {
        // 114 M-cycles = 456 dots = one scanline at single speed.
        let mut single = bus();
        for _ in 0..114 {
            single.advance_m();
        }
        assert_eq!(single.ppu.ly, 1);

        // At double speed the PPU only advances 2 dots per M-cycle, so the same
        // 114 M-cycles is 228 dots — still on line 0.
        let mut double = bus();
        double.write(0xFF4D, 0x01);
        CpuBus::stop(&mut double);
        for _ in 0..114 {
            double.advance_m();
        }
        assert_eq!(double.ppu.ly, 0);
    }

    #[test]
    fn gdma_copies_immediately() {
        let mut b = bus();
        for i in 0..0x10u16 {
            b.write(0xC000 + i, i as u8);
        }
        b.write(0xFF51, 0xC0); // src 0xC000
        b.write(0xFF52, 0x00);
        b.write(0xFF53, 0x00); // dst VRAM 0x8000
        b.write(0xFF54, 0x00);
        b.write(0xFF55, 0x00); // 1 block, bit 7 clear -> immediate
        assert_eq!(b.ppu.vram[0], 0x00);
        assert_eq!(b.ppu.vram[0x0F], 0x0F);
        assert_eq!(b.read(0xFF55), 0xFF); // idle/complete
    }

    #[test]
    fn hdma_hblank_steps_one_block_per_hblank() {
        let mut b = bus();
        for i in 0..0x20u16 {
            b.write(0xC000 + i, i as u8);
        }
        b.write(0xFF51, 0xC0);
        b.write(0xFF52, 0x00);
        b.write(0xFF53, 0x00);
        b.write(0xFF54, 0x00);
        b.write(0xFF55, 0x80 | 0x01); // HBlank DMA, 2 blocks
        assert_eq!(b.read(0xFF55), 0x01); // active, 2 blocks left (remaining-1)
        assert_eq!(b.ppu.vram[0], 0x00); // nothing copied yet

        // First visible-line HBlank starts at dot 252 = 63 M-cycles.
        for _ in 0..63 {
            b.advance_m();
        }
        assert_eq!(b.ppu.vram[0], 0x00);
        assert_eq!(b.ppu.vram[0x0F], 0x0F); // one block copied
        assert_eq!(b.read(0xFF55), 0x00); // 1 block left

        // Next line's HBlank copies the last block and completes.
        for _ in 0..114 {
            b.advance_m();
        }
        assert_eq!(b.ppu.vram[0x10], 0x10);
        assert_eq!(b.read(0xFF55), 0xFF); // complete
    }

    #[test]
    fn hdma_cancel_stops_transfer() {
        let mut b = bus();
        b.write(0xFF55, 0x80 | 0x03); // start a 4-block HBlank DMA
        assert_eq!(b.read(0xFF55) & 0x80, 0); // active
        b.write(0xFF55, 0x00); // bit 7 clear cancels it
        assert_eq!(b.read(0xFF55), 0xFF); // idle
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
