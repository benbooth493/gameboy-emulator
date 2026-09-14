//! SM83 disassembler used by the debugger.

const R8: [&str; 8] = ["B", "C", "D", "E", "H", "L", "(HL)", "A"];
const R16: [&str; 4] = ["BC", "DE", "HL", "SP"];
const R16_STK: [&str; 4] = ["BC", "DE", "HL", "AF"];
const COND: [&str; 4] = ["NZ", "Z", "NC", "C"];
const ALU: [&str; 8] = ["ADD A,", "ADC A,", "SUB", "SBC A,", "AND", "XOR", "OR", "CP"];
const ROT: [&str; 8] = ["RLC", "RRC", "RL", "RR", "SLA", "SRA", "SWAP", "SRL"];

/// Disassemble one instruction from `read(addr)`.
/// Returns (mnemonic, byte length).
pub fn disassemble(read: impl Fn(u16) -> u8, addr: u16) -> (String, u16) {
    let op = read(addr);
    let d8 = || read(addr.wrapping_add(1));
    let d16 = || {
        u16::from_le_bytes([read(addr.wrapping_add(1)), read(addr.wrapping_add(2))])
    };
    let r8 = |i: u8| R8[(i & 7) as usize];

    let (text, len): (String, u16) = match op {
        0x00 => ("NOP".into(), 1),
        0x10 => ("STOP".into(), 2),
        0x76 => ("HALT".into(), 1),
        0xF3 => ("DI".into(), 1),
        0xFB => ("EI".into(), 1),
        0x07 => ("RLCA".into(), 1),
        0x0F => ("RRCA".into(), 1),
        0x17 => ("RLA".into(), 1),
        0x1F => ("RRA".into(), 1),
        0x27 => ("DAA".into(), 1),
        0x2F => ("CPL".into(), 1),
        0x37 => ("SCF".into(), 1),
        0x3F => ("CCF".into(), 1),
        0x08 => (format!("LD (${:04X}),SP", d16()), 3),
        0xE8 => (format!("ADD SP,{}", d8() as i8), 2),
        0xF8 => (format!("LD HL,SP{:+}", d8() as i8), 2),
        0xF9 => ("LD SP,HL".into(), 1),
        0x01 | 0x11 | 0x21 | 0x31 => {
            (format!("LD {},${:04X}", R16[(op >> 4) as usize], d16()), 3)
        }
        0x02 => ("LD (BC),A".into(), 1),
        0x12 => ("LD (DE),A".into(), 1),
        0x22 => ("LD (HL+),A".into(), 1),
        0x32 => ("LD (HL-),A".into(), 1),
        0x0A => ("LD A,(BC)".into(), 1),
        0x1A => ("LD A,(DE)".into(), 1),
        0x2A => ("LD A,(HL+)".into(), 1),
        0x3A => ("LD A,(HL-)".into(), 1),
        0x03 | 0x13 | 0x23 | 0x33 => (format!("INC {}", R16[(op >> 4) as usize]), 1),
        0x0B | 0x1B | 0x2B | 0x3B => (format!("DEC {}", R16[(op >> 4) as usize]), 1),
        0x09 | 0x19 | 0x29 | 0x39 => (format!("ADD HL,{}", R16[(op >> 4) as usize]), 1),
        0x04 | 0x0C | 0x14 | 0x1C | 0x24 | 0x2C | 0x34 | 0x3C => {
            (format!("INC {}", r8(op >> 3)), 1)
        }
        0x05 | 0x0D | 0x15 | 0x1D | 0x25 | 0x2D | 0x35 | 0x3D => {
            (format!("DEC {}", r8(op >> 3)), 1)
        }
        0x06 | 0x0E | 0x16 | 0x1E | 0x26 | 0x2E | 0x36 | 0x3E => {
            (format!("LD {},${:02X}", r8(op >> 3), d8()), 2)
        }
        0x18 => {
            let target = addr.wrapping_add(2).wrapping_add_signed(d8() as i8 as i16);
            (format!("JR ${target:04X}"), 2)
        }
        0x20 | 0x28 | 0x30 | 0x38 => {
            let target = addr.wrapping_add(2).wrapping_add_signed(d8() as i8 as i16);
            (format!("JR {},${:04X}", COND[((op >> 3) & 3) as usize], target), 2)
        }
        0x40..=0x7F => (format!("LD {},{}", r8(op >> 3), r8(op)), 1),
        0x80..=0xBF => (format!("{} {}", ALU[((op >> 3) & 7) as usize], r8(op)), 1),
        0xC6 | 0xCE | 0xD6 | 0xDE | 0xE6 | 0xEE | 0xF6 | 0xFE => {
            (format!("{} ${:02X}", ALU[((op >> 3) & 7) as usize], d8()), 2)
        }
        0xC9 => ("RET".into(), 1),
        0xD9 => ("RETI".into(), 1),
        0xC0 | 0xC8 | 0xD0 | 0xD8 => {
            (format!("RET {}", COND[((op >> 3) & 3) as usize]), 1)
        }
        0xC3 => (format!("JP ${:04X}", d16()), 3),
        0xE9 => ("JP HL".into(), 1),
        0xC2 | 0xCA | 0xD2 | 0xDA => {
            (format!("JP {},${:04X}", COND[((op >> 3) & 3) as usize], d16()), 3)
        }
        0xCD => (format!("CALL ${:04X}", d16()), 3),
        0xC4 | 0xCC | 0xD4 | 0xDC => {
            (format!("CALL {},${:04X}", COND[((op >> 3) & 3) as usize], d16()), 3)
        }
        0xC7 | 0xCF | 0xD7 | 0xDF | 0xE7 | 0xEF | 0xF7 | 0xFF => {
            (format!("RST ${:02X}", op & 0x38), 1)
        }
        0xC5 | 0xD5 | 0xE5 | 0xF5 => {
            (format!("PUSH {}", R16_STK[((op >> 4) & 3) as usize]), 1)
        }
        0xC1 | 0xD1 | 0xE1 | 0xF1 => {
            (format!("POP {}", R16_STK[((op >> 4) & 3) as usize]), 1)
        }
        0xE0 => (format!("LDH (${:02X}),A", d8()), 2),
        0xF0 => (format!("LDH A,(${:02X})", d8()), 2),
        0xE2 => ("LD (C),A".into(), 1),
        0xF2 => ("LD A,(C)".into(), 1),
        0xEA => (format!("LD (${:04X}),A", d16()), 3),
        0xFA => (format!("LD A,(${:04X})", d16()), 3),
        0xCB => {
            let cb = d8();
            let reg = R8[(cb & 7) as usize];
            let text = match cb >> 6 {
                0 => format!("{} {}", ROT[((cb >> 3) & 7) as usize], reg),
                1 => format!("BIT {},{}", (cb >> 3) & 7, reg),
                2 => format!("RES {},{}", (cb >> 3) & 7, reg),
                _ => format!("SET {},{}", (cb >> 3) & 7, reg),
            };
            (text, 2)
        }
        _ => (format!("DB ${op:02X}"), 1),
    };
    (text, len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dis(bytes: &[u8]) -> (String, u16) {
        let v = bytes.to_vec();
        disassemble(move |a| v.get(a as usize).copied().unwrap_or(0), 0)
    }

    #[test]
    fn basic_ops() {
        assert_eq!(dis(&[0x00]).0, "NOP");
        assert_eq!(dis(&[0x3E, 0x42]).0, "LD A,$42");
        assert_eq!(dis(&[0x21, 0x34, 0x12]).0, "LD HL,$1234");
        assert_eq!(dis(&[0xC3, 0x50, 0x01]).0, "JP $0150");
        assert_eq!(dis(&[0x78]).0, "LD A,B");
        assert_eq!(dis(&[0x86]).0, "ADD A, (HL)");
    }

    #[test]
    fn jr_computes_target() {
        // JR -2 at addr 0 -> target 0
        assert_eq!(dis(&[0x18, 0xFE]).0, "JR $0000");
    }

    #[test]
    fn cb_ops() {
        assert_eq!(dis(&[0xCB, 0x7C]).0, "BIT 7,H");
        assert_eq!(dis(&[0xCB, 0x37]).0, "SWAP A");
        assert_eq!(dis(&[0xCB, 0x37]).1, 2);
    }

    #[test]
    fn lengths() {
        assert_eq!(dis(&[0x00]).1, 1);
        assert_eq!(dis(&[0x3E, 0x00]).1, 2);
        assert_eq!(dis(&[0xC3, 0x00, 0x00]).1, 3);
    }
}
