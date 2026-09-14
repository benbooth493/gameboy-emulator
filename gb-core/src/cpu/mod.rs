//! SM83 (Game Boy CPU) core: full instruction set, interrupts, HALT.

pub mod registers;

use crate::memory::Memory;
use registers::{Registers, FLAG_C, FLAG_H, FLAG_N, FLAG_Z};

/// Interrupt vector for each IF/IE bit, lowest bit first (highest priority).
const VECTORS: [u16; 5] = [0x40, 0x48, 0x50, 0x58, 0x60];

/// IF (interrupt request) and IE (interrupt enable) live in the memory map.
const IF_ADDR: u16 = 0xFF0F;
const IE_ADDR: u16 = 0xFFFF;

#[derive(Default)]
pub struct Cpu {
    pub regs: Registers,
    pub ime: bool,
    ei_pending: bool,
    pub halted: bool,
    halt_bug: bool,
}

impl Cpu {
    pub fn new() -> Self {
        Cpu {
            regs: Registers::dmg(),
            ime: false,
            ei_pending: false,
            halted: false,
            halt_bug: false,
        }
    }

    /// Interrupts that are both enabled (IE) and requested (IF), read through
    /// the memory seam. Bit set = pending.
    fn pending_interrupts(&self, mem: &impl Memory) -> u8 {
        mem.read(IE_ADDR) & mem.read(IF_ADDR) & 0x1F
    }

    /// Execute one instruction (or service an interrupt / idle in HALT).
    /// Returns T-cycles consumed. The caller ticks the bus.
    pub fn step<M: Memory>(&mut self, mem: &mut M) -> u32 {
        // EI takes effect after the following instruction.
        let enable_ime_after = self.ei_pending;

        let pending = self.pending_interrupts(mem);
        if self.ime {
            if pending != 0 {
                // Highest priority = lowest set bit.
                let bit = pending.trailing_zeros() as u8;
                self.halted = false;
                self.ime = false;
                self.ei_pending = false;
                // Acknowledge: clear this bit in IF.
                let if_val = mem.read(IF_ADDR) & !(1 << bit);
                mem.write(IF_ADDR, if_val);
                self.push16(mem, self.regs.pc);
                self.regs.pc = VECTORS[bit as usize];
                return 20;
            }
        } else if self.halted && pending != 0 {
            // Wake from HALT without servicing.
            self.halted = false;
        }

        if self.halted {
            return 4;
        }

        let opcode = self.fetch8(mem);
        if self.halt_bug {
            // PC failed to increment for this fetch.
            self.regs.pc = self.regs.pc.wrapping_sub(1);
            self.halt_bug = false;
        }
        let cycles = self.execute(opcode, mem);

        if enable_ime_after && self.ei_pending {
            self.ime = true;
            self.ei_pending = false;
        }
        cycles
    }

    // ---- memory helpers ----

    fn fetch8(&mut self, mem: &mut impl Memory) -> u8 {
        let v = mem.read(self.regs.pc);
        self.regs.pc = self.regs.pc.wrapping_add(1);
        v
    }

    fn fetch16(&mut self, mem: &mut impl Memory) -> u16 {
        let lo = self.fetch8(mem) as u16;
        let hi = self.fetch8(mem) as u16;
        hi << 8 | lo
    }

    fn push16(&mut self, mem: &mut impl Memory, v: u16) {
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        mem.write(self.regs.sp, (v >> 8) as u8);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        mem.write(self.regs.sp, v as u8);
    }

