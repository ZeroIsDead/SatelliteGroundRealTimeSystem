pub const TICK_RATE: u64 = 1; // 1ms
pub const SENSOR_FAULT_NOT_CONFIRMED: u16 = 0; // SENSOR FAULT TYPE NOT SET
pub const SEQUENCE_NOT_CONFIRMED: u32 = 0; // SEQUENCE_NO is only set in network thread so any other thread will set as NOT_CONFIRMED
pub const TIMESTAMP_NOT_CONFIRMED: u64 = 0; // FAULT TIMESTAMP NOT YET SET

pub const DATA_BUFFER_CAPACITY: usize = 256; // 1 Byte
pub const LOG_BUFFER_CAPACITY: usize = 256; // 1 Byte
pub const PACKET_HISTORY_BUFFER_CAPACITY: usize = 1024; // 10 Bits
pub const NORMAL_TO_DEGRADED_THRESHOLD: u32 = 80; // 80%
pub const DEGRADED_TO_NORMAL_THRESHOLD: u32 = 50; // 50%
pub const NETWORK_PORT: &str = "0.0.0.0:8000";

pub const MAX_SENSORS: usize = 3;
pub const MAX_SUBSYSTEM: usize = 2;

// Frequencies (in Milliseconds)
pub const SUBSYSTEM_FAULT_INJECTION_MS: u64 = 60 * TICK_RATE;
pub const SENSOR_FAULT_INJECTION_MS: u64 = 60 * TICK_RATE;
pub const SENSOR_FAULT_MS: u64 = 10 * TICK_RATE;
pub const SENSOR_DELAY_MS: u64 = 2 * TICK_RATE;
pub const SENSOR_DATA_CORRUPTION: u32 = 99999;
pub const SENSOR_INCREMENT_MAX: u32 = 101;
pub const MONITOR_MS: u64 = 5 * TICK_RATE;
pub const COMMAND_MS: u64 = 5 * TICK_RATE;
pub const NETWORK_MS: u64 = 2 * TICK_RATE;
pub const MAIN_MS: u64 = 1000 * TICK_RATE;

pub const FAULT_RECOVERY_MS: u64 = 200 * TICK_RATE;

// Thread Priority List
pub const MONITOR_PRIORITY: u8 = 10;
// Thermal Sensor = 9
pub const NETWORK_PRIORITY: u8 = 5;
pub const COMMAND_PRIORITY: u8 = 4;
// Pitch/Yaw Sensor = 3
pub const SIMULATION_PRIORITY: u8 = 1;
// Moisture Sensor = 0
pub const LOGGING_PRIORITY: u8 = 0;
// Sensor Priorities - Moisture = 0, Pitch/Yaw = 3, Thermal = 9 (Based on Priority Enum)

// pub const NUMBER_OF_THREADS: u64 = 7;
pub const NUMBER_OF_CORES: u64 = 4; // Lower Amount to account for other computer and also since both satellite and ground in one computer

// Network Logic
pub const INIT_HANDSHAKE_LIMIT_MS: u64 = 5 * TICK_RATE;
pub const VISIBILITY_WINDOW_LIMIT_MS: u64 = 30 * TICK_RATE;
pub const VISIBILITY_WINDOW_CYCLE_MS: u64 = 50 * TICK_RATE; // Visible + Invisible
