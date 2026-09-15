//! Joypad register (0xFF00). Buttons are active-low.

use crate::interrupts::{Interrupts, INT_JOYPAD};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Right,
    Left,
    Up,
    Down,
    A,
    B,
    Select,
    Start,
}

#[derive(Debug, Clone)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Joypad {
    select: u8,     // bits 4-5 of P1 as written by the game
    dpad: u8,       // pressed = 1 in our internal state
    buttons: u8,
}

impl Default for Joypad {
    fn default() -> Self {
        Joypad { select: 0x30, dpad: 0, buttons: 0 }
    }
}

impl Joypad {
    pub fn read(&self) -> u8 {
        let mut v = 0xC0 | self.select | 0x0F;
        if self.select & 0x10 == 0 {
            v &= !(self.dpad & 0x0F) | 0xF0;
        }
        if self.select & 0x20 == 0 {
            v &= !(self.buttons & 0x0F) | 0xF0;
        }
        v
    }

    pub fn write(&mut self, val: u8) {
        self.select = val & 0x30;
    }

    pub fn set_button(&mut self, b: Button, pressed: bool, ints: &mut Interrupts) {
        let (field, bit) = match b {
            Button::Right => (&mut self.dpad, 0),
            Button::Left => (&mut self.dpad, 1),
            Button::Up => (&mut self.dpad, 2),
            Button::Down => (&mut self.dpad, 3),
            Button::A => (&mut self.buttons, 0),
            Button::B => (&mut self.buttons, 1),
            Button::Select => (&mut self.buttons, 2),
            Button::Start => (&mut self.buttons, 3),
        };
        let was = *field & (1 << bit) != 0;
        if pressed {
            *field |= 1 << bit;
            if !was {
                ints.raise(INT_JOYPAD);
            }
        } else {
            *field &= !(1 << bit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unselected_reads_all_released() {
        let j = Joypad::default();
        assert_eq!(j.read() & 0x0F, 0x0F);
    }

    #[test]
    fn dpad_reads_active_low_when_selected() {
        let mut j = Joypad::default();
        let mut i = Interrupts::default();
        j.set_button(Button::Left, true, &mut i);
        j.write(0x20); // select d-pad (bit 4 low)
        assert_eq!(j.read() & 0x0F, 0b1101);
        // action buttons view unaffected
        j.write(0x10);
        assert_eq!(j.read() & 0x0F, 0x0F);
    }

    #[test]
    fn press_raises_joypad_interrupt() {
        let mut j = Joypad::default();
        let mut i = Interrupts::default();
        i.enable = 0xFF;
        j.set_button(Button::Start, true, &mut i);
        assert!(i.pending() & INT_JOYPAD != 0);
    }
}
