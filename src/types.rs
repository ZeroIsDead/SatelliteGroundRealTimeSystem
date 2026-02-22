use serde::{Serialize, Deserialize};
use std::sync::atomic::{AtomicU32};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
#[repr(u16)]
pub enum TaskID {
    None = 0,
    // Command Tasks
    RotateAntenna = 101,
    SetPowerMode =102,
    ClearSubsystemFault = 103,
    RequestRetransmit = 104,

    // Scheduled Tasks
    ThermalSensor = 201,
    PitchAndYawSensor = 202,
    MoistureSensor = 203,

    GlobalSystem = 300, 
    NetworkService = 301,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
#[repr(u16)]
pub enum EventID {
    // Command Events
    CommandNotFound = 101,
    SubsystemFault = 102, 
    SubsystemFixed = 103,
    CommandCompletion = 104,

    // Scheduled Task Events
    StartDelay = 201,
    CompletionDelay = 202,
    TaskFault = 203,
    DataCorruption = 204,
    TaskCompletion = 205,

    // System Mode Event
    DegradedMode = 301,
    NormalMode = 302,
    Startup = 303,
    MissionAbort = 304,
    Shutdown = 305,

    // Network Events
    MissedCommunication = 401,
    DataLoss = 402,
    PacketTooOld = 403,
    SyncStart = 404,
    SyncCompleted = 405,

    // System Info
    QueuePerformance = 501,   // Latency and Drops - Only Downlink No Event for Uplink
    ResourceUtilization = 502, // CPU and Buffer Fill
}

#[derive(Debug)]
pub struct Metrics {
    pub last_latency_ms: AtomicU32,
    pub jitter_ms: AtomicU32,
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Copy)]
pub enum EventData {
    None,
    QueuePerformance { latency_ms: u32, jitter_ms: u32, buffer_fill_rate: u32 },
    SchedulingDrift { drift_ms: u32},
    Hardware { value: u32 }, // Sensors
    SubsystemFault { subsystem_id: SubsystemID},
    SystemStats {active_ms: u64, inactive_ms: u64}, // SystemState CPU Active ms, System Uptime - Active
    FaultRecovery { recovery_time: u32},
    TimeSync { offset: u64}
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub struct Event {
    pub task_id: TaskID,
    pub event_id: EventID,
    pub data: EventData,
    pub timestamp: u64,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Copy, Clone)]
pub enum SubsystemID {
    Antenna = 0,
    Power = 1,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Copy, Clone)]
#[repr(u16)]
pub enum Priority {
    Low = 0,
    Normal = 3,
    Critical = 9,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone)]
pub enum LogSource {
    HealthMonitor = 1,
    Network = 2,
    Sensor = 3,
    CommandExecutor = 4,
    Main = 5,
}

pub struct Log {
    pub source: LogSource,
    pub event: Event,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Command {
    RotateAntenna { 
        target_angle: u16, 
    },
    SetPowerMode { 
        mode: u8 
    },
    ClearSubsystemFault { 
        subsystem_id: SubsystemID,
    },
    RequestRetransmit { 
        sequence_no: u32 
    },
}

impl Command {
    pub fn required_health(&self) -> Option<SubsystemID> {
        match self {
            Command::RotateAntenna { .. } => Some(SubsystemID::Antenna),
            Command::SetPowerMode { .. } => Some(SubsystemID::Power),
            Command::ClearSubsystemFault { .. } => None, 
            Command::RequestRetransmit { .. } => None, 
        }
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Copy, Clone)]
pub enum SatelliteMessage {
    SyncRequest, 
    SyncResponse { 
        ground_sent: u64, 
        satellite_receive: u64, 
    },
    SyncResult { offset: u64},
    Command {
        command: Command,
        sent_at: u64,
    },
    Telemetry { 
        event: Event,
    },
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Copy, Clone)]
pub struct TelemetryPacket {  // When Pop From Buffer, Recreate creation time with sent time
    pub priority: Priority,
    pub creation_time: u64, // Network Arrival Time or Data Creation Time
    pub payload: SatelliteMessage,
    pub sequence_no: u32,
}

impl PartialOrd for TelemetryPacket {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TelemetryPacket {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.priority.cmp(&self.priority)
            .then(other.creation_time.cmp(&self.creation_time))
    }
}
