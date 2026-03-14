use std::time::Duration;
use std::types::Command;

use crate::types::{Command, SubsystemID};

pub const TICK_RATE: u64 = 1; // 1ms

// Network Configuration
pub const TARGET_SATELLITE_ADDR: &str = "127.0.0.1:8000"; // GCS connects to Sat [cite: 120]
pub const TCP_READ_TIMEOUT_MS: u64 = 500 * TICK_RATE;

// Real-Time Deadlines (Rubric Constraints)
pub const DECODE_DEADLINE_MS: u128 = 3 * TICK_RATE as u128; // [cite: 120]
pub const DISPATCH_DEADLINE_MS: u128 = 2 * TICK_RATE as u128; // 
pub const FAULT_RESPONSE_LIMIT_MS: u64 = 100 * TICK_RATE; // [cite: 144]
pub const LOSS_OF_CONTACT_THRESHOLD: u32 = 23; // [cite: 124]

// Task Frequencies
pub const TELEMETRY_POLL_MS: u64 = 2 * TICK_RATE; // Rate at which GCS drains the priority queue
pub const SCHEDULER_TICK_MS: u64 = 5 * TICK_RATE;

pub struct ScheduledCommand {
    pub time_ms: u64,
    pub command: Command,
}

// Mission Schedule: (Execution Time MS, Command)
pub const MISSION_SCHEDULE: [ScheduledCommand; 4] = [
        ScheduledCommand { time_ms: 1000, command: Command::RotateAntenna { target_angle: 45 } },
        ScheduledCommand { time_ms: 5000, command: Command::SetPowerMode { mode: 2 } },
        ScheduledCommand { time_ms: 10000, command: Command::RotateAntenna { target_angle: 90 } },
        ScheduledCommand { time_ms: 15000, command: Command::RotateAntenna { target_angle: 180 } },
        // Notice we don't schedule "ClearFault" here. 
        // The GCS will generate that automatically when it hears the fault is resolved.
];

