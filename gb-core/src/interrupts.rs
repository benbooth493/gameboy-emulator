//! Interrupt controller: IF (0xFF0F) / IE (0xFFFF) registers.

pub const INT_VBLANK: u8 = 1 << 0;
pub const INT_STAT: u8 = 1 << 1;
pub const INT_TIMER: u8 = 1 << 2;
pub const INT_SERIAL: u8 = 1 << 3;
pub const INT_JOYPAD: u8 = 1 << 4;

/// The interrupt-enable (IE, 0xFFFF) and interrupt-request (IF, 0xFF0F)
/// registers. Peripherals `raise` bits here; the CPU reads them through the
/// memory seam and owns the priority/vector dispatch.
#[derive(Debug, Default, Clone)]
pub struct Interrupts {
    pub enable: u8,  // IE
    pub request: u8, // IF
}

impl Interrupts {
    /// Bitmask of interrupts that are both enabled and requested.
    pub fn pending(&self) -> u8 {
        self.enable & self.request & 0x1F
    }

    pub fn raise(&mut self, mask: u8) {
        self.request |= mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pending_when_masked() {
        let mut i = Interrupts::default();
        i.raise(INT_TIMER);
        assert_eq!(i.pending(), 0);
    }

    #[test]
    fn pending_requires_enable_and_request() {
        let mut i = Interrupts::default();
        i.enable = 0x1F;
        i.raise(INT_TIMER | INT_VBLANK);
        assert_eq!(i.pending(), INT_TIMER | INT_VBLANK);
    }
}
