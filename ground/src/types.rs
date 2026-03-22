use serde::{Serialize, Deserialize};
use std::sync::atomic::{AtomicU32, Ordering, AtomicU64};

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
#[repr(u16)]
pub enum TaskID {
    None = 0,
    RotateAntenna = 101,
    SetPowerMode = 102,
    ClearSubsystemFault = 103,
    RequestRetransmit = 104,
    ThermalSensor = 201,
    PitchAndYawSensor = 202,
    MoistureSensor = 203,
    GlobalSystem = 300,
    NetworkService = 301,
}

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
#[repr(u16)]
pub enum EventID {
    CommandNotFound = 101,
    SubsystemFault = 102,
    SubsystemFixed = 103,
    CommandCompletion = 104,
    StartDelay = 201,
    CompletionDelay = 202,
    TaskFault = 203,
    DataCorruption = 204,
    TaskCompletion = 205,
    DegradedMode = 301,
    NormalMode = 302,
    Startup = 303,
    MissionAbort = 304,
    Shutdown = 305,
    MissedCommunication = 401,
    DataLoss = 402,
    RetransmitFailed = 403,
    SyncStart = 404,
    SyncOngoing = 405,
    SyncCompleted = 406,
    ConnectionStart = 407,
    ConnectionEnd = 408,
    QueuePerformance = 501,
    ResourceUtilization = 502,
}

#[derive(Debug)]
pub struct Metrics {
    pub last_latency_ms: AtomicU64,
    pub total_latency_ms: AtomicU64,
    pub total_jitter_ms: AtomicU64,
    pub number_of_samples: AtomicU32,
}

impl Metrics {
    pub fn insert_new_metric(&self, new_latency: u64) {
        let last_latency = self.last_latency_ms.swap(new_latency, Ordering::Release);
        self.total_latency_ms.fetch_add(new_latency, Ordering::Relaxed);
        self.total_jitter_ms.fetch_add(last_latency.abs_diff(new_latency), Ordering::Relaxed);
        self.number_of_samples.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_jitter(&self) -> u64 {
        let total_jitter = self.total_jitter_ms.load(Ordering::Relaxed);
        let samples = self.number_of_samples.load(Ordering::Relaxed);
        if samples == 0 { return 0; }
        total_jitter / samples as u64
    }

    pub fn get_average_latency(&self) -> u64 {
        let total_latency = self.total_latency_ms.load(Ordering::Relaxed);
        let samples = self.number_of_samples.load(Ordering::Relaxed);
        if samples == 0 { return 0; }
        total_latency / samples as u64
    }
}

#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, Copy)]
pub enum EventData {
    None,
    QueuePerformance { latency_ms: u64, average_latency_ms: u64, jitter_ms: u64, buffer_fill_rate: u32, sample_count: u32 },
    SchedulingDrift { drift_ms: u32 },
    Hardware { value: u32, latency_ms: u64, average_latency_ms: u64, jitter_ms: u64, sample_count: u32 },
    CorruptedHardware { value: u32, recovery_time: u64 },
    Subsystem { subsystem_id: SubsystemID },
    SystemStats { active_ms: u64, inactive_ms: u64 },
    FaultRecovery { recovery_time: u64 },
    TimeSync { offset: u64 },
    PacketDrain { count: u32 },
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
    Emergency = 10,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone)]
pub enum LogSource {
    HealthMonitor = 1,
    Network = 2,
    CommandScheduler = 3,
    Main = 5,
    External = 6, 
}

pub struct Log {
    pub source: LogSource,
    pub event: Event,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Command {
    RotateAntenna { target_angle: u16 },
    SetPowerMode { mode: u8 },
    ClearSubsystemFault { subsystem_id: SubsystemID },
    RequestRetransmit { sequence_no: u32 },
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

    pub fn task_id(&self) -> TaskID {
        match self {
            Command::RotateAntenna { .. } => TaskID::RotateAntenna,
            Command::SetPowerMode { .. } => TaskID::SetPowerMode,
            Command::ClearSubsystemFault { .. } => TaskID::ClearSubsystemFault,
            Command::RequestRetransmit { .. } => TaskID::RequestRetransmit,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Copy, Clone)]
pub enum SatelliteMessage {
    SyncRequest,
    SyncResponse { ground_sent: u64, satellite_receive: u64 },
    SyncResult { offset: u64 },
    Command { command: Command, sent_at: u64 },
    Telemetry { event: Event },
}

impl SatelliteMessage {
    pub fn command_task_id(&self) -> TaskID {
        match self {
            SatelliteMessage::Command { command, .. } => command.task_id(),
            _ => TaskID::NetworkService,
        }
    }
}

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize, Copy, Clone)]
pub struct TelemetryPacket {
    pub priority: Priority,
    pub creation_time: u64,
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
        self.priority.cmp(&other.priority)
            .then(other.creation_time.cmp(&self.creation_time))
    }
}