# gbem

A Game Boy (DMG) emulator: a pure emulation core (`gb-core`) driven by a
desktop frontend (`gbem`). This glossary names the concepts the code and its
architecture reviews share.

## Language

**Memory**:
The seam the CPU is written against — the 16-bit address space as two
operations, `read(addr)` and `write(addr, val)`. The CPU observes everything
(RAM, VRAM, IO registers, the interrupt flags) through it. `Bus` is the
production adapter; a flat 64 KiB array is the test adapter.
_Avoid_: MMU (that is the `Bus`), address space.

**Bus**:
The production adapter satisfying the `Memory` seam: routes each address to a
peripheral and advances all peripherals in lock-step via `tick`.
_Avoid_: MMU, memory controller.

**Interrupts**:
The IE (0xFFFF) and IF (0xFF0F) register pair. Peripherals `raise` bits here;
the CPU reads them through the `Memory` seam and owns priority selection and
vector dispatch.
_Avoid_: IRQ, interrupt service (that is the CPU's dispatch).

**Cartridge**:
The loaded ROM plus its memory-bank controller (MBC) and any battery-backed
RAM. Reached through the `Bus`.
_Avoid_: game, rom (the raw bytes are the `rom`, the `Cartridge` wraps them).
