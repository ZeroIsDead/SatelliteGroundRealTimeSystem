pub const TICK_RATE: u64 = 1; // 1ms
pub const DATA_BUFFER_CAPACITY: usize = 100;
pub const LOG_BUFFER_CAPACITY: usize = 100;
pub const PACKET_HISTORY_BUFFER_CAPACITY: usize = 100;
pub const NORMAL_TO_DEGRADED_THRESHOLD: u32 = 80;
pub const DEGRADED_TO_NORMAL_THRESHOLD: u32 = 80;
pub const NETWORK_PORT: &str = "0.0.0.0:8000";

pub const MAX_SENSORS: usize = 3;
pub const MAX_SUBSYSTEM: usize = 2;

// Frequencies (in Milliseconds)
pub const SUBSYSTEM_FAULT_INJECTION_MS: u64 = 60 * TICK_RATE;
pub const SENSOR_FAULT_INJECTION_MS: u64 = 60 * TICK_RATE;
pub const SENSOR_FAULT_MS: u64 = 10 * TICK_RATE;
pub const SENSOR_DELAY_MS: u64 = 2 * TICK_RATE;
pub const SENSOR_DATA_CORRUPTION: u32 = 99999;
pub const MONITOR_MS: u64 = 5 * TICK_RATE;
pub const COMMAND_MS: u64 = 5 * TICK_RATE;
pub const NETWORK_MS: u64 = 2 * TICK_RATE;
pub const MAIN_MS: u64 = 1000 * TICK_RATE;

pub const FAULT_RECOVERY_MS: u64 = 100 * TICK_RATE;

pub const NETWORK_PRIORITY: u8 = 5;
pub const MONITOR_PRIORITY: u8 = 10;
pub const SIMULATION_PRIORITY: u8 = 1;
pub const LOGGING_PRIORITY: u8 = 0;
pub const COMMAND_PRIORITY: u8 = 4;

// pub const NUMBER_OF_THREADS: u64 = 7;
pub const NUMBER_OF_CORES: u64 = 8;

// Network Logic
pub const INIT_HANDSHAKE_LIMIT_MS: u64 = 5 * TICK_RATE;
pub const VISIBILITY_WINDOW_LIMIT_MS: u64 = 30 * TICK_RATE;
pub const VISIBILITY_WINDOW_CYCLE_MS: u64 = 50 * TICK_RATE; // Visible + Invisible
