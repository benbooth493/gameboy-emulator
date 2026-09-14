//! Pure, platform-independent Game Boy (DMG) emulator core.
//!
//! No I/O, no clocks, no threads: callers drive the emulation by calling
//! [`GameBoy::step`] / [`GameBoy::run_frame`] and read the PPU framebuffer
//! and APU sample ring out of the machine state.

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod debugger;
pub mod disasm;
pub mod gameboy;
pub mod interrupts;
pub mod joypad;
pub mod memory;
pub mod ppu;
pub mod timer;

pub use cartridge::Cartridge;
pub use debugger::Debugger;
pub use gameboy::GameBoy;
pub use joypad::Button;
pub use memory::Memory;
pub use ppu::{SCREEN_H, SCREEN_W};
