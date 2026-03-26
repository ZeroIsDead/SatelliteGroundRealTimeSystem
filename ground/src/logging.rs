use crate::config::LOGGING_PRIORITY;
use crate::types::{Log, LogSource, TaskID, EventID, EventData, SubsystemID, Priority};
use std::sync::mpsc::Receiver;
use thread_priority::*;
use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::fmt::Write as FmtWrite;

pub fn run_logger(log_rx: Receiver<Log>) {
    set_current_thread_priority(ThreadPriority::Crossplatform(
        LOGGING_PRIORITY.try_into().unwrap()
    )).unwrap();

    let mut format_buffer = String::with_capacity(512);
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open("gcs_mission.log")
        .expect("Failed to open GCS log file");

    while let Ok(log) = log_rx.recv() {
        format_buffer.clear();

        let source_str = format_source(&log.source);
        let task_str = format_task(&log.event.task_id);
        let event_str = format_event_id(&log.event.event_id, &log.source);

        let _ = write!(
            format_buffer,
            "[GCS] [{:>8}]\tTask: [{:>30}]\tEvent: [{:>35}]\t",
            source_str, task_str, event_str
        );

        format_event_data(&mut format_buffer, &log.event.data);

        let _ = write!(format_buffer, "Timestamp: [{} μs uptime]", log.event.timestamp);

        if let Err(e) = writeln!(file, "{}", format_buffer) {
            eprintln!("Failed to write to GCS log: {}", e);
        }
    }
}

fn format_source(source: &LogSource) -> &'static str {
    match source {
        LogSource::HealthMonitor  => "HEALTH",
        LogSource::Network        => "NETWORK",
        LogSource::CommandScheduler => "CMD_SCH",
        LogSource::Main           => "MAIN",
        LogSource::External       => "SAT",
    }
}

fn format_task(task: &TaskID) -> &'static str {
    match task {
        TaskID::RotateAntenna      => "Rotate Antenna",
        TaskID::SetPowerMode       => "Set Power Mode",
        TaskID::ClearSubsystemFault => "Clear Subsystem Fault",
        TaskID::RequestRetransmit  => "Request Retransmit",
        TaskID::ThermalSensor      => "Thermal Sensor",
        TaskID::PitchAndYawSensor  => "Pitch & Yaw Sensor",
        TaskID::MoistureSensor     => "Moisture Sensor",
        TaskID::GlobalSystem       => "Global System",
        TaskID::NetworkService     => "Network Service",
        TaskID::None               => "-",
    }
}

fn format_event_id(event_id: &EventID, source: &LogSource) -> &'static str {
    let is_external = matches!(source, LogSource::External);
    match event_id {
        EventID::CommandNotFound    => if is_external { "Satellite: Command Not Found" }     else { "Command Rejected (Interlock)" },
        EventID::SubsystemFault     => if is_external { "Satellite: Subsystem Fault" }       else { "Subsystem Interlock Armed" },
        EventID::SubsystemFixed     => if is_external { "Satellite: Subsystem Cleared" }     else { "Subsystem Interlock Released" },
        EventID::CommandCompletion  => if is_external { "Satellite: Command Completed" }     else { "Command Dispatched" },
        EventID::StartDelay         => if is_external { "Satellite: Task Start Delay" }      else { "Command Start Delay" },
        EventID::CompletionDelay    => if is_external { "Satellite: Task Completion Delay" } else { "Deadline Violation" },
        EventID::TaskFault          => if is_external { "Satellite: Task Fault" }            else { "Task Fault" },
        EventID::DataCorruption     => if is_external { "Satellite: Data Corrupted" }        else { "Data Corrupted" },
        EventID::TaskCompletion     => if is_external { "Satellite: Task Completed" }        else { "Command Scheduled" },
        EventID::DegradedMode       => "Satellite: Degraded Mode Entered",
        EventID::NormalMode         => "Satellite: Normal Mode Restored",
        EventID::Startup            => if is_external { "Satellite: Initialized" }           else { "GCS Initialized" },
        EventID::MissionAbort       => if is_external { "Satellite: Mission Abort" }         else { "GCS Critical Alert" },
        EventID::Shutdown           => if is_external { "Satellite: Shutdown" }              else { "GCS Shutdown" },
        EventID::MissedCommunication => if is_external { "Satellite: Missed Comm Window" }  else { "Loss of Contact" },
        EventID::DataLoss           => if is_external { "Satellite: Packet Dropped" }       else { "Uplink Packet Dropped" },
        EventID::RetransmitFailed   => if is_external { "Satellite: Retransmit Failed" }    else { "Retransmit Failed" },
        EventID::SyncStart          => "Clock Sync Started",
        EventID::SyncOngoing        => "Clock Sync Ongoing",
        EventID::SyncCompleted      => if is_external { "Satellite: Clock Calibrated" }     else { "Clock Sync Offset Sent" },
        EventID::ConnectionStart    => "Connection Started",
        EventID::ConnectionEnd      => "Connection End",
        EventID::QueuePerformance   => if is_external { "Satellite: Queue Performance" }    else { "GCS Queue Performance" },
        EventID::ResourceUtilization => if is_external { "Satellite: CPU Utilization" }     else { "GCS CPU Utilization" },
        EventID::NetworkPerformance => "Network Performance"
    }
}

