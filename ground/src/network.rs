use std::io::{Read, ErrorKind};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::time::Instant;

use crate::config::{DECODE_DEADLINE_MS, FAULT_RESPONSE_LIMIT_MS, MAX_LOSS_GAP, TCP_READ_TIMEOUT};
use crate::types::{SatelliteMessage, PayloadType};
use crate::state::GcsState;
use crate::logger::{GcsLogMessage, GcsEvent};

pub fn run_telemetry_listener(
    mut stream: TcpStream, 
    state: Arc<GcsState>, 
    log_tx: SyncSender<GcsLogMessage>,
    gcs_boot_time: Instant,
) {
    // Prevent the thread from deadlocking if the satellite crashes
    stream.set_read_timeout(Some(TCP_READ_TIMEOUT)).unwrap();
    let mut expected_seq: u32 = 0;

    println!("[Network] Telemetry listener active.");

    while state.is_running.load(Ordering::Relaxed) {
        // 1. Read Length Header (2 bytes)
        let mut len_buf = [0u8; 2];
        if let Err(e) = stream.read_exact(&mut len_buf) {
            if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut {
                continue; // Normal timeout, keep looping
            }
            println!("[Network] Connection dropped: {}", e);
            break;
        }
        let len = u16::from_be_bytes(len_buf) as usize;

        // 2. Read Payload
        let mut buffer = vec![0u8; len];
        if stream.read_exact(&mut buffer).is_err() { break; }

        // --- REQUIREMENT 1: 3ms Decode Deadline ---
        let decode_start = Instant::now();
        
        let packet: SatelliteMessage = match bincode::deserialize(&buffer) {
            Ok(p) => p,
            Err(_) => continue,
        };
        
        let decode_duration = decode_start.elapsed().as_millis();
        let current_gcs_time = gcs_boot_time.elapsed().as_millis() as u64;

        if decode_duration > DECODE_DEADLINE_MS {
            state.metrics.record_deadline_miss();
            let _ = log_tx.try_send(GcsLogMessage {
                timestamp_ms: current_gcs_time,
                event: GcsEvent::DeadlineMiss { 
                    task: "Decode", limit_ms: DECODE_DEADLINE_MS, actual_ms: decode_duration 
                }
            });
        }

        // 3. Process the Data
        if let SatelliteMessage::Downlink(telemetry) = packet {
            let seq = telemetry.sequence_no;
            state.metrics.increment_received();
            state.metrics.update_highest_seq(seq);

            // Jitter & Loss Calculation
            let jitter = state.metrics.calculate_jitter(seq, current_gcs_time);
            if seq > expected_seq && (seq - expected_seq) >= MAX_LOSS_GAP {
                state.interlocks.set_loss_of_contact(true);
                let _ = log_tx.try_send(GcsLogMessage {
                    timestamp_ms: current_gcs_time,
                    event: GcsEvent::LossOfContact { missing_packets: seq - expected_seq }
                });
            }
            if seq >= expected_seq { expected_seq = seq + 1; }

            // Log Telemetry RX
            let _ = log_tx.try_send(GcsLogMessage {
                timestamp_ms: current_gcs_time,
                event: GcsEvent::TelemetryRx { seq, decode_time_ms: decode_duration, drift_ms: jitter }
            });

            // --- REQUIREMENT 3: Fault Latency (<100ms) ---
            match telemetry.payload {
                PayloadType::FaultInjected { subsystem_id, .. } => {
                    let reaction_latency = current_gcs_time.saturating_sub(telemetry.timestamp);
                    state.interlocks.set_subsystem_lock(subsystem_id, true);

                    if reaction_latency > FAULT_RESPONSE_LIMIT_MS as u64 {
                        let _ = log_tx.try_send(GcsLogMessage {
                            timestamp_ms: current_gcs_time,
                            event: GcsEvent::FaultAlert { subsystem_id, response_latency_ms: reaction_latency }
                        });
                    }
                },
                PayloadType::FaultResolved { subsystem_id } => {
                    state.interlocks.set_subsystem_lock(subsystem_id, false);
                },
                _ => {}
            }
        }
    }
}