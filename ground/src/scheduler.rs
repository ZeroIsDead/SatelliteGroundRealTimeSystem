use std::collections::BinaryHeap;
use std::io::Write;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, SyncSender};
use std::time::Instant;
use std::cmp::Ordering as StdOrdering;

use crate::config::{DISPATCH_DEADLINE_MS, SCHEDULER_TICK_RATE};
use crate::types::{Command, SatelliteMessage};
use crate::state::GcsState;
use crate::logger::{GcsLogMessage, GcsEvent};

#[derive(Debug)]
struct PrioritizedCommand {
    priority: u8,
    cmd: Command,
}

impl Ord for PrioritizedCommand {
    fn cmp(&self, other: &Self) -> StdOrdering { self.priority.cmp(&other.priority) }
}
impl PartialOrd for PrioritizedCommand {
    fn partial_cmp(&self, other: &Self) -> Option<StdOrdering> { Some(self.cmp(other)) }
}
impl PartialEq for PrioritizedCommand {
    fn eq(&self, other: &Self) -> bool { self.priority == other.priority }
}
impl Eq for PrioritizedCommand {}

pub fn run_command_scheduler(
    mut stream: TcpStream,
    cmd_rx: Receiver<Command>,
    state: Arc<GcsState>,
    log_tx: SyncSender<GcsLogMessage>,
    gcs_boot_time: Instant,
) {
    let mut queue: BinaryHeap<PrioritizedCommand> = BinaryHeap::new();
    println!("[Scheduler] Command scheduler active.");

    while state.is_running.load(Ordering::Relaxed) {
        let loop_start = Instant::now();
        let current_gcs_time = gcs_boot_time.elapsed().as_millis() as u64;

        // 1. Drain pending commands into Priority Queue
        while let Ok(cmd) = cmd_rx.try_recv() {
            let priority = if cmd.is_urgent() { 255 } else { 10 };
            queue.push(PrioritizedCommand { priority, cmd });
        }

        // 2. Process highest priority command
        if let Some(p_cmd) = queue.pop() {
            let cmd_name = get_cmd_name(&p_cmd.cmd);

            // --- REQUIREMENT 3: Interlock Validation ---
            if state.interlocks.is_command_safe(&p_cmd.cmd) {
                
                // --- REQUIREMENT 2: 2ms Dispatch Deadline ---
                let dispatch_start = Instant::now();
                
                let msg = SatelliteMessage::Uplink(p_cmd.cmd.clone());
                let bytes = bincode::serialize(&msg).unwrap();
                let len_header = (bytes.len() as u16).to_be_bytes();

                if stream.write_all(&len_header).is_ok() && stream.write_all(&bytes).is_ok() {
                    let dispatch_duration = dispatch_start.elapsed().as_millis();
                    
                    if dispatch_duration > DISPATCH_DEADLINE_MS {
                        state.metrics.record_deadline_miss();
                        let _ = log_tx.try_send(GcsLogMessage {
                            timestamp_ms: current_gcs_time,
                            event: GcsEvent::DeadlineMiss {
                                task: "Dispatch", limit_ms: DISPATCH_DEADLINE_MS, actual_ms: dispatch_duration
                            }
                        });
                    }

                    let _ = log_tx.try_send(GcsLogMessage {
                        timestamp_ms: current_gcs_time,
                        event: GcsEvent::CommandDispatch { cmd_name, dispatch_time_ms: dispatch_duration }
                    });
                }
            } else {
                // Command Rejected by Interlock
                let _ = log_tx.try_send(GcsLogMessage {
                    timestamp_ms: current_gcs_time,
                    event: GcsEvent::CommandRejected { cmd_name, reason: "Safety Interlock Active" }
                });
            }
        }

        // Maintain polling rate to avoid 100% CPU usage
        let elapsed = loop_start.elapsed();
        if elapsed < SCHEDULER_TICK_RATE {
            std::thread::sleep(SCHEDULER_TICK_RATE - elapsed);
        }
    }
}

fn get_cmd_name(cmd: &Command) -> &'static str {
    match cmd {
        Command::RotateAntenna { .. } => "RotateAntenna",
        Command::SetPowerMode { .. } => "SetPowerMode",
        Command::ClearFault { .. } => "ClearFault",
        Command::RequestRetransmit { .. } => "RequestRetransmit",
        Command::SystemReboot => "SystemReboot",
    }
}