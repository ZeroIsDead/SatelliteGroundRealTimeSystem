mod config;
mod types;
mod state;
mod network;
mod scheduler;
mod logger;

use std::net::TcpStream;
use std::sync::{Arc, mpsc::sync_channel};
use std::thread;
use std::time::{Instant, Duration};
use std::sync::atomic::Ordering;

use crate::types::Command;
use crate::config::*;
// use crate::state::GcsState;

fn main() {
    println!("--- GCS INITIALIZING ---");
    let gcs_boot_time = Instant::now();

    let state = Arc::new(GcsState::new());
    let (log_tx, log_rx) = sync_channel(LOG_CHANNEL_CAPACITY);
    let (cmd_tx, cmd_rx) = sync_channel(CMD_QUEUE_CAPACITY);

    // Spawn Logger
    thread::spawn(move || logger::run_gcs_logger(log_rx));

    // Connect to Satellite
    let address = format!("{}:{}", SAT_IP, SAT_PORT);
    let stream = TcpStream::connect(&address).expect("FATAL: Could not connect to Satellite");
    
    // Spawn Network Thread
    let rx_stream = stream.try_clone().unwrap();
    let state_net = Arc::clone(&state);
    let log_tx_net = log_tx.clone();
    thread::spawn(move || network::run_telemetry_listener(rx_stream, state_net, log_tx_net, gcs_boot_time));

    // Spawn Scheduler Thread
    let tx_stream = stream.try_clone().unwrap();
    let state_sched = Arc::clone(&state);
    let log_tx_sched = log_tx.clone();
    thread::spawn(move || scheduler::run_command_scheduler(tx_stream, cmd_rx, state_sched, log_tx_sched, gcs_boot_time));

    // --- AUTOMATED MISSION SEQUENCER ---
    let mut schedule = vec![
        (5000, Command::RotateAntenna { target_angle: 90 }),
        (10000, Command::SetPowerMode { mode: 1 }),
        (15000, Command::RotateAntenna { target_angle: 180 }),
    ];

    while gcs_boot_time.elapsed().as_millis() < MISSION_DURATION_MS as u128 {
        let current_ms = gcs_boot_time.elapsed().as_millis() as u64;

        schedule.retain(|(time, cmd)| {
            if current_ms >= *time {
                let _ = cmd_tx.send(cmd.clone());
                false // dispatch and remove
            } else {
                true // keep waiting
            }
        });

        if !state.is_running.load(Ordering::Relaxed) { break; }
        thread::sleep(Duration::from_millis(10));
    }

    println!("[Mission] Complete. Shutting down.");
    state.is_running.store(false, Ordering::Relaxed);
    thread::sleep(Duration::from_millis(500)); // allow logs to flush
}
