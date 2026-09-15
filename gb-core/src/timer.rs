//! DIV/TIMA/TMA/TAC timer (0xFF04-0xFF07), driven by T-cycles.

use crate::interrupts::{Interrupts, INT_TIMER};

#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Timer {
    /// 16-bit internal divider; DIV is the upper 8 bits.
    div: u16,
    pub tima: u8,
    pub tma: u8,
    pub tac: u8,
    /// TIMA overflowed and is waiting one M-cycle to reload from TMA.
    overflow_delay: u8,
}

impl Default for Timer {
    fn default() -> Self {
        Timer {
            div: 0xABCC, // approximate post-boot value
            tima: 0,
            tma: 0,
            tac: 0xF8,
            overflow_delay: 0,
        }
    }
}

impl Timer {
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8,
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, val: u8, ints: &mut Interrupts) {
        match addr {
            0xFF04 => {
                // Writing DIV resets the whole internal counter; a falling
                // edge on the selected bit ticks TIMA.
                let old = self.div;
                self.div = 0;
                if self.enabled() && Self::selected_bit(old, self.tac) {
                    self.increment_tima(ints);
                }
            }
            0xFF05 => {
                self.tima = val;
                self.overflow_delay = 0;
            }
            0xFF06 => self.tma = val,
            0xFF07 => {
                let old_signal = self.enabled() && Self::selected_bit(self.div, self.tac);
                self.tac = val & 0x07;
                let new_signal = self.enabled() && Self::selected_bit(self.div, self.tac);
                if old_signal && !new_signal {
                    self.increment_tima(ints);
                }
            }
            _ => {}
        }
    }

    fn enabled(&self) -> bool {
        self.tac & 0x04 != 0
    }

    fn selected_bit(div: u16, tac: u8) -> bool {
        let bit = match tac & 0x03 {
            0 => 9, // 4096 Hz
            1 => 3, // 262144 Hz
            2 => 5, // 65536 Hz
            _ => 7, // 16384 Hz
        };
        div & (1 << bit) != 0
    }

    fn increment_tima(&mut self, _ints: &mut Interrupts) {
        let (v, carry) = self.tima.overflowing_add(1);
        self.tima = v;
        if carry {
            // Reload + interrupt happens 4 T-cycles later.
            self.overflow_delay = 4;
        }
    }

    /// Advance by `cycles` T-cycles.
    pub fn tick(&mut self, cycles: u32, ints: &mut Interrupts) {
        for _ in 0..cycles {
            if self.overflow_delay > 0 {
                self.overflow_delay -= 1;
                if self.overflow_delay == 0 {
                    self.tima = self.tma;
                    ints.raise(INT_TIMER);
                }
            }
            let old = self.div;
            self.div = self.div.wrapping_add(1);
            if self.enabled()
                && Self::selected_bit(old, self.tac)
                && !Self::selected_bit(self.div, self.tac)
            {
                self.increment_tima(ints);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (Timer, Interrupts) {
        let mut t = Timer::default();
        t.div = 0;
        (t, Interrupts::default())
    }

    #[test]
    fn div_increments_every_256_cycles() {
        let (mut t, mut i) = fresh();
        t.tick(255, &mut i);
        assert_eq!(t.read(0xFF04), 0);
        t.tick(1, &mut i);
        assert_eq!(t.read(0xFF04), 1);
    }

    #[test]
    fn div_write_resets() {
        let (mut t, mut i) = fresh();
        t.tick(512, &mut i);
        assert_eq!(t.read(0xFF04), 2);
        t.write(0xFF04, 0x55, &mut i);
        assert_eq!(t.read(0xFF04), 0);
    }

    #[test]
    fn tima_ticks_at_selected_rate() {
        let (mut t, mut i) = fresh();
        t.write(0xFF07, 0b101, &mut i); // enabled, 262144 Hz => every 16 T-cycles
        t.tick(16 * 10, &mut i);
        assert_eq!(t.tima, 10);
    }

    #[test]
    fn tima_overflow_reloads_tma_and_raises_interrupt() {
        let (mut t, mut i) = fresh();
        i.enable = 0xFF;
        t.write(0xFF06, 0x42, &mut i); // TMA
        t.write(0xFF07, 0b101, &mut i);
        t.tima = 0xFF;
        t.tick(16 + 4, &mut i); // overflow + 4-cycle reload delay
        assert_eq!(t.tima, 0x42);
        assert!(i.pending() & INT_TIMER != 0);
    }

    #[test]
    fn disabled_timer_does_not_tick_tima() {
        let (mut t, mut i) = fresh();
        t.write(0xFF07, 0b001, &mut i); // rate set but disabled
        t.tick(1024, &mut i);
        assert_eq!(t.tima, 0);
    }
}
