mod config;
mod logging;
mod network;
mod monitor;
mod types;
mod state;
mod buffer;
mod command;

use std::sync::{Arc, mpsc};
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::thread;

use crate::types::*;
use crate::state::GroundState;
use crate::buffer::BoundedBuffer;
use crate::logging::run_logger;
use crate::network::run_network_thread;
use crate::monitor::run_fault_monitor;
use crate::command::run_command_scheduler;
use crate::config::{UPLINK_BUFFER_CAPACITY, LOG_BUFFER_CAPACITY, MAIN_MS};

fn main() {
    let state = Arc::new(GroundState::new());
    let uplink_buffer = Arc::new(BoundedBuffer::new(UPLINK_BUFFER_CAPACITY));
    let (log_tx, log_rx) = mpsc::sync_channel::<Log>(LOG_BUFFER_CAPACITY);
    let now = state.uptime_ms();

    let logger_handle = thread::spawn(move || {
        run_logger(log_rx);
    });

    let m_state = Arc::clone(&state);
    let m_buffer = Arc::clone(&uplink_buffer);
    let m_log = log_tx.clone();
    thread::spawn(move || { run_fault_monitor(m_state, m_buffer, m_log); });

    let c_state = Arc::clone(&state);
    let c_buffer = Arc::clone(&uplink_buffer);
    let c_log = log_tx.clone();
    thread::spawn(move || { run_command_scheduler(c_state, c_buffer, c_log); });

    let n_state = Arc::clone(&state);
    let n_buffer = Arc::clone(&uplink_buffer);
    let n_log = log_tx.clone();
    thread::spawn(move || { run_network_thread(n_state, n_buffer, n_log); });

    log_tx.send(Log {
        source: LogSource::Main,
        event: Event {
            task_id: TaskID::GlobalSystem,
            event_id: EventID::Startup,
            data: EventData::None,
            timestamp: state.uptime_ms(),
        },
    }).ok();

    state.cpu_active_ms.fetch_add(state.uptime_ms() - now, Ordering::SeqCst);

    let h_state = Arc::clone(&state);

    ctrlc::set_handler(move || {
        println!("\nCtrl+C detected! Shutting down...");
        h_state.is_running.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    while state.is_running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_micros(MAIN_MS));
    }

    log_tx.send(Log {
        source: LogSource::Main,
        event: Event {
            task_id: TaskID::GlobalSystem,
            event_id: EventID::Shutdown,
            data: EventData::None,
            timestamp: state.uptime_ms(),
        },
    }).ok();

    display_summary(&state);

    drop(log_tx);
    
    logger_handle.join().unwrap();
}


pub fn display_summary(state: &GroundState) {
    println!("--------------------------------------SUMMARY--------------------------------------");

    println!("GENERAL METRICS: [ACTIVE_MS: {}, CPU_UTILIZATION: {:.2}%]", 
                state.cpu_active_ms.load(Ordering::Relaxed), 
                state.cpu_active_ms.load(Ordering::Relaxed) as f64 / state.uptime_ms() as f64 * 100.0);

    println!();

    println!("COMMAND METRICS: [{:?}]", state.command_dispatch_latency);

    println!();

    println!("NETWORK METRICS: [{:?}]", state.telemetry_reception_latency);
}