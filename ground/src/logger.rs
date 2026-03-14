use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::fmt::Write as FmtWrite;
use std::sync::mpsc::Receiver;
use crate::types::{GcsLogMessage, GcsEvent};

pub fn run_gcs_logger(log_rx: Receiver<GcsLogMessage>) {
    let mut file = OpenOptions::new().create(true).append(true).open("gcs_mission.log").unwrap();
    let mut buffer = String::with_capacity(256);

    println!("[Logger] Writing mission metrics to gcs_mission.log...");

    while let Ok(msg) = log_rx.recv() {
        buffer.clear();
        let _ = write!(&mut buffer, "[{:08}ms] | ", msg.timestamp_ms);

        match msg.event {
            GcsEvent::TelemetryRx { seq, decode_time_ms, drift_ms } => {
                let _ = write!(&mut buffer, "TELEMETRY_RX | Seq: {} | Decode: {}ms | Jitter: {}ms", seq, decode_time_ms, drift_ms);
            }
            GcsEvent::CommandDispatch { cmd_name, dispatch_time_ms } => {
                let _ = write!(&mut buffer, "CMD_DISPATCH | {} | Latency: {}ms", cmd_name, dispatch_time_ms);
            }
            GcsEvent::CommandRejected { cmd_name, reason } => {
                let _ = write!(&mut buffer, "CMD_REJECTED | {} | Reason: {}", cmd_name, reason);
            }
            GcsEvent::DeadlineMiss { task, limit_ms, actual_ms } => {
                let _ = write!(&mut buffer, "DEADLINE_MISS| Task: {} | Limit: {}ms | Actual: {}ms", task, limit_ms, actual_ms);
            }
            GcsEvent::FaultAlert { subsystem_id, response_latency_ms } => {
                let _ = write!(&mut buffer, "FAULT_ALERT  | Subsys: {} | Response: {}ms", subsystem_id, response_latency_ms);
            }
            GcsEvent::LossOfContact { missing_packets } => {
                let _ = write!(&mut buffer, "CONTACT_LOST | Gap: {} packets", missing_packets);
            }
        }

        let _ = writeln!(file, "{}", buffer);
    }
}