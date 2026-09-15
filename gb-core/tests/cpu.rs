//! SM83 instruction-level tests, run against the `Memory` seam with a flat
//! 64 KiB address space — no Bus, no Cartridge. Interrupt-dispatch tests poke
//! IE (0xFFFF) / IF (0xFF0F) as plain bytes.

use gb_core::cpu::registers::{FLAG_C, FLAG_H, FLAG_N, FLAG_Z};
use gb_core::cpu::Cpu;
use gb_core::interrupts::INT_VBLANK;
use gb_core::CpuBus;

const IE_ADDR: u16 = 0xFFFF;
const IF_ADDR: u16 = 0xFF0F;

/// A flat 64 KiB memory: the test adapter for the CPU's `CpuBus` seam. Its
/// clocking methods are no-ops — opcode tests care about results and the cycle
/// counts `step` returns, not real-time peripheral advancement.
struct FlatMem {
    ram: [u8; 0x10000],
}

impl CpuBus for FlatMem {
    fn read(&mut self, addr: u16) -> u8 {
        self.ram[addr as usize]
    }
    fn write(&mut self, addr: u16, val: u8) {
        self.ram[addr as usize] = val;
    }
    fn idle(&mut self) {}
    fn peek(&self, addr: u16) -> u8 {
        self.ram[addr as usize]
    }
    fn poke(&mut self, addr: u16, val: u8) {
        self.ram[addr as usize] = val;
    }
    fn elapsed(&self) -> u64 {
        0
    }
}

/// Build a CPU + flat memory with `code` placed at 0x0100 (the DMG entry point).
fn setup(code: &[u8]) -> (Cpu, FlatMem) {
    let mut mem = FlatMem { ram: [0; 0x10000] };
    mem.ram[0x100..0x100 + code.len()].copy_from_slice(code);
    (Cpu::new(), mem)
}

fn run(cpu: &mut Cpu, mem: &mut FlatMem, instructions: usize) -> u32 {
    (0..instructions).map(|_| cpu.step(mem)).sum()
}

#[test]
fn nop_advances_pc() {
    let (mut cpu, mut mem) = setup(&[0x00]);
    let c = cpu.step(&mut mem);
    assert_eq!(c, 4);
    assert_eq!(cpu.regs.pc, 0x101);
}

#[test]
fn ld_immediate_16() {
    let (mut cpu, mut mem) = setup(&[0x21, 0x34, 0x12]); // LD HL, 0x1234
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.hl(), 0x1234);
}

#[test]
fn ld_r8_r8_matrix() {
    let (mut cpu, mut mem) = setup(&[0x41]); // LD B, C
    cpu.regs.c = 0x7E;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.b, 0x7E);
}

#[test]
fn ld_hl_indirect() {
    // LD HL,0xC000; LD (HL),0x42; LD A,(HL)
    let (mut cpu, mut mem) = setup(&[0x21, 0x00, 0xC0, 0x36, 0x42, 0x7E]);
    run(&mut cpu, &mut mem, 3);
    assert_eq!(cpu.regs.a, 0x42);
    assert_eq!(mem.read(0xC000), 0x42);
}

#[test]
fn ldi_ldd() {
    // LD HL,0xC000; LD A,0xAA; LDI (HL),A; LDD (HL),A
    let (mut cpu, mut mem) = setup(&[0x21, 0x00, 0xC0, 0x3E, 0xAA, 0x22, 0x32]);
    run(&mut cpu, &mut mem, 4);
    assert_eq!(mem.read(0xC000), 0xAA);
    assert_eq!(mem.read(0xC001), 0xAA);
    assert_eq!(cpu.regs.hl(), 0xC000);
}

#[test]
fn add_sets_flags() {
    let (mut cpu, mut mem) = setup(&[0x80]); // ADD A, B
    cpu.regs.a = 0x0F;
    cpu.regs.b = 0x01;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x10);
    assert!(cpu.regs.flag(FLAG_H));
    assert!(!cpu.regs.flag(FLAG_Z));
    assert!(!cpu.regs.flag(FLAG_C));
}

