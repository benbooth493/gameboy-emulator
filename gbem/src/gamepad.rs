//! Game controller input via `gilrs`. Bluetooth (or USB) pads paired through
//! the OS appear here as standard gamepads. Bindings are configurable (see
//! [`crate::config`]); this module resolves them against the live pad and also
//! captures a raw button press for the rebinding UI.

use gilrs::{Button as Pad, EventType, Gilrs};

use crate::config::Controls;

/// Current pressed state of the eight Game Boy buttons from the gamepad,
/// in the [`crate::config::BUTTONS`] order.
pub type PadState = [bool; 8];

pub struct Gamepad {
    gilrs: Option<Gilrs>,
    /// Last raw button press seen this frame (for rebinding).
    captured: Option<Pad>,
    /// Name of the most recently seen gamepad, for display.
    name: Option<String>,
}

impl Gamepad {
    /// Initialise the gamepad subsystem; degrades to no input if unavailable.
    pub fn new() -> Self {
        let gilrs = Gilrs::new().ok();
        let name = gilrs
            .as_ref()
            .and_then(|g| g.gamepads().next().map(|(_, gp)| gp.name().to_string()));
        Gamepad {
            gilrs,
            captured: None,
            name,
        }
    }

    /// Drain events (updating hotplug state and capturing any raw press), then
    /// read the first connected pad's state for the configured bindings.
    pub fn poll(&mut self, controls: &Controls) -> PadState {
        self.captured = None;
        let Some(gilrs) = self.gilrs.as_mut() else {
            return [false; 8];
        };
        while let Some(ev) = gilrs.next_event() {
            match ev.event {
                EventType::ButtonPressed(btn, _) => self.captured = Some(btn),
                EventType::Connected => {
                    self.name = Some(gilrs.gamepad(ev.id).name().to_string());
                }
                EventType::Disconnected => self.name = None,
                _ => {}
            }
        }

        let Some((id, _)) = gilrs.gamepads().next() else {
            return [false; 8];
        };
        let gp = gilrs.gamepad(id);
        let mut state = [false; 8];
        for (i, s) in state.iter_mut().enumerate() {
            *s = gp.is_pressed(controls.pads[i]);
        }
        state
    }

    /// A gamepad button pressed this frame, for the rebinding UI.
    pub fn take_captured(&mut self) -> Option<Pad> {
        self.captured.take()
    }

    /// The connected controller's name, if any.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}
