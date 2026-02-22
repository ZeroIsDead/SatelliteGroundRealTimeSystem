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
            EventID::TaskFault => "Task Thread Failed",
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
            EventID::PacketTooOld => "Packet Too Old",
            EventID::SyncStart => "Time Sync Start",
            EventID::SyncCompleted => "Time Sync Complete",

            // System Info
            EventID::QueuePerformance => "Queue Performance",  
            EventID::ResourceUtilization => "Resource Utilization",
        };

        let _ = write!(format_buffer, "[Satellite] [{:>7}]\tTask: [{:>30}]\tEvent: [{:>30}]\t", source_str, task_str, event_str);

        match log.event.data {
            EventData::QueuePerformance { latency_ms, jitter_ms, buffer_fill_rate } => {
                let _ = write!(format_buffer, "QUEUE_PERF: [Latency: {}ms, Jitter: {}ms, Buffer Fill Rate: {}%]\t", latency_ms, jitter_ms, buffer_fill_rate);
            }
            EventData::SchedulingDrift { drift_ms  } => {
                let _ = write!(format_buffer, "DRIFT: [{}ms]\t", drift_ms);
            }
            EventData::Hardware { value } => {
                let _ = write!(format_buffer, "VALUE: [{}]\t", value as f32 / 100 as f32); // Sensor Data Stored as Integer -> (Float with 2 Decimal Places)
            }
            EventData::SubsystemFault { subsystem_id }=> {
                let _ = write!(format_buffer, "SUBSYSTEM: [{}]\t", match subsystem_id {
                    SubsystemID::Antenna => "Antenna",
                    SubsystemID::Power => "Power",
                });
            }
            EventData::SystemStats { active_ms, inactive_ms } => {
                let _ = write!(format_buffer, "CPU: Active: [{}ms, Inactive: {}ms]\t", active_ms, inactive_ms);

            }
            EventData::FaultRecovery { recovery_time } => {
                let _ = write!(format_buffer, "RECOVERY TIME: [{}ms]\t", recovery_time);
            }
            EventData::TimeSync { offset } => {
                let _ = write!(format_buffer, "OFFSET: [{}ms]\t", offset);
            }
            EventData::None => {}
        }

        let _ = write!(format_buffer, "Time: [{} uptime ms]\t", log.event.timestamp);

        println!("{}", format_buffer);

        if let Err(e) = writeln!(file, "{}", format_buffer) {
            eprintln!("Failed to write to disk: {}", e);
        }
    }
}