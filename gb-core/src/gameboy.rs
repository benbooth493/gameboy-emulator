//! Top-level machine: wires CPU + bus together and integrates the debugger.

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::debugger::{Debugger, StopReason};
use crate::joypad::Button;
use crate::ppu::{SCREEN_H, SCREEN_W};

pub const CYCLES_PER_FRAME: u32 = 70224;

pub struct GameBoy {
    pub cpu: Cpu,
    pub bus: Bus,
    pub debugger: Debugger,
    pub cycles: u64,
}

impl GameBoy {
    pub fn new(cart: Cartridge) -> Self {
        GameBoy {
            cpu: Cpu::new(),
            bus: Bus::new(cart),
            debugger: Debugger::new(),
            cycles: 0,
        }
    }

    /// Execute a single instruction and advance all hardware.
    pub fn step(&mut self) -> u32 {
        self.debugger.record_pc(self.cpu.regs.pc);
        let cycles = self.cpu.step(&mut self.bus);
        self.bus.tick(cycles);
        self.cycles += cycles as u64;
        cycles
    }

    /// Run until a full frame is rendered, a breakpoint is hit, or the
    /// debugger pauses. Returns the reason the run stopped, if any.
    pub fn run_frame(&mut self) -> Option<StopReason> {
        if self.debugger.paused {
            return Some(StopReason::Paused);
        }
        self.bus.ppu.frame_ready = false;
        let mut budget = CYCLES_PER_FRAME * 2; // safety margin (LCD may be off)
        while !self.bus.ppu.frame_ready && budget > 0 {
            budget = budget.saturating_sub(self.step());
            if self.debugger.has_breakpoint(self.cpu.regs.pc) {
                self.debugger.paused = true;
                return Some(StopReason::Breakpoint(self.cpu.regs.pc));
            }
        }
        None
    }

    /// Single-step one instruction while paused.
    pub fn debug_step(&mut self) -> StopReason {
        self.step();
        StopReason::Step
    }

    pub fn framebuffer(&self) -> &[u8; SCREEN_W * SCREEN_H] {
        &self.bus.ppu.framebuffer
    }

    pub fn set_button(&mut self, b: Button, pressed: bool) {
        self.bus.joypad.set_button(b, pressed, &mut self.bus.ints);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gb_with(code: &[u8]) -> GameBoy {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x100 + code.len()].copy_from_slice(code);
        GameBoy::new(Cartridge::from_rom(rom).unwrap())
    }

    #[test]
    fn run_frame_produces_frame() {
        let mut gb = gb_with(&[0x18, 0xFE]); // JR -2: spin forever
        assert!(gb.run_frame().is_none());
        assert!(gb.cycles >= 65664); // at least 144 visible lines
    }

    #[test]
    fn breakpoint_stops_run() {
        // NOP; NOP; JR -2
        let mut gb = gb_with(&[0x00, 0x00, 0x18, 0xFE]);
        gb.debugger.toggle_breakpoint(0x0102);
        let stop = gb.run_frame();
        assert_eq!(stop, Some(StopReason::Breakpoint(0x0102)));
        assert_eq!(gb.cpu.regs.pc, 0x0102);
        assert!(gb.debugger.paused);
    }

    #[test]
    fn paused_machine_does_not_run() {
        let mut gb = gb_with(&[0x00]);
        gb.debugger.paused = true;
        let pc = gb.cpu.regs.pc;
        assert_eq!(gb.run_frame(), Some(StopReason::Paused));
        assert_eq!(gb.cpu.regs.pc, pc);
    }

    #[test]
    fn debug_step_advances_one_instruction() {
        let mut gb = gb_with(&[0x00, 0x00]);
        gb.debugger.paused = true;
        gb.debug_step();
        assert_eq!(gb.cpu.regs.pc, 0x0101);
    }

    #[test]
    fn timer_and_ppu_advance_with_cpu() {
        let mut gb = gb_with(&[0x18, 0xFE]);
        let div0 = gb.bus.read(0xFF04);
        gb.run_frame();
        assert_ne!(gb.bus.read(0xFF04), div0);
        assert!(gb.bus.ppu.frame_ready);
    }
}