#[test]
fn add_carry_and_zero() {
    let (mut cpu, mut mem) = setup(&[0x80]);
    cpu.regs.a = 0xFF;
    cpu.regs.b = 0x01;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x00);
    assert!(cpu.regs.flag(FLAG_Z));
    assert!(cpu.regs.flag(FLAG_C));
    assert!(cpu.regs.flag(FLAG_H));
}

#[test]
fn adc_uses_carry() {
    let (mut cpu, mut mem) = setup(&[0x88]); // ADC A, B
    cpu.regs.a = 0x00;
    cpu.regs.b = 0x00;
    cpu.regs.set_flag(FLAG_C, true);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x01);
}

#[test]
fn sub_and_compare() {
    let (mut cpu, mut mem) = setup(&[0x90]); // SUB B
    cpu.regs.a = 0x10;
    cpu.regs.b = 0x20;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0xF0);
    assert!(cpu.regs.flag(FLAG_C));
    assert!(cpu.regs.flag(FLAG_N));

    let (mut cpu, mut mem) = setup(&[0xFE, 0x42]); // CP 0x42
    cpu.regs.a = 0x42;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x42); // unchanged
    assert!(cpu.regs.flag(FLAG_Z));
}

#[test]
fn sbc_borrow_chain() {
    let (mut cpu, mut mem) = setup(&[0x98]); // SBC A, B
    cpu.regs.a = 0x00;
    cpu.regs.b = 0x00;
    cpu.regs.set_flag(FLAG_C, true);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0xFF);
    assert!(cpu.regs.flag(FLAG_C));
}

#[test]
fn logic_ops() {
    let (mut cpu, mut mem) = setup(&[0xE6, 0x0F]); // AND 0x0F
    cpu.regs.a = 0xF3;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x03);
    assert!(cpu.regs.flag(FLAG_H));

    let (mut cpu, mut mem) = setup(&[0xEE, 0xFF]); // XOR 0xFF
    cpu.regs.a = 0xAA;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x55);

    let (mut cpu, mut mem) = setup(&[0xF6, 0x00]); // OR 0
    cpu.regs.a = 0;
    cpu.step(&mut mem);
    assert!(cpu.regs.flag(FLAG_Z));
}

#[test]
fn inc_dec_half_carry() {
    let (mut cpu, mut mem) = setup(&[0x3C]); // INC A
    cpu.regs.a = 0x0F;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x10);
    assert!(cpu.regs.flag(FLAG_H));

    let (mut cpu, mut mem) = setup(&[0x3D]); // DEC A
    cpu.regs.a = 0x10;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x0F);
    assert!(cpu.regs.flag(FLAG_H));
    assert!(cpu.regs.flag(FLAG_N));
}

#[test]
fn dec_preserves_carry() {
    let (mut cpu, mut mem) = setup(&[0x05]); // DEC B
    cpu.regs.b = 1;
    cpu.regs.set_flag(FLAG_C, true);
    cpu.step(&mut mem);
    assert!(cpu.regs.flag(FLAG_Z));
    assert!(cpu.regs.flag(FLAG_C));
}

#[test]
fn add_hl_flags() {
    let (mut cpu, mut mem) = setup(&[0x09]); // ADD HL, BC
    cpu.regs.set_hl(0x0FFF);
    cpu.regs.set_bc(0x0001);
    cpu.regs.set_flag(FLAG_Z, true); // Z must be preserved
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.hl(), 0x1000);
    assert!(cpu.regs.flag(FLAG_H));
    assert!(cpu.regs.flag(FLAG_Z));
}

#[test]
fn add_sp_e8_flags() {
    let (mut cpu, mut mem) = setup(&[0xE8, 0x01]); // ADD SP, +1
    cpu.regs.sp = 0x00FF;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.sp, 0x0100);
    assert!(cpu.regs.flag(FLAG_C));
    assert!(cpu.regs.flag(FLAG_H));
    assert!(!cpu.regs.flag(FLAG_Z));

    let (mut cpu, mut mem) = setup(&[0xF8, 0xFF]); // LD HL, SP-1
    cpu.regs.sp = 0x0000;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.hl(), 0xFFFF);
}

