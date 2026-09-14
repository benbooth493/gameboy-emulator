//! Runs Blargg's cpu_instrs test ROM (if present in test-roms/) and checks
//! the pass/fail report it prints over the serial port.

use gb_core::{Cartridge, GameBoy};

#[test]
fn blargg_cpu_instrs() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test-roms/cpu_instrs.gb");
    let Ok(rom) = std::fs::read(path) else {
        eprintln!("test-roms/cpu_instrs.gb not found; skipping");
        return;
    };
    let mut gb = GameBoy::new(Cartridge::from_rom(rom).unwrap());

    // The full suite needs a few emulated minutes.
    for _ in 0..60 * 120 {
        gb.run_frame();
        let out = String::from_utf8_lossy(&gb.bus.serial_out);
        if out.contains("Passed") || out.contains("Failed") {
            break;
        }
    }
    let out = String::from_utf8_lossy(&gb.bus.serial_out);
    println!("serial output:\n{out}");
    assert!(
        out.contains("Passed"),
        "cpu_instrs did not pass:\n{out}"
    );
}

#[test]
fn blargg_instr_timing() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test-roms/instr_timing.gb");
    let Ok(rom) = std::fs::read(path) else {
        eprintln!("test-roms/instr_timing.gb not found; skipping");
        return;
    };
    let mut gb = GameBoy::new(Cartridge::from_rom(rom).unwrap());
    for _ in 0..60 * 30 {
        gb.run_frame();
        let out = String::from_utf8_lossy(&gb.bus.serial_out);
        if out.contains("Passed") || out.contains("Failed") {
            break;
        }
    }
    let out = String::from_utf8_lossy(&gb.bus.serial_out);
    println!("serial output:\n{out}");
    assert!(out.contains("Passed"), "instr_timing did not pass:\n{out}");
}
