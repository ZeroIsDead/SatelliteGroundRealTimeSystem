use crate::{config::LOGGING_PRIORITY, types::{Log, SubsystemID}};
use std::sync::mpsc::{Receiver};
use thread_priority::*;
use crate::types::{LogSource, TaskID, EventID, EventData};
use std::fs::OpenOptions;
use std::io::Write as IoWrite;  
use std::fmt::Write as FmtWrite; 

pub fn run_logger(log_rx: Receiver<Log>) {
    set_current_thread_priority(ThreadPriority::Crossplatform(LOGGING_PRIORITY.try_into().unwrap())).unwrap();

    let mut format_buffer = String::with_capacity(256);
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("satellite_mission.log")
        .expect("Failed to open log file");

    while let Ok(log) = log_rx.recv() {
        format_buffer.clear();

        let source_str = match log.source {
            LogSource::HealthMonitor => "HEALTH",
            LogSource::Network => "NETWORK",
            LogSource::Sensor => "SENSOR",
            LogSource::CommandExecutor => "CMD_EXE",
            LogSource::Main => "MAIN",
        };

        let task_str = match log.event.task_id {
            TaskID::RotateAntenna => "Rotate Antenna Command",
            TaskID::SetPowerMode => "Set Power Mode Command",
            TaskID::ClearSubsystemFault => "Clear Subsystem Fault Command",
            TaskID::RequestRetransmit => "Request Retransmit Command",

            // Scheduled Tasks
            TaskID::ThermalSensor => "Thermal Sensor",
            TaskID::PitchAndYawSensor => "Pitch & Yaw Sensor",
            TaskID::MoistureSensor => "Moisture Sensor",

            TaskID::GlobalSystem => "Global", 
            TaskID::NetworkService => "Network Service",
            TaskID::None => ""
        };

        let event_str = match log.event.event_id {
            EventID::CommandNotFound => "Command Not Found",
            EventID::SubsystemFault => "Subsystem Fault Found", 
            EventID::SubsystemFixed => "Subsystem Fault Fixed",
            EventID::CommandCompletion => "Command Completed",

            // Scheduled Task Events
            EventID::StartDelay => "Task Scheduling Drift",
            EventID::CompletionDelay => "Task Completion Delay",
            EventID::TaskFault => "Task Failed",
            EventID::DataCorruption => "Data Corrupted",
            EventID::TaskCompletion => "Task Completed",

            // System Mode Event
            EventID::DegradedMode => "System Status Turned Degraded",
            EventID::NormalMode => "System Status Returned to Normal",
            EventID::Startup => "Satellite Initialized",
            EventID::MissionAbort => "Mission Abort",
            EventID::Shutdown => "Shutdown",

            // Network Events
            EventID::MissedCommunication => "Communication Window Missed",
            EventID::DataLoss => "Packet Dropped",
            EventID::RetransmitFailed => "Retransmit Failed",
            EventID::SyncStart => "Time Sync Start",
            EventID::SyncOngoing => "Time Sync Ongoing",
            EventID::SyncCompleted => "Time Sync Complete",
            EventID::ConnectionStart => "Connection Start",
            EventID::ConnectionEnd => "Connection End",

            // System Info
            EventID::QueuePerformance => "Queue Performance",  
            EventID::ResourceUtilization => "Resource Utilization",
        };

        let _ = write!(format_buffer, "[Satellite] [{:>7}]\tTask: [{:>30}]\tEvent: [{:>30}]\t", source_str, task_str, event_str);

        match log.event.data {
            EventData::QueuePerformance { latency_ms, average_latency_ms, jitter_ms, buffer_fill_rate, sample_count } => {
                let _ = write!(format_buffer, "QUEUE_PERF: [Latency: {}μs, Average Latency: {}μs, Jitter: {}μs, Buffer Fill Rate: {}%, Sample Count: {}]\t", latency_ms, average_latency_ms, jitter_ms, buffer_fill_rate, sample_count);
            }
            EventData::SchedulingDrift { drift_ms  } => {
                let _ = write!(format_buffer, "DRIFT: [{}μs]\t", drift_ms);
            }
            EventData::Hardware { value, latency_ms, average_latency_ms, jitter_ms, sample_count } => {
                let _ = write!(format_buffer, "HARDWARE: [Value: {}, Latency: {}μs, Average Latency: {}μs, Jitter: {}μs, Sample Count: {}]\t", (value / 100) as f32,  latency_ms, average_latency_ms, jitter_ms, sample_count); // Sensor Data Stored as Integer -> (Float with 2 Decimal Places)
            }
            EventData::CorruptedHardware { value, recovery_time } => {
let _ = write!(format_buffer, "CORRUPTED_HARDWARE: [Value: {}, Recovery Time: {}μs]\t", (value / 100) as f32, recovery_time); // Sensor Data Stored as Integer -> (Float with 2 Decimal Places)
            }
            EventData::Subsystem { subsystem_id } => {
                let _ = write!(format_buffer, "SUBSYSTEM: [{}]\t", match subsystem_id {
                    SubsystemID::Antenna => "Antenna",
                    SubsystemID::Power => "Power",
                });
            }
            EventData::SystemStats { active_ms, inactive_ms } => {
                let _ = write!(format_buffer, "CPU: Active: [{}μs, Inactive: {}μs]\t", active_ms, inactive_ms);
            }
            EventData::FaultRecovery { recovery_time } => {
                let _ = write!(format_buffer, "RECOVERY TIME: [{}μs]\t", recovery_time);
            }
            EventData::TimeSync { offset } => {
                let _ = write!(format_buffer, "OFFSET: [{}μs]\t", offset);
            }
            EventData::None => {}
        }

        let _ = write!(format_buffer, "Event Timestamp: [{} uptime μs]\t", log.event.timestamp);

        // println!("{}", format_buffer);

        if let Err(e) = writeln!(file, "{}", format_buffer) {
            eprintln!("Failed to write to disk: {}", e);
        }
    }
}