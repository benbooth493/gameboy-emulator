//! Debugger core: breakpoints, watchpoints, stepping and execution trace.

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint(u16),
    Watchpoint { addr: u16, value: u8 },
    Step,
    Paused,
}

#[derive(Default)]
pub struct Debugger {
    pub breakpoints: HashSet<u16>,
    pub watchpoints: HashSet<u16>,
    pub paused: bool,
    /// Ring buffer of recently executed PCs.
    trace: Vec<u16>,
    trace_pos: usize,
    pub trace_capacity: usize,
}

impl Debugger {
    pub fn new() -> Self {
        Debugger {
            trace_capacity: 256,
            ..Default::default()
        }
    }

    pub fn toggle_breakpoint(&mut self, addr: u16) {
        if !self.breakpoints.remove(&addr) {
            self.breakpoints.insert(addr);
        }
    }

    pub fn has_breakpoint(&self, addr: u16) -> bool {
        self.breakpoints.contains(&addr)
    }

    pub fn record_pc(&mut self, pc: u16) {
        if self.trace_capacity == 0 {
            return;
        }
        if self.trace.len() < self.trace_capacity {
            self.trace.push(pc);
        } else {
            self.trace[self.trace_pos] = pc;
        }
        self.trace_pos = (self.trace_pos + 1) % self.trace_capacity;
    }

    /// Most recent PCs, oldest first.
    pub fn trace(&self) -> Vec<u16> {
        if self.trace.len() < self.trace_capacity {
            self.trace.clone()
        } else {
            let mut v = Vec::with_capacity(self.trace.len());
            v.extend_from_slice(&self.trace[self.trace_pos..]);
            v.extend_from_slice(&self.trace[..self.trace_pos]);
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breakpoint_toggle() {
        let mut d = Debugger::new();
        d.toggle_breakpoint(0x150);
        assert!(d.has_breakpoint(0x150));
        d.toggle_breakpoint(0x150);
        assert!(!d.has_breakpoint(0x150));
    }

    #[test]
    fn trace_ring_buffer() {
        let mut d = Debugger::new();
        d.trace_capacity = 4;
        for pc in 0..6u16 {
            d.record_pc(pc);
        }
        assert_eq!(d.trace(), vec![2, 3, 4, 5]);
    }
}
