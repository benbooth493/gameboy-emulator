//! SM83 register file.

pub const FLAG_Z: u8 = 0x80;
pub const FLAG_N: u8 = 0x40;
pub const FLAG_H: u8 = 0x20;
pub const FLAG_C: u8 = 0x10;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
}

impl Registers {
    /// Post-boot-ROM state for the original DMG.
    pub fn dmg() -> Self {
        Registers {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
        }
    }

    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }

    pub fn set_af(&mut self, v: u16) {
        self.a = (v >> 8) as u8;
        self.f = (v as u8) & 0xF0; // low nibble of F is always zero
    }
    pub fn set_bc(&mut self, v: u16) {
        self.b = (v >> 8) as u8;
        self.c = v as u8;
    }
    pub fn set_de(&mut self, v: u16) {
        self.d = (v >> 8) as u8;
        self.e = v as u8;
    }
    pub fn set_hl(&mut self, v: u16) {
        self.h = (v >> 8) as u8;
        self.l = v as u8;
    }

    pub fn flag(&self, mask: u8) -> bool {
        self.f & mask != 0
    }

    pub fn set_flag(&mut self, mask: u8, on: bool) {
        if on {
            self.f |= mask;
        } else {
            self.f &= !mask;
        }
        self.f &= 0xF0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_registers_roundtrip() {
        let mut r = Registers::default();
        r.set_bc(0xBEEF);
        assert_eq!(r.b, 0xBE);
        assert_eq!(r.c, 0xEF);
        assert_eq!(r.bc(), 0xBEEF);
        r.set_hl(0x1234);
        assert_eq!(r.hl(), 0x1234);
    }

    #[test]
    fn f_low_nibble_forced_zero() {
        let mut r = Registers::default();
        r.set_af(0xABCD);
        assert_eq!(r.f, 0xC0);
        assert_eq!(r.af(), 0xABC0);
    }

    #[test]
    fn flag_helpers() {
        let mut r = Registers::default();
        r.set_flag(FLAG_Z, true);
        r.set_flag(FLAG_C, true);
        assert!(r.flag(FLAG_Z));
        assert!(r.flag(FLAG_C));
        assert!(!r.flag(FLAG_N));
        r.set_flag(FLAG_Z, false);
        assert!(!r.flag(FLAG_Z));
    }
}
