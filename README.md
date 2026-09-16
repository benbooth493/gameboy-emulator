# gbem — Game Boy emulator

A Game Boy (DMG) emulator written in Rust, built test-first, with a
GPU-accelerated display (wgpu) whose WGSL shader emulates the original
DMG LCD, and a built-in debugger.

## Features

- **Complete SM83 CPU** — full instruction set (including CB prefix),
  interrupts, HALT (with the HALT bug), delayed EI. **M-cycle accurate**: the
  CPU drives the bus one M-cycle per access, so peripherals advance mid-
  instruction. Verified against Blargg's `cpu_instrs`, `instr_timing`, and
  `mem_timing`.
- **PPU** — scanline renderer: background, window, 8x8/8x16 sprites with
  DMG priority rules, mode timing, STAT/LYC interrupts. Outputs RGBA.
  Verified pixel-perfect against **dmg-acid2** and **cgb-acid2**.
- **Game Boy Color** — CGB ROMs run in colour: BG/OBJ colour palettes, VRAM
  banking with tile attributes, WRAM banking, double-speed mode (KEY1), and
  VRAM DMA (immediate GDMA + HBlank-paced HDMA). DMG ROMs keep the classic
  green rendering.
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
| F5 / F9 | Save / load state |

A **game controller** (Bluetooth or USB, paired through the OS) works too, via
`gilrs`. Keyboard and controller are read together.

All of the above are the **defaults** — every button's keyboard key and
controller binding is remappable in **Settings → Controls** (click a binding,
then press the input). Bindings persist to a JSON file in the OS config
directory.

## Tests

```sh
cargo test --workspace                           # unit + integration tests
cargo test --release --test blargg -p gb-core    # Blargg hardware test ROMs
cargo test --release --test dmg_acid2 -p gb-core # PPU conformance
```

Conformance ROMs live in `test-roms/` (git-ignored) and each test skips
gracefully if its ROM is absent:

- **Blargg** `cpu_instrs`, `instr_timing`, `mem_timing` (from the
  `retrio/gb-test-roms` mirror) — all pass. The CPU is M-cycle accurate: it
  drives the bus one M-cycle per memory access, so reads/writes land on the
  correct cycle within an instruction.
- **dmg-acid2** (matt currie) — renders a reference image exercising the
  background, window, and sprites (flips, priority, 8x16, palettes). The test
  compares the framebuffer pixel-for-pixel against the checked-in reference
  (`gb-core/tests/data/dmg-acid2-ref.bin`, decoded from the project's PNG) and
  **passes exactly**.