#[test]
fn daa_after_add() {
    // LD A,0x45; ADD A,0x38; DAA -> 0x83
    let (mut cpu, mut mem) = setup(&[0x3E, 0x45, 0xC6, 0x38, 0x27]);
    run(&mut cpu, &mut mem, 3);
    assert_eq!(cpu.regs.a, 0x83);
    assert!(!cpu.regs.flag(FLAG_C));
}

#[test]
fn daa_after_sub() {
    // LD A,0x45; SUB 0x06; DAA -> 0x39
    let (mut cpu, mut mem) = setup(&[0x3E, 0x45, 0xD6, 0x06, 0x27]);
    run(&mut cpu, &mut mem, 3);
    assert_eq!(cpu.regs.a, 0x39);
}

#[test]
fn jumps_and_calls() {
    // JP 0x0110
    let (mut cpu, mut mem) = setup(&[0xC3, 0x10, 0x01]);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.pc, 0x0110);

    // CALL then RET
    let mut code = vec![0xCD, 0x10, 0x01]; // CALL 0x0110
    code.resize(0x10, 0x00);
    code.push(0xC9); // RET at 0x0110
    let (mut cpu, mut mem) = setup(&code);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.pc, 0x0110);
    assert_eq!(cpu.regs.sp, 0xFFFC);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.pc, 0x0103);
    assert_eq!(cpu.regs.sp, 0xFFFE);
}

#[test]
fn conditional_jr_taken_and_not() {
    let (mut cpu, mut mem) = setup(&[0x28, 0x05]); // JR Z, +5
    cpu.regs.set_flag(FLAG_Z, true);
    let c = cpu.step(&mut mem);
    assert_eq!(c, 12);
    assert_eq!(cpu.regs.pc, 0x0107);

    let (mut cpu, mut mem) = setup(&[0x28, 0x05]);
    cpu.regs.set_flag(FLAG_Z, false);
    let c = cpu.step(&mut mem);
    assert_eq!(c, 8);
    assert_eq!(cpu.regs.pc, 0x0102);
}

#[test]
fn jr_negative_offset() {
    let (mut cpu, mut mem) = setup(&[0x18, 0xFE]); // JR -2 (self)
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.pc, 0x0100);
}

#[test]
fn push_pop_roundtrip() {
    let (mut cpu, mut mem) = setup(&[0xC5, 0xF1]); // PUSH BC; POP AF
    cpu.regs.set_bc(0x1234);
    run(&mut cpu, &mut mem, 2);
    assert_eq!(cpu.regs.af(), 0x1230); // low nibble of F masked
}

#[test]
fn rst_vectors() {
    let (mut cpu, mut mem) = setup(&[0xEF]); // RST 0x28
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.pc, 0x0028);
}

#[test]
fn rotates_on_a() {
    let (mut cpu, mut mem) = setup(&[0x07]); // RLCA
    cpu.regs.a = 0x80;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x01);
    assert!(cpu.regs.flag(FLAG_C));
    assert!(!cpu.regs.flag(FLAG_Z)); // RLCA always clears Z

    let (mut cpu, mut mem) = setup(&[0x1F]); // RRA
    cpu.regs.a = 0x01;
    cpu.regs.set_flag(FLAG_C, true);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x80);
    assert!(cpu.regs.flag(FLAG_C));
}

#[test]
fn cb_bit_res_set_swap() {
    let (mut cpu, mut mem) = setup(&[0xCB, 0x7F]); // BIT 7, A
    cpu.regs.a = 0x80;
    cpu.step(&mut mem);
    assert!(!cpu.regs.flag(FLAG_Z));
    assert!(cpu.regs.flag(FLAG_H));

    let (mut cpu, mut mem) = setup(&[0xCB, 0x87]); // RES 0, A
    cpu.regs.a = 0xFF;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0xFE);

    let (mut cpu, mut mem) = setup(&[0xCB, 0xC7]); // SET 0, A
    cpu.regs.a = 0x00;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x01);

    let (mut cpu, mut mem) = setup(&[0xCB, 0x37]); // SWAP A
    cpu.regs.a = 0xAB;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0xBA);
}

#[test]
fn cb_srl_sra() {
    let (mut cpu, mut mem) = setup(&[0xCB, 0x3F]); // SRL A
    cpu.regs.a = 0x81;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x40);
    assert!(cpu.regs.flag(FLAG_C));

    let (mut cpu, mut mem) = setup(&[0xCB, 0x2F]); // SRA A
    cpu.regs.a = 0x81;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0xC0);
    assert!(cpu.regs.flag(FLAG_C));
}

