//! Debugger state: breakpoints, pause flag, and the execution trace.
//!
//! This module owns the *state* the run-control verbs act on; the verbs
//! themselves ([`GameBoy::pause`](crate::GameBoy::pause),
//! [`step_frame`](crate::GameBoy::step_frame), etc.) live on the machine that
//! drives them. Fields are private so callers go through the verbs, not the
//! data.

use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint(u16),
    Step,
    Paused,
}

pub struct Debugger {
    breakpoints: HashSet<u16>,
    paused: bool,
    /// Ring buffer of recently executed PCs.
    trace: Vec<u16>,
    trace_pos: usize,
    trace_capacity: usize,
}

impl Default for Debugger {
    fn default() -> Self {
        Debugger {
            breakpoints: HashSet::new(),
            paused: false,
            trace: Vec::new(),
            trace_pos: 0,
            trace_capacity: 256,
        }
    }
}

impl Debugger {
    pub fn new() -> Self {
        Debugger::default()
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn toggle_breakpoint(&mut self, addr: u16) {
        if !self.breakpoints.remove(&addr) {
            self.breakpoints.insert(addr);
        }
    }

    pub fn has_breakpoint(&self, addr: u16) -> bool {
        self.breakpoints.contains(&addr)
    }

    /// Breakpoint addresses, ascending.
    pub fn breakpoints(&self) -> Vec<u16> {
        let mut v: Vec<u16> = self.breakpoints.iter().copied().collect();
        v.sort_unstable();
        v
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

    #[cfg(test)]
    fn set_trace_capacity(&mut self, cap: usize) {
        self.trace_capacity = cap;
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
    fn breakpoints_listed_sorted() {
        let mut d = Debugger::new();
        d.toggle_breakpoint(0x0200);
        d.toggle_breakpoint(0x0100);
        d.toggle_breakpoint(0x0150);
        assert_eq!(d.breakpoints(), vec![0x0100, 0x0150, 0x0200]);
    }

    #[test]
    fn trace_ring_buffer() {
        let mut d = Debugger::new();
        d.set_trace_capacity(4);
        for pc in 0..6u16 {
            d.record_pc(pc);
        }
        assert_eq!(d.trace(), vec![2, 3, 4, 5]);
    }
}
