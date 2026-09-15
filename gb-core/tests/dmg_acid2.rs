//! dmg-acid2 PPU conformance test (matt currie): renders a reference image
//! exercising background, window, and sprite features (flips, priority, 8x16,
//! palettes). We render the ROM and compare the framebuffer, pixel for pixel,
//! against the canonical reference decoded from the project's PNG.

use gb_core::{Cartridge, GameBoy, SCREEN_H, SCREEN_W};

#[test]
fn dmg_acid2_matches_reference() {
    let rom_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../test-roms/dmg-acid2.gb");
    let Ok(rom) = std::fs::read(rom_path) else {
        eprintln!("test-roms/dmg-acid2.gb not found; skipping");
        return;
    };
    // Reference shade indices (0 = lightest .. 3 = darkest), one byte per pixel.
    let reference: &[u8] = include_bytes!("data/dmg-acid2-ref.bin");
    assert_eq!(reference.len(), SCREEN_W * SCREEN_H);

    let mut gb = GameBoy::new(Cartridge::from_rom(rom).unwrap());
    // The ROM sets everything up and signals completion with `LD B,B`; a
    // handful of frames is plenty for the final image to settle.
    for _ in 0..30 {
        gb.run_frame();
    }

    // The framebuffer is RGBA; the PPU renders DMG shades through this palette.
    const DMG_PALETTE: [[u8; 3]; 4] = [
        [155, 188, 15],
        [139, 172, 15],
        [48, 98, 48],
        [15, 56, 15],
    ];
    let fb = gb.framebuffer();
    let diff = (0..SCREEN_W * SCREEN_H)
        .filter(|&i| {
            let rgb = [fb[i * 4], fb[i * 4 + 1], fb[i * 4 + 2]];
            let shade = DMG_PALETTE.iter().position(|&c| c == rgb).unwrap_or(255) as u8;
            shade != reference[i]
        })
        .count();
    assert_eq!(
        diff, 0,
        "dmg-acid2: {diff} of {} pixels differ from the reference",
        SCREEN_W * SCREEN_H
    );
}
