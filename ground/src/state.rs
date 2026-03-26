use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use crate::types::{Metrics, SubsystemID, Command, Priority};
use crate::config::{MAX_SUBSYSTEM, TICK_RATE};

#[derive(Debug)]
pub struct SyncState {
    pub is_calibrated: AtomicBool,
    pub last_sent_at: AtomicU64,
    pub samples: AtomicU64,
}

#[derive(Debug)]
pub struct LinkState {
    pub last_packet_sequence: AtomicU32,
    pub consecutive_missing: AtomicU32,
    pub last_packet_time: AtomicU64,
    pub windows_since_sync: AtomicU32,
}

#[derive(Debug)]
pub struct SubsystemInterlockState {
    pub id: SubsystemID,
    pub interlock: AtomicBool,
    pub fault_detected_at: AtomicU64,
    pub alert_sent: AtomicBool,
}

impl SubsystemInterlockState {
    pub fn clear(&self) {
        self.interlock.store(false, Ordering::Release);
        self.fault_detected_at.store(0, Ordering::Release);
        self.alert_sent.store(false, Ordering::Release);
    }
}

#[derive(Debug)]
pub struct ScheduledCommand {
    pub command: Command,
    pub priority: Priority,
    pub interval_ms: u64,
    pub next_send_time: AtomicU64, 
    pub enabled: AtomicBool,
}

#[derive(Debug)]
pub struct GroundState {
    pub is_running: AtomicBool,
    pub clock_sync: SyncState,
    pub boot_time: Instant,
    pub link: LinkState,
    pub subsystem_health: [SubsystemInterlockState; MAX_SUBSYSTEM],
    pub command_schedule: Mutex<Vec<ScheduledCommand>>,
    pub cpu_active_ms: AtomicU64,
    pub buffer_fill_rate: AtomicU32,
    pub command_dispatch_latency: Metrics,
    pub telemetry_reception_latency: Metrics,
}

impl GroundState {
    pub fn new() -> Self {
        Self {
            is_running: AtomicBool::new(true),
            clock_sync: SyncState {
                is_calibrated: AtomicBool::new(false),
                last_sent_at: AtomicU64::new(0),
                samples: AtomicU64::new(0),
            },
            boot_time: Instant::now(),
            link: LinkState {
                last_packet_sequence: AtomicU32::new(0),
                consecutive_missing: AtomicU32::new(0),
                last_packet_time: AtomicU64::new(0),
                windows_since_sync: AtomicU32::new(0),
            },
            subsystem_health: [
                SubsystemInterlockState {
                    id: SubsystemID::Antenna,
                    interlock: AtomicBool::new(false),
                    fault_detected_at: AtomicU64::new(0),
                    alert_sent: AtomicBool::new(false),
                },
                SubsystemInterlockState {
                    id: SubsystemID::Power,
                    interlock: AtomicBool::new(false),
                    fault_detected_at: AtomicU64::new(0),
                    alert_sent: AtomicBool::new(false),
                },
            ],
            command_schedule: Mutex::new(vec![
                ScheduledCommand {
                    command: Command::RotateAntenna { target_angle: 9000 },
                    priority: Priority::Normal,
                    interval_ms: 5 * TICK_RATE,
                    next_send_time: AtomicU64::new(0),
                    enabled: AtomicBool::new(true),
                },
                ScheduledCommand {
                    command: Command::SetPowerMode { mode: 1 },
                    priority: Priority::Normal,
                    interval_ms: 20 * TICK_RATE,
                    next_send_time: AtomicU64::new(0),
                    enabled: AtomicBool::new(true),
                },
            ]),
            cpu_active_ms: AtomicU64::new(0),
            buffer_fill_rate: AtomicU32::new(0),
            command_dispatch_latency: Metrics {
                last_latency_ms: AtomicU64::new(0),
                total_latency_ms: AtomicU64::new(0),
                max_latency_ms: AtomicU64::new(0),
                min_latency_ms: AtomicU64::new(0),
                last_jitter_ms: AtomicU64::new(0),
                total_jitter_ms: AtomicU64::new(0),
                max_jitter: AtomicU64::new(0),
                min_jitter: AtomicU64::new(0),
                number_of_samples: AtomicU32::new(0),
            },
            telemetry_reception_latency: Metrics {
                last_latency_ms: AtomicU64::new(0),
                total_latency_ms: AtomicU64::new(0),
                max_latency_ms: AtomicU64::new(0),
                min_latency_ms: AtomicU64::new(0),
                last_jitter_ms: AtomicU64::new(0),
                total_jitter_ms: AtomicU64::new(0),
                max_jitter: AtomicU64::new(0),
                min_jitter: AtomicU64::new(0),
                number_of_samples: AtomicU32::new(0),
            },
        }
    }

    pub fn uptime_ms(&self) -> u64 {
        self.boot_time.elapsed().as_micros() as u64
    }

    pub fn find_subsystem(&self, id: SubsystemID) -> Option<&SubsystemInterlockState> {
        self.subsystem_health.iter().find(|s| s.id == id)
    }
}