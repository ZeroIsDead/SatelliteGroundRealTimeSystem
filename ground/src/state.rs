use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use crate::config::TELEMETRY_RATE_MS;
use crate::types::Command;

pub struct GcsState {
    pub is_running: AtomicBool,
    pub interlocks: InterlockManager,
    pub metrics: PerformanceMetrics,
}

impl GcsState {
    pub fn new() -> Self {
        Self {
            is_running: AtomicBool::new(true),
            interlocks: InterlockManager::new(),
            metrics: PerformanceMetrics::new(),
        }
    }
}

pub struct InterlockManager {
    antenna_locked: AtomicBool,
    loss_of_contact: AtomicBool,
}

impl InterlockManager {
    fn new() -> Self {
        Self {
            antenna_locked: AtomicBool::new(false),
            loss_of_contact: AtomicBool::new(false),
        }
    }

    pub fn is_command_safe(&self, cmd: &Command) -> bool {
        // [cite: 129, 140]
        if self.loss_of_contact.load(Ordering::SeqCst) { return false; }
        match cmd {
            Command::RotateAntenna { .. } => !self.antenna_locked.load(Ordering::SeqCst),
            _ => true,
        }
    }

    pub fn set_subsystem_lock(&self, _id: u8, state: bool) {
        self.antenna_locked.store(state, Ordering::SeqCst);
    }

    pub fn set_loss_of_contact(&self, state: bool) {
        self.loss_of_contact.store(state, Ordering::SeqCst);
    }
}

pub struct PerformanceMetrics {
    packets_received: AtomicU32,
    missed_deadlines: AtomicU32,
    highest_seq: AtomicU32,
    first_packet_time: AtomicU64,
}

impl PerformanceMetrics {
    fn new() -> Self {
        Self {
            packets_received: AtomicU32::new(0),
            missed_deadlines: AtomicU32::new(0),
            highest_seq: AtomicU32::new(0),
            first_packet_time: AtomicU64::new(0),
        }
    }

    pub fn calculate_jitter(&self, seq: u32, current_time: u64) -> i64 {
        // [cite: 153]
        let start_anchor = self.first_packet_time.load(Ordering::Relaxed);
        if start_anchor == 0 {
            self.first_packet_time.store(current_time, Ordering::Relaxed);
            return 0;
        }
        let expected = start_anchor + (seq as u64 * TELEMETRY_RATE_MS);
        (current_time as i64) - (expected as i64)
    }

    pub fn increment_received(&self) { self.packets_received.fetch_add(1, Ordering::Relaxed); }
    pub fn record_deadline_miss(&self) { self.missed_deadlines.fetch_add(1, Ordering::Relaxed); }
    pub fn update_highest_seq(&self, seq: u32) { self.highest_seq.fetch_max(seq, Ordering::Relaxed); }
}