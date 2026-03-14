use std::sync::atomic::{AtomicBool, Ordering};
use crate::types::Command;

pub struct InterlockManager {
    pub antenna_locked: AtomicBool,
    pub power_locked: AtomicBool,
    pub global_loss_of_contact: AtomicBool,
}

impl InterlockManager {
    pub fn new() -> Self {
        Self {
            antenna_locked: AtomicBool::new(false),
            power_locked: AtomicBool::new(false),
            global_loss_of_contact: AtomicBool::new(false),
        }
    }

    /// Checks if a command is safe to send. 
    /// Fulfills Requirement 3: "Block unsafe commands until resolved."
    pub fn is_safe(&self, cmd: &Command) -> bool {
        // If we lost contact, block ALL routine commands. Only allow urgent recovery.
        if self.global_loss_of_contact.load(Ordering::SeqCst) && !cmd.is_urgent() {
            return false;
        }

        match cmd {
            Command::RotateAntenna { .. } => !self.antenna_locked.load(Ordering::SeqCst),
            Command::SetPowerMode { .. } => !self.power_locked.load(Ordering::SeqCst),
            _ => true, // Urgent commands (ClearFault, Reboot) are always allowed
        }
    }

    pub fn set_subsystem_lock(&self, subsystem_id: u8, state: bool) {
        match subsystem_id {
            0 => self.antenna_locked.store(state, Ordering::SeqCst),
            1 => self.power_locked.store(state, Ordering::SeqCst),
            _ => {}
        }
    }

    /// Fulfills Requirement 1: "Simulate loss of contact if >= 3 packets fail"
    pub fn trigger_loss_of_contact(&self) {
        self.global_loss_of_contact.store(true, Ordering::SeqCst);
        // Safest action: Lock all physical actuators
        self.antenna_locked.store(true, Ordering::SeqCst);
        self.power_locked.store(true, Ordering::SeqCst);
    }
}