    fn pop16(&mut self, mem: &mut impl Memory) -> u16 {
        let lo = mem.read(self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = mem.read(self.regs.sp) as u16;
        self.regs.sp = self.regs.sp.wrapping_add(1);
        hi << 8 | lo
    }

    // ---- 8-bit operand access by index (B,C,D,E,H,L,(HL),A) ----

    fn get_r8(&mut self, idx: u8, mem: &mut impl Memory) -> u8 {
        match idx {
            0 => self.regs.b,
            1 => self.regs.c,
            2 => self.regs.d,
            3 => self.regs.e,
            4 => self.regs.h,
            5 => self.regs.l,
            6 => mem.read(self.regs.hl()),
            _ => self.regs.a,
        }
    }

    fn set_r8(&mut self, idx: u8, val: u8, mem: &mut impl Memory) {
        match idx {
            0 => self.regs.b = val,
            1 => self.regs.c = val,
            2 => self.regs.d = val,
            3 => self.regs.e = val,
            4 => self.regs.h = val,
            5 => self.regs.l = val,
            6 => mem.write(self.regs.hl(), val),
            _ => self.regs.a = val,
        }
    }

    // ---- ALU ----

    fn add(&mut self, val: u8, carry: bool) {
        let c = (carry && self.regs.flag(FLAG_C)) as u16;
        let a = self.regs.a as u16;
        let r = a + val as u16 + c;
        self.regs.set_flag(FLAG_Z, r & 0xFF == 0);
        self.regs.set_flag(FLAG_N, false);
        self.regs.set_flag(FLAG_H, (a & 0xF) + (val as u16 & 0xF) + c > 0xF);
        self.regs.set_flag(FLAG_C, r > 0xFF);
        self.regs.a = r as u8;
    }

    fn sub(&mut self, val: u8, carry: bool, store: bool) {
        let c = (carry && self.regs.flag(FLAG_C)) as i16;
        let a = self.regs.a as i16;
        let r = a - val as i16 - c;
        self.regs.set_flag(FLAG_Z, r & 0xFF == 0);
        self.regs.set_flag(FLAG_N, true);
        self.regs.set_flag(FLAG_H, (a & 0xF) - (val as i16 & 0xF) - c < 0);
        self.regs.set_flag(FLAG_C, r < 0);
        if store {
            self.regs.a = r as u8;
        }
    }

    fn and(&mut self, val: u8) {
        self.regs.a &= val;
        let z = self.regs.a == 0;
        self.regs.f = 0;
        self.regs.set_flag(FLAG_Z, z);
        self.regs.set_flag(FLAG_H, true);
    }

    fn or(&mut self, val: u8) {
        self.regs.a |= val;
        let z = self.regs.a == 0;
        self.regs.f = 0;
        self.regs.set_flag(FLAG_Z, z);
    }

    fn xor(&mut self, val: u8) {
        self.regs.a ^= val;
        let z = self.regs.a == 0;
        self.regs.f = 0;
        self.regs.set_flag(FLAG_Z, z);
    }

    fn inc8(&mut self, val: u8) -> u8 {
        let r = val.wrapping_add(1);
        self.regs.set_flag(FLAG_Z, r == 0);
        self.regs.set_flag(FLAG_N, false);
        self.regs.set_flag(FLAG_H, val & 0xF == 0xF);
        r
    }

    fn dec8(&mut self, val: u8) -> u8 {
        let r = val.wrapping_sub(1);
        self.regs.set_flag(FLAG_Z, r == 0);
        self.regs.set_flag(FLAG_N, true);
        self.regs.set_flag(FLAG_H, val & 0xF == 0);
        r
    }

    fn add_hl(&mut self, val: u16) {
        let hl = self.regs.hl();
        let r = hl.wrapping_add(val);
        self.regs.set_flag(FLAG_N, false);
        self.regs.set_flag(FLAG_H, (hl & 0xFFF) + (val & 0xFFF) > 0xFFF);
        self.regs.set_flag(FLAG_C, (hl as u32) + (val as u32) > 0xFFFF);
        self.regs.set_hl(r);
    }

    fn add_sp_e8(&mut self, mem: &mut impl Memory) -> u16 {
        let e = self.fetch8(mem) as i8 as i16 as u16;
        let sp = self.regs.sp;
        self.regs.f = 0;
        self.regs.set_flag(FLAG_H, (sp & 0xF) + (e & 0xF) > 0xF);
        self.regs.set_flag(FLAG_C, (sp & 0xFF) + (e & 0xFF) > 0xFF);
        sp.wrapping_add(e)
    }

    fn daa(&mut self) {
        let mut a = self.regs.a;
        let mut adjust = 0u8;
        let mut carry = self.regs.flag(FLAG_C);
        if self.regs.flag(FLAG_H) || (!self.regs.flag(FLAG_N) && a & 0xF > 9) {
            adjust |= 0x06;
        }
        if carry || (!self.regs.flag(FLAG_N) && a > 0x99) {
            adjust |= 0x60;
            carry = true;
        }
        a = if self.regs.flag(FLAG_N) {
            a.wrapping_sub(adjust)
        } else {
            a.wrapping_add(adjust)
        };
        self.regs.set_flag(FLAG_Z, a == 0);
        self.regs.set_flag(FLAG_H, false);
        self.regs.set_flag(FLAG_C, carry);
        self.regs.a = a;
    }

    // ---- rotates/shifts (CB and A-register variants) ----

    fn rlc(&mut self, v: u8) -> u8 {
        let r = v.rotate_left(1);
        self.set_rot_flags(r, v & 0x80 != 0);
        r
    }
    fn rrc(&mut self, v: u8) -> u8 {
        let r = v.rotate_right(1);
        self.set_rot_flags(r, v & 1 != 0);
        r
    }
    fn rl(&mut self, v: u8) -> u8 {
        let r = (v << 1) | self.regs.flag(FLAG_C) as u8;
        self.set_rot_flags(r, v & 0x80 != 0);
        r
    }
    fn rr(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | ((self.regs.flag(FLAG_C) as u8) << 7);
        self.set_rot_flags(r, v & 1 != 0);
        r
    }
    fn sla(&mut self, v: u8) -> u8 {
        let r = v << 1;
        self.set_rot_flags(r, v & 0x80 != 0);
        r
    }
    fn sra(&mut self, v: u8) -> u8 {
        let r = (v >> 1) | (v & 0x80);
        self.set_rot_flags(r, v & 1 != 0);
        r
    }
    fn swap(&mut self, v: u8) -> u8 {
        let r = v.rotate_left(4);
        self.set_rot_flags(r, false);
        r
    }
    fn srl(&mut self, v: u8) -> u8 {
        let r = v >> 1;
        self.set_rot_flags(r, v & 1 != 0);
        r
    }

    fn set_rot_flags(&mut self, result: u8, carry: bool) {
        self.regs.f = 0;
        self.regs.set_flag(FLAG_Z, result == 0);
        self.regs.set_flag(FLAG_C, carry);
    }

    fn condition(&self, idx: u8) -> bool {
        match idx {
            0 => !self.regs.flag(FLAG_Z),
            1 => self.regs.flag(FLAG_Z),
            2 => !self.regs.flag(FLAG_C),
            _ => self.regs.flag(FLAG_C),
        }
    }

    // ---- dispatch ----

    fn execute(&mut self, opcode: u8, mem: &mut impl Memory) -> u32 {
        match opcode {
            0x00 => 4, // NOP
            0x10 => {
                // STOP: consume the padding byte.
                self.fetch8(mem);
                4
            }
            0x76 => {
                // HALT
                if !self.ime && self.pending_interrupts(mem) != 0 {
                    self.halt_bug = true;
                } else {
                    self.halted = true;
                }
                4
            }
            0xF3 => {
                self.ime = false;
                self.ei_pending = false;
                4
            }
            0xFB => {
                self.ei_pending = true;
                4
            }

            // 16-bit loads
            0x01 => { let v = self.fetch16(mem); self.regs.set_bc(v); 12 }
            0x11 => { let v = self.fetch16(mem); self.regs.set_de(v); 12 }
            0x21 => { let v = self.fetch16(mem); self.regs.set_hl(v); 12 }
            0x31 => { self.regs.sp = self.fetch16(mem); 12 }
            0x08 => {
                let addr = self.fetch16(mem);
                mem.write(addr, self.regs.sp as u8);
                mem.write(addr.wrapping_add(1), (self.regs.sp >> 8) as u8);
                20
            }
            0xF8 => { let v = self.add_sp_e8(mem); self.regs.set_hl(v); 12 }
            0xF9 => { self.regs.sp = self.regs.hl(); 8 }

            // 8-bit indirect loads
            0x02 => { mem.write(self.regs.bc(), self.regs.a); 8 }
            0x12 => { mem.write(self.regs.de(), self.regs.a); 8 }
            0x22 => { let hl = self.regs.hl(); mem.write(hl, self.regs.a); self.regs.set_hl(hl.wrapping_add(1)); 8 }
            0x32 => { let hl = self.regs.hl(); mem.write(hl, self.regs.a); self.regs.set_hl(hl.wrapping_sub(1)); 8 }
            0x0A => { self.regs.a = mem.read(self.regs.bc()); 8 }
            0x1A => { self.regs.a = mem.read(self.regs.de()); 8 }
            0x2A => { let hl = self.regs.hl(); self.regs.a = mem.read(hl); self.regs.set_hl(hl.wrapping_add(1)); 8 }
            0x3A => { let hl = self.regs.hl(); self.regs.a = mem.read(hl); self.regs.set_hl(hl.wrapping_sub(1)); 8 }
            0xE0 => { let off = self.fetch8(mem) as u16; mem.write(0xFF00 + off, self.regs.a); 12 }
            0xF0 => { let off = self.fetch8(mem) as u16; self.regs.a = mem.read(0xFF00 + off); 12 }
            0xE2 => { mem.write(0xFF00 + self.regs.c as u16, self.regs.a); 8 }
            0xF2 => { self.regs.a = mem.read(0xFF00 + self.regs.c as u16); 8 }
            0xEA => { let addr = self.fetch16(mem); mem.write(addr, self.regs.a); 16 }
            0xFA => { let addr = self.fetch16(mem); self.regs.a = mem.read(addr); 16 }

            // 16-bit inc/dec and ADD HL
            0x03 => { self.regs.set_bc(self.regs.bc().wrapping_add(1)); 8 }
            0x13 => { self.regs.set_de(self.regs.de().wrapping_add(1)); 8 }
            0x23 => { self.regs.set_hl(self.regs.hl().wrapping_add(1)); 8 }
            0x33 => { self.regs.sp = self.regs.sp.wrapping_add(1); 8 }
            0x0B => { self.regs.set_bc(self.regs.bc().wrapping_sub(1)); 8 }
            0x1B => { self.regs.set_de(self.regs.de().wrapping_sub(1)); 8 }
            0x2B => { self.regs.set_hl(self.regs.hl().wrapping_sub(1)); 8 }
            0x3B => { self.regs.sp = self.regs.sp.wrapping_sub(1); 8 }
            0x09 => { self.add_hl(self.regs.bc()); 8 }
            0x19 => { self.add_hl(self.regs.de()); 8 }
            0x29 => { self.add_hl(self.regs.hl()); 8 }
            0x39 => { self.add_hl(self.regs.sp); 8 }
            0xE8 => { self.regs.sp = self.add_sp_e8(mem); 16 }

            // INC/DEC r8: 0x04/0x0C pattern
            0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
                let idx = (opcode >> 3) & 7;
                let v = self.get_r8(idx, mem);
                let r = self.inc8(v);
                self.set_r8(idx, r, mem);
                if idx == 6 { 12 } else { 4 }
            }
            0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
                let idx = (opcode >> 3) & 7;
                let v = self.get_r8(idx, mem);
                let r = self.dec8(v);
                self.set_r8(idx, r, mem);
                if idx == 6 { 12 } else { 4 }
            }

            // LD r8, d8
            0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
                let idx = (opcode >> 3) & 7;
                let v = self.fetch8(mem);
                self.set_r8(idx, v, mem);
                if idx == 6 { 12 } else { 8 }
            }

