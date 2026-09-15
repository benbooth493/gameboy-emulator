//! The clocked bus seam the CPU drives.
//!
//! The SM83 core reads and writes bytes in the 16-bit address space; on real
//! hardware each such access is one M-cycle (4 T-cycles) during which the
//! timer, PPU, and APU advance. So `read`/`write`/`idle` here *advance time*:
//! they tick the rest of the machine by one M-cycle. This is what makes
//! peripherals move in lock-step within an instruction (cycle accuracy).
//!
//! `peek`/`poke` are the non-clocking escape hatch for CPU-internal queries
//! that do not consume a cycle — chiefly the interrupt-pending check and its
//! acknowledge, which happen "for free" around the instruction fetch.
//!
//! Production code satisfies this with [`crate::bus::Bus`]; tests use a flat
//! array whose clocking methods are no-ops.

pub trait CpuBus {
    /// Read one byte; advances the machine by one M-cycle.
    fn read(&mut self, addr: u16) -> u8;
    /// Write one byte; advances the machine by one M-cycle.
    fn write(&mut self, addr: u16, val: u8);
    /// An internal CPU M-cycle with no memory access; advances the machine.
    fn idle(&mut self);
    /// Non-clocking read (interrupt-controller queries); no time passes.
    fn peek(&self, addr: u16) -> u8;
    /// Non-clocking write (interrupt acknowledge); no time passes.
    fn poke(&mut self, addr: u16, val: u8);
    /// Total T-cycles the machine has advanced, for cycle accounting.
    fn elapsed(&self) -> u64;
}
