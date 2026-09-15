//! Game controller input via `gilrs`. Bluetooth (or USB) pads paired through
//! the OS appear here as standard gamepads. Read alongside the keyboard: the
//! frontend ORs the two per button each frame.

use gilrs::{Axis, Button, Gilrs};

/// Current pressed state of the eight Game Boy buttons from the gamepad.
#[derive(Default, Clone, Copy)]
pub struct PadState {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub a: bool,
    pub b: bool,
    pub start: bool,
    pub select: bool,
}

pub struct Gamepad {
    gilrs: Option<Gilrs>,
}

impl Gamepad {
    /// Initialise the gamepad subsystem; degrades to no input if unavailable.
    pub fn new() -> Self {
        Gamepad {
            gilrs: Gilrs::new().ok(),
        }
    }

    /// Drain pending events and read the first connected pad's current state.
    /// D-pad and left stick both drive the direction pad; face buttons match
    /// their labels (South = A, East = B).
    pub fn poll(&mut self) -> PadState {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return PadState::default();
        };
        // Pump the event queue so polled state is current and hotplug is seen.
        while gilrs.next_event().is_some() {}

        let Some((id, _)) = gilrs.gamepads().next() else {
            return PadState::default();
        };
        let gp = gilrs.gamepad(id);
        let th = 0.5;
        let x = gp.value(Axis::LeftStickX);
        let y = gp.value(Axis::LeftStickY);
        PadState {
            up: gp.is_pressed(Button::DPadUp) || y > th,
            down: gp.is_pressed(Button::DPadDown) || y < -th,
            left: gp.is_pressed(Button::DPadLeft) || x < -th,
            right: gp.is_pressed(Button::DPadRight) || x > th,
            a: gp.is_pressed(Button::South),
            b: gp.is_pressed(Button::East),
            start: gp.is_pressed(Button::Start),
            select: gp.is_pressed(Button::Select),
        }
    }
}
