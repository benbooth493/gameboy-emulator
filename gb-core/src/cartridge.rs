//! Cartridge loading and MBC (memory bank controller) emulation.
//! Supports ROM-only, MBC1, MBC3 (incl. battery RAM), and MBC5.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MbcKind {
    None,
    Mbc1,
    Mbc3,
    Mbc5,
}

pub struct Cartridge {
    pub rom: Vec<u8>,
    pub ram: Vec<u8>,
    pub kind: MbcKind,
    pub title: String,
    rom_bank: usize,
    ram_bank: usize,
    ram_enabled: bool,
    banking_mode: u8, // MBC1 mode
    /// Set whenever cartridge RAM is written; the host clears it after
    /// persisting the RAM to a save file.
    ram_dirty: bool,
}

#[derive(Debug)]
pub enum CartError {
    TooSmall,
    UnsupportedMbc(u8),
}

impl std::fmt::Display for CartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CartError::TooSmall => write!(f, "ROM image too small"),
            CartError::UnsupportedMbc(t) => write!(f, "unsupported cartridge type {t:#04x}"),
        }
    }
}
impl std::error::Error for CartError {}

impl Cartridge {
    pub fn from_rom(rom: Vec<u8>) -> Result<Self, CartError> {
        if rom.len() < 0x150 {
            return Err(CartError::TooSmall);
        }
        let kind = match rom[0x147] {
            0x00 | 0x08 | 0x09 => MbcKind::None,
            0x01..=0x03 => MbcKind::Mbc1,
            0x0F..=0x13 => MbcKind::Mbc3,
            0x19..=0x1E => MbcKind::Mbc5,
            t => return Err(CartError::UnsupportedMbc(t)),
        };
        let ram_size = match rom[0x149] {
            0x02 => 0x2000,
            0x03 => 0x8000,
            0x04 => 0x20000,
            0x05 => 0x10000,
            _ => 0,
        };
        let title = rom[0x134..0x144]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '?' })
            .collect();
        Ok(Cartridge {
            rom,
            ram: vec![0; ram_size],
            kind,
            title,
            rom_bank: 1,
            ram_bank: 0,
            ram_enabled: false,
            banking_mode: 0,
            ram_dirty: false,
        })
    }

    fn rom_bank_count(&self) -> usize {
        (self.rom.len() / 0x4000).max(1)
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x3FFF => {
                let mut bank = 0usize;
                if self.kind == MbcKind::Mbc1 && self.banking_mode == 1 {
                    bank = (self.ram_bank << 5) % self.rom_bank_count();
                }
                self.rom.get(bank * 0x4000 + addr as usize).copied().unwrap_or(0xFF)
            }
            0x4000..=0x7FFF => {
                let mut bank = self.rom_bank % self.rom_bank_count();
                if self.kind == MbcKind::Mbc1 {
                    bank = ((self.ram_bank << 5) | self.rom_bank) % self.rom_bank_count();
                }
                self.rom
                    .get(bank * 0x4000 + (addr as usize - 0x4000))
                    .copied()
                    .unwrap_or(0xFF)
            }
            0xA000..=0xBFFF => {
                if !self.ram_enabled || self.ram.is_empty() {
                    return 0xFF;
                }
                let idx = self.ram_bank * 0x2000 + (addr as usize - 0xA000);
                self.ram.get(idx).copied().unwrap_or(0xFF)
            }
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8) {
        match self.kind {
            MbcKind::None => {
                if let 0xA000..=0xBFFF = addr {
                    if !self.ram.is_empty() {
                        let idx = (addr as usize - 0xA000) % self.ram.len();
                        self.ram[idx] = val;
                        self.ram_dirty = true;
                    }
                }
            }
            MbcKind::Mbc1 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = val & 0x0F == 0x0A,
                0x2000..=0x3FFF => {
                    let v = (val & 0x1F) as usize;
                    self.rom_bank = if v == 0 { 1 } else { v };
                }
                0x4000..=0x5FFF => self.ram_bank = (val & 0x03) as usize,
                0x6000..=0x7FFF => self.banking_mode = val & 1,
                0xA000..=0xBFFF => self.write_ram(addr, val),
                _ => {}
            },
            MbcKind::Mbc3 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = val & 0x0F == 0x0A,
                0x2000..=0x3FFF => {
                    let v = (val & 0x7F) as usize;
                    self.rom_bank = if v == 0 { 1 } else { v };
                }
                0x4000..=0x5FFF => {
                    // 0x08-0x0C select RTC registers; we map them to no RAM.
                    self.ram_bank = (val & 0x03) as usize;
                }
                0xA000..=0xBFFF => self.write_ram(addr, val),
                _ => {}
            },
            MbcKind::Mbc5 => match addr {
                0x0000..=0x1FFF => self.ram_enabled = val & 0x0F == 0x0A,
                0x2000..=0x2FFF => {
                    self.rom_bank = (self.rom_bank & 0x100) | val as usize;
                }
                0x3000..=0x3FFF => {
                    self.rom_bank = (self.rom_bank & 0xFF) | (((val & 1) as usize) << 8);
                }
                0x4000..=0x5FFF => self.ram_bank = (val & 0x0F) as usize,
                0xA000..=0xBFFF => self.write_ram(addr, val),
                _ => {}
            },
        }
    }

    fn write_ram(&mut self, addr: u16, val: u8) {
        if !self.ram_enabled || self.ram.is_empty() {
            return;
        }
        let idx = self.ram_bank * 0x2000 + (addr as usize - 0xA000);
        if idx < self.ram.len() {
            self.ram[idx] = val;
            self.ram_dirty = true;
        }
    }

    /// Whether this cartridge has battery-backed RAM worth persisting.
    pub fn has_battery(&self) -> bool {
        matches!(
            self.rom.get(0x147).copied().unwrap_or(0),
            0x03 | 0x09 | 0x0F | 0x10 | 0x13 | 0x1B | 0x1E
        )
    }

    /// The battery-backed RAM, for writing to a save file.
    pub fn ram(&self) -> &[u8] {
        &self.ram
    }

    /// Restore battery-backed RAM from a save file. Ignored unless the sizes
    /// match, so a stale or foreign save can't corrupt state.
    pub fn load_ram(&mut self, data: &[u8]) {
        if data.len() == self.ram.len() {
            self.ram.copy_from_slice(data);
            self.ram_dirty = false;
        }
    }

    /// Return whether RAM changed since the last call, clearing the flag.
    /// The host uses this to decide when to flush a save file.
    pub fn take_ram_dirty(&mut self) -> bool {
        std::mem::take(&mut self.ram_dirty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rom(kind: u8, ram: u8, banks: usize) -> Vec<u8> {
        let mut r = vec![0u8; 0x4000 * banks];
        r[0x147] = kind;
        r[0x149] = ram;
        // Stamp each bank with its index for identification.
        for b in 0..banks {
            r[b * 0x4000 + 0x1000] = b as u8;
        }
        r
    }

    #[test]
    fn rejects_tiny_rom() {
        assert!(Cartridge::from_rom(vec![0; 16]).is_err());
    }

    #[test]
    fn has_battery_detects_battery_types() {
        // 0x03 = MBC1+RAM+BATTERY; 0x01 = MBC1 (no battery).
        assert!(Cartridge::from_rom(rom(0x03, 0x02, 2)).unwrap().has_battery());
        assert!(!Cartridge::from_rom(rom(0x01, 0x02, 2)).unwrap().has_battery());
    }

    #[test]
    fn ram_writes_set_dirty_flag() {
        let mut c = Cartridge::from_rom(rom(0x03, 0x02, 2)).unwrap();
        assert!(!c.take_ram_dirty());
        c.write(0x0000, 0x0A); // enable RAM
        c.write(0xA000, 0x42);
        assert!(c.take_ram_dirty());
        assert!(!c.take_ram_dirty(), "flag cleared after being taken");
    }

    #[test]
    fn save_and_restore_ram_roundtrips() {
        let mut c = Cartridge::from_rom(rom(0x03, 0x02, 2)).unwrap();
        c.write(0x0000, 0x0A);
        c.write(0xA000, 0xAB);
        c.write(0xA001, 0xCD);
        let saved = c.ram().to_vec();

        let mut fresh = Cartridge::from_rom(rom(0x03, 0x02, 2)).unwrap();
        fresh.load_ram(&saved);
        fresh.write(0x0000, 0x0A); // enable to read back
        assert_eq!(fresh.read(0xA000), 0xAB);
        assert_eq!(fresh.read(0xA001), 0xCD);
        assert!(!fresh.take_ram_dirty(), "loading a save is not a dirtying write");
    }

    #[test]
    fn load_ram_ignores_size_mismatch() {
        let mut c = Cartridge::from_rom(rom(0x03, 0x02, 2)).unwrap(); // 8 KiB RAM
        c.load_ram(&[0xFF; 4]); // wrong size: ignored
        c.write(0x0000, 0x0A);
        assert_eq!(c.read(0xA000), 0x00);
    }

    #[test]
    fn rom_only_reads_flat() {
        let c = Cartridge::from_rom(rom(0x00, 0, 2)).unwrap();
        assert_eq!(c.read(0x1000), 0);
        assert_eq!(c.read(0x5000), 1);
    }

    #[test]
    fn mbc1_bank_switching() {
        let mut c = Cartridge::from_rom(rom(0x01, 0, 8)).unwrap();
        assert_eq!(c.read(0x5000), 1); // default bank 1
        c.write(0x2000, 5);
        assert_eq!(c.read(0x5000), 5);
        c.write(0x2000, 0); // bank 0 maps to 1
        assert_eq!(c.read(0x5000), 1);
    }

    #[test]
    fn mbc1_ram_enable_gate() {
        let mut c = Cartridge::from_rom(rom(0x03, 0x02, 2)).unwrap();
        c.write(0xA000, 0x42);
        assert_eq!(c.read(0xA000), 0xFF); // disabled
        c.write(0x0000, 0x0A);
        c.write(0xA000, 0x42);
        assert_eq!(c.read(0xA000), 0x42);
        c.write(0x0000, 0x00);
        assert_eq!(c.read(0xA000), 0xFF);
    }

    #[test]
    fn mbc5_nine_bit_rom_bank() {
        let mut c = Cartridge::from_rom(rom(0x19, 0, 4)).unwrap();
        c.write(0x2000, 3);
        assert_eq!(c.read(0x5000), 3);
        // 9th bit wraps modulo bank count
        c.write(0x3000, 1);
        assert_eq!(c.read(0x5000), (256usize + 3) as u8 % 4);
    }

    #[test]
    fn mbc5_bank_zero_is_real_zero() {
        let mut c = Cartridge::from_rom(rom(0x19, 0, 4)).unwrap();
        c.write(0x2000, 0);
        assert_eq!(c.read(0x5000), 0);
    }

    #[test]
    fn title_parsed() {
        let mut r = rom(0x00, 0, 2);
        r[0x134..0x13A].copy_from_slice(b"TETRIS");
        let c = Cartridge::from_rom(r).unwrap();
        assert_eq!(c.title, "TETRIS");
    }
}