fn format_event_data(buf: &mut String, data: &EventData) {
    match data {
        EventData::None => {}

        EventData::QueuePerformance { latency_ms, jitter_ms, buffer_fill_rate, sample_count } => {
            let _ = write!(buf,
                "Latency: {}μs Jitter: {}μs  Fill: {}%  Samples: {}\t",
                latency_ms, jitter_ms, buffer_fill_rate, sample_count
            );
        }

        EventData::SchedulingDrift { drift_ms } => {
            let _ = write!(buf, "Drift: {}μs\t", drift_ms);
        }

        EventData::Hardware { value, latency_ms, jitter_ms, sample_count } => {
            let _ = write!(buf,
                "Value: {:.2}  Latency: {}μs Jitter: {}μs  Samples: {}\t",
                (*value as f32) / 100.0, latency_ms, jitter_ms, sample_count
            );
        }

        EventData::CorruptedHardware { value, recovery_time } => {
            let _ = write!(buf,
                "Corrupted Value: {:.2}  Recovery: {}μs\t",
                (*value as f32) / 100.0, recovery_time
            );
        }

        EventData::Subsystem { subsystem_id } => {
            let _ = write!(buf, "Subsystem: {}\t", match subsystem_id {
                SubsystemID::Antenna => "Antenna",
                SubsystemID::Power   => "Power",
            });
        }

        EventData::SystemStats { active_ms, inactive_ms } => {
            let total = active_ms + inactive_ms;
            let pct = if total > 0 { (active_ms * 100) / total } else { 0 };
            let _ = write!(buf,
                "Active: {}μs  Idle: {}μs  Utilization: {}%\t",
                active_ms, inactive_ms, pct
            );
        }

        EventData::FaultRecovery { recovery_time } => {
            let _ = write!(buf, "Recovery Time: {}μs\t", recovery_time);
        }

        EventData::TimeSync { offset } => {
            let _ = write!(buf, "Clock Offset: {}μs\t", offset);
        }

        EventData::PacketDrain { count } => {
            let _ = write!(buf, "Count: {}\t", count);
        }

        EventData::NetworkPerformance { priority, latency_ms, jitter_ms, sample_count } => {
            let priority_string = match priority {
                Priority::Emergency => "Emergency",
                Priority::Critical => "Critical",
                Priority::Normal => "Normal",
                Priority::Low => "Low"
            };

            let _ = write!(buf, "NETWORK PERFORMANCE: Priority: {} Latency: {}μs Jitter: {}μs Sample Count: {}\t", priority_string, latency_ms, jitter_ms, sample_count);
        }
    }
}