use std::io::{self, Read};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;
use crate::types::{SatelliteMessage, PayloadType, DECODE_DEADLINE_MS, MAX_LOSS_GAP, FAULT_RESPONSE_LIMIT_MS};
use crate::interlock::InterlockManager;

pub fn run_telemetry_listener(mut stream: TcpStream, interlocks: Arc<InterlockManager>) {
    let mut expected_seq: u32 = 0;

    loop {
        // 1. Read Length Header (2 bytes)
        let mut len_buf = [0u8; 2];
        if stream.read_exact(&mut len_buf).is_err() { continue; }
        let len = u16::from_be_bytes(len_buf) as usize;

        // 2. Read Payload Bytes
        let mut buffer = vec![0u8; len];
        if stream.read_exact(&mut buffer).is_err() { continue; }

        // --- TIMING START (Requirement 1: Decode within 3ms) ---
        let decode_start = Instant::now();
        
        let packet: SatelliteMessage = match bincode::deserialize(&buffer) {
            Ok(p) => p,
            Err(_) => continue,
        };

        let decode_duration = decode_start.elapsed().as_millis();
        if decode_duration > DECODE_DEADLINE_MS {
            // Log it, but KEEP the packet!
            println!("[LOG] Deadline Missed: Decode took {}ms (Limit: 3ms)", decode_duration);
        }
        // --- TIMING END ---

        if let SatelliteMessage::Downlink(telemetry) = packet {
            let seq = telemetry.sequence_no;

            // --- GAP DETECTION (Requirement 1: Loss of Contact) ---
            if seq > expected_seq {
                let gap = seq - expected_seq;
                if gap >= MAX_LOSS_GAP {
                    println!("[CRITICAL] Gap of {} detected! Triggering Loss of Contact protocol.", gap);
                    interlocks.trigger_loss_of_contact();
                } else {
                    // Send RequestRetransmit for the missing packets to the Uplink queue...
                    println!("[WARN] Gap detected. Missing sequences {} to {}", expected_seq, seq - 1);
                }
            }
            // Update expected (handle out-of-order retransmits safely)
            if seq >= expected_seq { expected_seq = seq + 1; }

            // --- FAULT PROCESSING (Requirement 3: 100ms Reaction) ---
            match telemetry.payload {
                PayloadType::FaultInjected { subsystem_id, .. } => {
                    interlocks.set_subsystem_lock(subsystem_id, true);
                    println!("[GCS] SUBSYSTEM {} LOCKED: Fault Detected.", subsystem_id);
                },
                PayloadType::FaultResolved { subsystem_id } => {
                    println!("[GCS] Satellite reports Subsystem {} recovered. Uplinking ClearFault...", subsystem_id);
                    
                    // --- AUTONOMOUS REPLY ---
                    // We push a high-priority ClearFault command directly to the scheduler
                    let recovery_cmd = Command::ClearFault { subsystem_id };
                    let _ = cmd_tx.send(recovery_cmd); 
                    
                    // Unlock locally so the next scheduled command can pass through
                    interlocks.set_subsystem_lock(subsystem_id, false);
                },
                _ => {}
            }

        }
    }
}

// Dummy function for compilation context
fn get_gcs_uptime_ms() -> u64 { 0 }