#[test]
fn cpl_scf_ccf() {
    let (mut cpu, mut mem) = setup(&[0x2F, 0x37, 0x3F]);
    cpu.regs.a = 0xAA;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.a, 0x55);
    cpu.step(&mut mem); // SCF
    assert!(cpu.regs.flag(FLAG_C));
    cpu.step(&mut mem); // CCF
    assert!(!cpu.regs.flag(FLAG_C));
}

#[test]
fn interrupt_dispatch() {
    let (mut cpu, mut mem) = setup(&[0x00]);
    cpu.ime = true;
    mem.write(IE_ADDR, INT_VBLANK);
    mem.write(IF_ADDR, INT_VBLANK);
    let c = cpu.step(&mut mem);
    assert_eq!(c, 20);
    assert_eq!(cpu.regs.pc, 0x0040);
    assert!(!cpu.ime);
    assert_eq!(mem.read(IF_ADDR) & INT_VBLANK, 0); // IF bit acknowledged
    // Return address on stack.
    assert_eq!(mem.read(0xFFFC), 0x00);
    assert_eq!(mem.read(0xFFFD), 0x01);
}

#[test]
fn interrupt_priority_is_lowest_bit_first() {
    use gb_core::interrupts::INT_TIMER;
    // Both VBLANK (bit 0) and TIMER (bit 2) pending: VBLANK wins, then TIMER.
    let (mut cpu, mut mem) = setup(&[0x00]);
    cpu.ime = true;
    mem.write(IE_ADDR, 0x1F);
    mem.write(IF_ADDR, INT_VBLANK | INT_TIMER);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.pc, 0x0040); // VBLANK vector
    assert_eq!(mem.read(IF_ADDR), INT_TIMER); // only VBLANK cleared
    cpu.ime = true;
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.pc, 0x0050); // TIMER vector
}

#[test]
fn ei_is_delayed_one_instruction() {
    let (mut cpu, mut mem) = setup(&[0xFB, 0x00, 0x00]); // EI; NOP; NOP
    mem.write(IE_ADDR, INT_VBLANK);
    mem.write(IF_ADDR, INT_VBLANK);
    cpu.step(&mut mem); // EI
    assert!(!cpu.ime);
    cpu.step(&mut mem); // NOP — IME becomes true after this
    assert!(cpu.ime);
    let c = cpu.step(&mut mem); // now the interrupt fires
    assert_eq!(c, 20);
    assert_eq!(cpu.regs.pc, 0x0040);
}

#[test]
fn halt_wakes_on_interrupt_without_ime() {
    let (mut cpu, mut mem) = setup(&[0x76, 0x00]); // HALT; NOP
    cpu.step(&mut mem);
    assert!(cpu.halted);
    cpu.step(&mut mem);
    assert!(cpu.halted); // still halted, nothing pending
    mem.write(IE_ADDR, INT_VBLANK);
    mem.write(IF_ADDR, INT_VBLANK);
    cpu.step(&mut mem); // wakes, executes NOP
    assert!(!cpu.halted);
    assert_eq!(cpu.regs.pc, 0x0102);
}

#[test]
fn reti_enables_ime() {
    let mut code = vec![0xD9]; // RETI
    code.resize(0x20, 0);
    let (mut cpu, mut mem) = setup(&code);
    cpu.regs.sp = 0xFFFC;
    mem.write(0xFFFC, 0x50);
    mem.write(0xFFFD, 0x01);
    cpu.step(&mut mem);
    assert!(cpu.ime);
    assert_eq!(cpu.regs.pc, 0x0150);
}

#[test]
fn ldh_high_ram_access() {
    // LD A,0x99; LDH (0x80),A; LDH A,(0x80)
    let (mut cpu, mut mem) = setup(&[0x3E, 0x99, 0xE0, 0x80, 0xF0, 0x80]);
    run(&mut cpu, &mut mem, 3);
    assert_eq!(cpu.regs.a, 0x99);
    assert_eq!(mem.read(0xFF80), 0x99);
}
