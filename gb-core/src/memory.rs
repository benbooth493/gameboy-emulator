//! The memory seam the CPU is written against.
//!
//! The SM83 core only needs to read and write bytes in the 16-bit address
//! space; everything else it observes (VRAM, IO registers, the interrupt
//! flags at 0xFF0F / 0xFFFF) is reached through this interface. Production
//! code satisfies it with [`crate::bus::Bus`]; tests satisfy it with a flat
//! array. Reads are `&self` because they never mutate machine state.

pub trait Memory {
    fn read(&self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, val: u8);
}
