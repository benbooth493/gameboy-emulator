# gbem — Game Boy emulator

A Game Boy (DMG) emulator written in Rust, built test-first, with a
GPU-accelerated display (wgpu) whose WGSL shader emulates the original
DMG LCD, and a built-in debugger.

## Features

- **Complete SM83 CPU** — full instruction set (including CB prefix),
  interrupts, HALT (with the HALT bug), delayed EI. Verified against
  Blargg's `cpu_instrs` and `instr_timing` hardware test ROMs.
- **PPU** — scanline renderer: background, window, 8x8/8x16 sprites with
  DMG priority rules, mode timing, STAT/LYC interrupts.
- **APU** — both pulse channels (with sweep and envelope), wave channel,
  noise channel, frame sequencer; audio output via cpal.
- **Cartridges** — ROM-only, MBC1, MBC3, MBC5 with banked RAM.
- **Timer, joypad, OAM DMA, serial capture** (used by test ROMs).
- **GPU-shaded screen** — a custom wgpu render pipeline draws the frame
  through `gbem/src/shaders/gb_screen.wgsl`: classic DMG green palette,
  visible pixel grid, LCD response-time ghosting (previous frame blended
  in), vignette. Ghosting/grid strength are adjustable live.
- **Built-in debugger** — pause/continue, single-step, step-frame, live
  disassembly following PC, click-to-toggle breakpoints, register/flag/IO
  view, memory hex viewer, execution trace.

## Layout

- `gb-core/` — pure emulation core, no I/O dependencies; all the logic and
  all the tests live here.
- `gbem/` — desktop frontend: eframe/egui UI, wgpu screen pipeline, cpal
  audio.

## Run

```sh
cargo run --release -- path/to/rom.gb
```

or start it empty and use **File → Open ROM…** / drag & drop.

### Controls

| Key | Button |
|-----|--------|
| Arrow keys | D-pad |
| Z / X | A / B |
| Enter / Backspace | Start / Select |
| P | Pause/continue |
| N | Single-step (while paused) |

## Tests

```sh
cargo test --workspace            # unit + integration tests
cargo test --release --test blargg -p gb-core   # hardware test ROMs
```

The Blargg tests need `test-roms/cpu_instrs.gb` and
`test-roms/instr_timing.gb` (from the `retrio/gb-test-roms` mirror); they
skip gracefully if absent.
