//! cgb-acid2 PPU conformance test (matt currie): the CGB analogue of
//! dmg-acid2, exercising colour palettes, VRAM-bank tile attributes, BG/OBJ
//! priority, and the master-priority bit. We render the ROM and compare the
//! RGBA framebuffer pixel-for-pixel against the canonical reference (decoded
//! from the project's PNG; the PPU uses the same straight 5-bit -> 8-bit colour
//! expansion the reference does).

use gb_core::{Cartridge, GameBoy, SCREEN_H, SCREEN_W};

#[test]
fn cgb_acid2_matches_reference() {
    let rom_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test-roms/cgb-acid2.gbc");
    let Ok(rom) = std::fs::read(rom_path) else {
        eprintln!("test-roms/cgb-acid2.gbc not found; skipping");
        return;
    };
    let reference: &[u8] = include_bytes!("data/cgb-acid2-ref.bin");
    assert_eq!(reference.len(), SCREEN_W * SCREEN_H * 4);

    let mut gb = GameBoy::new(Cartridge::from_rom(rom).unwrap());
    for _ in 0..30 {
        gb.run_frame();
    }

    let fb = gb.framebuffer();
    // Compare RGB (ignore the constant alpha byte).
    let diff = (0..SCREEN_W * SCREEN_H)
        .filter(|&i| fb[i * 4..i * 4 + 3] != reference[i * 4..i * 4 + 3])
        .count();
    assert_eq!(
        diff, 0,
        "cgb-acid2: {diff} of {} pixels differ from the reference",
        SCREEN_W * SCREEN_H
    );
}
