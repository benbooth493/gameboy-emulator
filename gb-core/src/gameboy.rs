//! Top-level machine: wires CPU + bus together and integrates the debugger.

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::debugger::{Debugger, StopReason};
use crate::joypad::Button;
use crate::ppu::{SCREEN_H, SCREEN_W};

pub const CYCLES_PER_FRAME: u32 = 70224;

#[derive(serde::Serialize, serde::Deserialize)]
pub struct GameBoy {
    pub cpu: Cpu,
    pub bus: Bus,
    /// Breakpoints/trace/pause are debugging aids, not machine state — excluded
    /// from save states and preserved across a load.
    #[serde(skip)]
    debugger: Debugger,
    pub cycles: u64,
}

/// Save-state header magic and format version.
const STATE_MAGIC: &[u8; 8] = b"GBEMSAVE";
const STATE_VERSION: u32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub enum StateError {
    BadMagic,
    VersionMismatch { found: u32, expected: u32 },
    RomMismatch,
    Corrupt,
}

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateError::BadMagic => write!(f, "not a gbem save state"),
            StateError::VersionMismatch { found, expected } => {
                write!(f, "save-state version {found} (expected {expected})")
            }
            StateError::RomMismatch => write!(f, "save state is from a different game"),
            StateError::Corrupt => write!(f, "save state is corrupt"),
        }
    }
}
impl std::error::Error for StateError {}

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
        if self.debugger.is_paused() {
            return Some(StopReason::Paused);
        }
        self.bus.ppu.frame_ready = false;
        let mut budget = CYCLES_PER_FRAME * 2; // safety margin (LCD may be off)
        while !self.bus.ppu.frame_ready && budget > 0 {
            budget = budget.saturating_sub(self.step());
            if self.debugger.has_breakpoint(self.cpu.regs.pc) {
                self.debugger.set_paused(true);
                return Some(StopReason::Breakpoint(self.cpu.regs.pc));
            }
        }
        None
    }

    // ---- run control ----

    pub fn is_paused(&self) -> bool {
        self.debugger.is_paused()
    }

    pub fn pause(&mut self) {
        self.debugger.set_paused(true);
    }

    pub fn resume(&mut self) {
        self.debugger.set_paused(false);
    }

    pub fn toggle_pause(&mut self) {
        let p = self.debugger.is_paused();
        self.debugger.set_paused(!p);
    }

    /// Execute exactly one instruction (regardless of the pause flag).
    pub fn step_instruction(&mut self) -> StopReason {
        self.step();
        StopReason::Step
    }

    /// Run a single frame even while paused, leaving the machine paused after.
    pub fn step_frame(&mut self) -> Option<StopReason> {
        let was_paused = self.debugger.is_paused();
        self.debugger.set_paused(false);
        let stop = self.run_frame();
        // A breakpoint mid-frame already re-paused us; otherwise restore state.
        if !matches!(stop, Some(StopReason::Breakpoint(_))) {
            self.debugger.set_paused(was_paused);
        }
        stop
    }

    // ---- breakpoints & trace (read-only views for a UI) ----

    pub fn toggle_breakpoint(&mut self, addr: u16) {
        self.debugger.toggle_breakpoint(addr);
    }

    pub fn has_breakpoint(&self, addr: u16) -> bool {
        self.debugger.has_breakpoint(addr)
    }

    /// Breakpoint addresses, ascending.
    pub fn breakpoints(&self) -> Vec<u16> {
        self.debugger.breakpoints()
    }

    /// Recently executed PCs, oldest first.
    pub fn trace(&self) -> Vec<u16> {
        self.debugger.trace()
    }

    /// RGBA8 framebuffer, one pixel per (x, y).
    pub fn framebuffer(&self) -> &[u8; SCREEN_W * SCREEN_H * 4] {
        &self.bus.ppu.framebuffer
    }

    pub fn set_button(&mut self, b: Button, pressed: bool) {
        self.bus.joypad.set_button(b, pressed, &mut self.bus.ints);
    }

    // ---- save states ----

    /// Serialize the full machine state (everything except the ROM, the
    /// transient audio/serial buffers, and the debugger). The result is bound
    /// to the loaded ROM's identity so it can only be restored onto the same
    /// game.
    pub fn save_state(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&STATE_VERSION.to_le_bytes());
        out.extend_from_slice(&self.bus.cart.rom_identity());
        // Excluded fields (rom, samples, serial_out, debugger) are `#[serde(skip)]`.
        let body = bincode::serialize(self).expect("machine state is serializable");
        out.extend_from_slice(&body);
        out
    }

    /// Restore machine state produced by [`save_state`]. The ROM, debugger, and
    /// transient buffers of the current machine are preserved.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), StateError> {
        const HEADER: usize = 8 + 4 + 18;
        if data.len() < 8 || &data[..8] != STATE_MAGIC {
            return Err(StateError::BadMagic);
        }
        if data.len() < HEADER {
            return Err(StateError::Corrupt);
        }
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version != STATE_VERSION {
            return Err(StateError::VersionMismatch {
                found: version,
                expected: STATE_VERSION,
            });
        }
        if data[12..30] != self.bus.cart.rom_identity() {
            return Err(StateError::RomMismatch);
        }
        let mut restored: GameBoy =
            bincode::deserialize(&data[HEADER..]).map_err(|_| StateError::Corrupt)?;
        // Re-attach the fields excluded from the snapshot.
        restored.bus.cart.rom = std::mem::take(&mut self.bus.cart.rom);
        std::mem::swap(&mut restored.debugger, &mut self.debugger);
        *self = restored;
        Ok(())
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
        gb.toggle_breakpoint(0x0102);
        let stop = gb.run_frame();
        assert_eq!(stop, Some(StopReason::Breakpoint(0x0102)));
        assert_eq!(gb.cpu.regs.pc, 0x0102);
        assert!(gb.is_paused());
    }

    #[test]
    fn paused_machine_does_not_run() {
        let mut gb = gb_with(&[0x00]);
        gb.pause();
        let pc = gb.cpu.regs.pc;
        assert_eq!(gb.run_frame(), Some(StopReason::Paused));
        assert_eq!(gb.cpu.regs.pc, pc);
    }

    #[test]
    fn step_instruction_advances_one_instruction() {
        let mut gb = gb_with(&[0x00, 0x00]);
        gb.pause();
        gb.step_instruction();
        assert_eq!(gb.cpu.regs.pc, 0x0101);
    }

    #[test]
    fn step_frame_runs_one_frame_and_stays_paused() {
        let mut gb = gb_with(&[0x18, 0xFE]); // spin
        gb.pause();
        let c0 = gb.cycles;
        let stop = gb.step_frame();
        assert!(stop.is_none());
        assert!(gb.cycles > c0); // a frame's worth of work happened
        assert!(gb.is_paused()); // ...and we're paused again
    }

    #[test]
    fn step_frame_stops_and_stays_paused_on_breakpoint() {
        let mut gb = gb_with(&[0x00, 0x00, 0x18, 0xFE]);
        gb.pause();
        gb.toggle_breakpoint(0x0102);
        let stop = gb.step_frame();
        assert_eq!(stop, Some(StopReason::Breakpoint(0x0102)));
        assert!(gb.is_paused());
    }

    #[test]
    fn toggle_pause_flips_state() {
        let mut gb = gb_with(&[0x00]);
        assert!(!gb.is_paused());
        gb.toggle_pause();
        assert!(gb.is_paused());
        gb.toggle_pause();
        assert!(!gb.is_paused());
    }

    fn gb_with_rom(code: &[u8], id_byte: u8) -> GameBoy {
        let mut rom = vec![0u8; 0x8000];
        rom[0x100..0x100 + code.len()].copy_from_slice(code);
        rom[0x134] = id_byte; // vary the title so ROM identities differ
        GameBoy::new(Cartridge::from_rom(rom).unwrap())
    }

    #[test]
    fn save_state_roundtrips() {
        let mut gb = gb_with_rom(&[0x18, 0xFE], b'A');
        for _ in 0..3 {
            gb.run_frame();
        }
        gb.cpu.regs.a = 0x99;
        gb.bus.wram[0x100] = 0x77;
        let snapshot = gb.save_state();
        let cycles = gb.cycles;

        // Diverge, then restore.
        gb.run_frame();
        gb.cpu.regs.a = 0x00;
        gb.bus.wram[0x100] = 0x00;

        gb.load_state(&snapshot).unwrap();
        assert_eq!(gb.cpu.regs.a, 0x99);
        assert_eq!(gb.bus.wram[0x100], 0x77);
        assert_eq!(gb.cycles, cycles);
        // ROM is preserved across a load.
        assert_eq!(gb.bus.read(0x0100), 0x18);
    }

    #[test]
    fn load_state_preserves_breakpoints() {
        let mut gb = gb_with_rom(&[0x00], b'A');
        gb.toggle_breakpoint(0x0123);
        let snapshot = gb.save_state();
        gb.load_state(&snapshot).unwrap();
        assert!(gb.has_breakpoint(0x0123), "debugger state survives a load");
    }

    #[test]
    fn load_state_rejects_bad_magic() {
        let mut gb = gb_with_rom(&[0x00], b'A');
        assert_eq!(gb.load_state(b"not a state"), Err(StateError::BadMagic));
    }

    #[test]
    fn load_state_rejects_other_game() {
        let snapshot = gb_with_rom(&[0x00], b'A').save_state();
        let mut other = gb_with_rom(&[0x00], b'B'); // different ROM identity
        assert_eq!(other.load_state(&snapshot), Err(StateError::RomMismatch));
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
