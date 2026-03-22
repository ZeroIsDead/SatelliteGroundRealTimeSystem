use std::sync::atomic::{AtomicU32, AtomicU16, AtomicBool, AtomicU64, Ordering};
use std::time::{Instant};
use crate::types::{TaskID, Priority, SubsystemID, Metrics};
use crate::config::{MAX_SENSORS, TICK_RATE, MAX_SUBSYSTEM};

#[derive(Debug)]
pub struct SatelliteState {
    // Control Flags
    pub is_running: AtomicBool,
    pub is_visible: AtomicBool, 
    pub degraded_mode: AtomicBool,

    // Clock
    pub clock_sync: SyncState,
    pub boot_time: Instant,

    // Sensor
    pub sensors: [SensorState; MAX_SENSORS],
    pub subsystem_health: [SubsystemState; MAX_SUBSYSTEM],
    
    // Performance Metrics
    pub cpu_active_ms: AtomicU64,
    pub buffer_fill_rate: AtomicU32, // (Current size * 100) / Capacity
    pub packet_sequence_no: AtomicU32,
}

#[derive(Debug)]
pub struct SubsystemState {
    pub id: SubsystemID,
    pub value: AtomicU32,
    pub fault: AtomicBool,
    pub fault_interlock: AtomicBool, // For Subsystem, Need Ground Permission to Use again if Fault Happen
    pub fault_timestamp: AtomicU64,
    pub fault_reported: AtomicBool,
}

#[derive(Debug)]
pub struct SensorState {
    // Immutable
    pub priority: Priority,
    pub data_priority: Priority,
    pub task_id: TaskID,
    pub period: u64,
    pub min_data: u32,
    pub max_data: u32,

    // Mutable Through Atomic Methods
    pub value: AtomicU32,
    pub heartbeat: AtomicU64, // Last Seen
    pub fault: AtomicU16,
    pub fault_timestamp: AtomicU64,
    pub metrics: Metrics,
}

impl SensorState {
    pub fn has_valid_value(&self) -> bool {
        let value = self.value.load(Ordering::Relaxed);

        value >= self.min_data && value <= self.max_data
    }
}

#[derive(Debug)]
pub struct SyncState {
    pub total_offset_ms: AtomicU64,
    pub average_offset_ms: AtomicU64, 
    pub number_of_sample: AtomicU32,
    pub is_calibrated: AtomicBool, // Is it Syncing or not
}

impl SatelliteState {
    pub fn new() -> Self {
        Self {
            is_running: AtomicBool::new(true),
            is_visible: AtomicBool::new(false),
            degraded_mode: AtomicBool::new(false),
            clock_sync: SyncState {
                total_offset_ms: AtomicU64::new(0),
                average_offset_ms: AtomicU64::new(0),
                number_of_sample: AtomicU32::new(0),
                is_calibrated: AtomicBool::new(true),
            },
            boot_time: Instant::now(),
            
            sensors: [
                SensorState {
                    priority: Priority::Critical,
                    data_priority: Priority::Critical,
                    task_id: TaskID::ThermalSensor,
                    period: 5 * TICK_RATE,
                    min_data: 2500,
                    max_data: 5000,  
                    value: AtomicU32::new(2500),
                    heartbeat: AtomicU64::new(u64::MAX),
                    fault: AtomicU16::new(0),
                    fault_timestamp: AtomicU64::new(0),
                    metrics: Metrics {
                        last_latency_ms: AtomicU64::new(0),
                        total_latency_ms: AtomicU64::new(0),
                        total_jitter_ms: AtomicU64::new(0),
                        number_of_samples: AtomicU32::new(0),
                    }
                },
                SensorState {
                    priority: Priority::Normal,
                    data_priority: Priority::Normal,
                    task_id: TaskID::PitchAndYawSensor,
                    period: 10 * TICK_RATE,
                    min_data: 00000,
                    max_data: 36000, 
                    value: AtomicU32::new(18000),
                    heartbeat: AtomicU64::new(u64::MAX),
                    fault: AtomicU16::new(0),
                    fault_timestamp: AtomicU64::new(0),
                    metrics: Metrics {
                        last_latency_ms: AtomicU64::new(0),
                        total_latency_ms: AtomicU64::new(0),
                        total_jitter_ms: AtomicU64::new(0),
                        number_of_samples: AtomicU32::new(0),
                    }
                },
                SensorState {
                    priority: Priority::Low,
                    data_priority: Priority::Normal,
                    task_id: TaskID::MoistureSensor,
                    period: 20 * TICK_RATE,
                    min_data: 2500,
                    max_data: 5000, 
                    value: AtomicU32::new(4500),
                    heartbeat: AtomicU64::new(u64::MAX),
                    fault: AtomicU16::new(0),
                    fault_timestamp: AtomicU64::new(0),
                    metrics: Metrics {
                        last_latency_ms: AtomicU64::new(0),
                        total_latency_ms: AtomicU64::new(0),
                        total_jitter_ms: AtomicU64::new(0),
                        number_of_samples: AtomicU32::new(0),
                    }
                },
            ],
            subsystem_health: [
                SubsystemState {
                    id: SubsystemID::Antenna,
                    value: AtomicU32::new(0),
                    fault: AtomicBool::new(false),
                    fault_interlock: AtomicBool::new(false),
                    fault_timestamp: AtomicU64::new(0),
                    fault_reported: AtomicBool::new(false),
                },

                SubsystemState {
                    id: SubsystemID::Power,
                    value: AtomicU32::new(0),
                    fault: AtomicBool::new(false),
                    fault_interlock: AtomicBool::new(false),
                    fault_timestamp: AtomicU64::new(0),
                    fault_reported: AtomicBool::new(false),
                }
            ],
            
            cpu_active_ms: AtomicU64::new(0),
            buffer_fill_rate: AtomicU32::new(0),
            packet_sequence_no: AtomicU32::new(1),
        }
    }

    pub fn uptime_ms(&self) -> u64 {
        self.boot_time.elapsed().as_micros() as u64 
    }

    pub fn get_synchronized_timestamp(&self) -> u64 {
        self.uptime_ms() + &self.clock_sync.average_offset_ms.load(Ordering::Relaxed) 
    }
}