            // rotate A
            0x07 => { self.regs.a = self.rlc(self.regs.a); self.regs.set_flag(FLAG_Z, false); 4 }
            0x0F => { self.regs.a = self.rrc(self.regs.a); self.regs.set_flag(FLAG_Z, false); 4 }
            0x17 => { self.regs.a = self.rl(self.regs.a); self.regs.set_flag(FLAG_Z, false); 4 }
            0x1F => { self.regs.a = self.rr(self.regs.a); self.regs.set_flag(FLAG_Z, false); 4 }

            0x27 => { self.daa(); 4 }
            0x2F => {
                self.regs.a = !self.regs.a;
                self.regs.set_flag(FLAG_N, true);
                self.regs.set_flag(FLAG_H, true);
                4
            }
            0x37 => {
                self.regs.set_flag(FLAG_N, false);
                self.regs.set_flag(FLAG_H, false);
                self.regs.set_flag(FLAG_C, true);
                4
            }
            0x3F => {
                let c = self.regs.flag(FLAG_C);
                self.regs.set_flag(FLAG_N, false);
                self.regs.set_flag(FLAG_H, false);
                self.regs.set_flag(FLAG_C, !c);
                4
            }

            // JR
            0x18 => {
                let e = self.fetch8(mem) as i8;
                self.regs.pc = self.regs.pc.wrapping_add_signed(e as i16);
                12
            }
            0x20 | 0x28 | 0x30 | 0x38 => {
                let e = self.fetch8(mem) as i8;
                if self.condition((opcode >> 3) & 3) {
                    self.regs.pc = self.regs.pc.wrapping_add_signed(e as i16);
                    12
                } else {
                    8
                }
            }

            // LD r8, r8 (0x40-0x7F except HALT)
            0x40..=0x7F => {
                let dst = (opcode >> 3) & 7;
                let src = opcode & 7;
                let v = self.get_r8(src, mem);
                self.set_r8(dst, v, mem);
                if dst == 6 || src == 6 { 8 } else { 4 }
            }

            // ALU A, r8 (0x80-0xBF)
            0x80..=0xBF => {
                let src = opcode & 7;
                let v = self.get_r8(src, mem);
                match (opcode >> 3) & 7 {
                    0 => self.add(v, false),
                    1 => self.add(v, true),
                    2 => self.sub(v, false, true),
                    3 => self.sub(v, true, true),
                    4 => self.and(v),
                    5 => self.xor(v),
                    6 => self.or(v),
                    _ => self.sub(v, false, false), // CP
                }
                if src == 6 { 8 } else { 4 }
            }

            // ALU A, d8
            0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
                let v = self.fetch8(mem);
                match (opcode >> 3) & 7 {
                    0 => self.add(v, false),
                    1 => self.add(v, true),
                    2 => self.sub(v, false, true),
                    3 => self.sub(v, true, true),
                    4 => self.and(v),
                    5 => self.xor(v),
                    6 => self.or(v),
                    _ => self.sub(v, false, false),
                }
                8
            }

            // RET / RETI / conditional RET
            0xC9 => { self.regs.pc = self.pop16(mem); 16 }
            0xD9 => { self.regs.pc = self.pop16(mem); self.ime = true; 16 }
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                if self.condition((opcode >> 3) & 3) {
                    self.regs.pc = self.pop16(mem);
                    20
                } else {
                    8
                }
            }

            // JP
            0xC3 => { self.regs.pc = self.fetch16(mem); 16 }
            0xE9 => { self.regs.pc = self.regs.hl(); 4 }
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let addr = self.fetch16(mem);
                if self.condition((opcode >> 3) & 3) {
                    self.regs.pc = addr;
                    16
                } else {
                    12
                }
            }

            // CALL
            0xCD => {
                let addr = self.fetch16(mem);
                self.push16(mem, self.regs.pc);
                self.regs.pc = addr;
                24
            }
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let addr = self.fetch16(mem);
                if self.condition((opcode >> 3) & 3) {
                    self.push16(mem, self.regs.pc);
                    self.regs.pc = addr;
                    24
                } else {
                    12
                }
            }

            // RST
            0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
                self.push16(mem, self.regs.pc);
                self.regs.pc = (opcode & 0x38) as u16;
                16
            }

            // PUSH/POP
            0xC5 => { self.push16(mem, self.regs.bc()); 16 }
            0xD5 => { self.push16(mem, self.regs.de()); 16 }
            0xE5 => { self.push16(mem, self.regs.hl()); 16 }
            0xF5 => { self.push16(mem, self.regs.af()); 16 }
            0xC1 => { let v = self.pop16(mem); self.regs.set_bc(v); 12 }
            0xD1 => { let v = self.pop16(mem); self.regs.set_de(v); 12 }
            0xE1 => { let v = self.pop16(mem); self.regs.set_hl(v); 12 }
            0xF1 => { let v = self.pop16(mem); self.regs.set_af(v); 12 }

            // CB prefix
            0xCB => self.execute_cb(mem),

            // Unused opcodes lock the CPU on hardware; treat as NOP.
            0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => 4,
        }
    }

    fn execute_cb(&mut self, mem: &mut impl Memory) -> u32 {
        let op = self.fetch8(mem);
        let idx = op & 7;
        let v = self.get_r8(idx, mem);
        let is_hl = idx == 6; // (HL) operand costs extra cycles
        match op >> 6 {
            0 => {
                let r = match (op >> 3) & 7 {
                    0 => self.rlc(v),
                    1 => self.rrc(v),
                    2 => self.rl(v),
                    3 => self.rr(v),
                    4 => self.sla(v),
                    5 => self.sra(v),
                    6 => self.swap(v),
                    _ => self.srl(v),
                };
                self.set_r8(idx, r, mem);
                if is_hl { 16 } else { 8 }
            }
            1 => {
                // BIT
                let bit = (op >> 3) & 7;
                self.regs.set_flag(FLAG_Z, v & (1 << bit) == 0);
                self.regs.set_flag(FLAG_N, false);
                self.regs.set_flag(FLAG_H, true);
                if is_hl { 12 } else { 8 }
            }
            2 => {
                // RES
                let bit = (op >> 3) & 7;
                self.set_r8(idx, v & !(1 << bit), mem);
                if is_hl { 16 } else { 8 }
            }
            _ => {
                // SET
                let bit = (op >> 3) & 7;
                self.set_r8(idx, v | (1 << bit), mem);
                if is_hl { 16 } else { 8 }
            }
        }
    }
}
