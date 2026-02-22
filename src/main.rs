mod config;
mod logging;
mod network;
mod monitor;
mod sensors;
mod simulation;
mod types;
mod state;
mod buffer;
mod command;

use std::time::Duration;

use std::sync::{Arc, mpsc};
use std::sync::atomic::{Ordering};
use std::thread;
use crate::types::*;
use crate::state::*;
use crate::buffer::*;
use crate::simulation::run_simulation;
use crate::network::run_network_thread;
use crate::sensors::run_sensor_task;
use crate::logging::run_logger;
use crate::monitor::run_health_monitor;
use crate::command::run_command_executor;
use crate::config::{DATA_BUFFER_CAPACITY, LOG_BUFFER_CAPACITY, MAIN_MS};

fn main() {
    let state = Arc::new(SatelliteState::new());
    let downlink_buffer = Arc::new(BoundedBuffer::new(DATA_BUFFER_CAPACITY));
    let uplink_buffer = Arc::new(BoundedBuffer::new(DATA_BUFFER_CAPACITY));

    let now = state.uptime_ms();

    let (log_tx, log_rx) = mpsc::sync_channel::<Log>(LOG_BUFFER_CAPACITY);

    let logger_handle = thread::spawn(move || {
        run_logger(log_rx);
    });

    let sim_state = Arc::clone(&state);
    thread::spawn(move || {
        run_simulation(sim_state);
    });

    for i in 0..3 {
        let s_state = Arc::clone(&state);
        let s_buffer = Arc::clone(&downlink_buffer);
        let s_log = log_tx.clone();
        
        thread::Builder::new()
            .name(format!("Sensor_{}", i))
            .spawn(move || {
                run_sensor_task(s_state, i, s_buffer, s_log);
            }).expect("Failed to spawn sensor thread");
    }

    let h_state = Arc::clone(&state);
    let h_buffer = Arc::clone(&downlink_buffer);
    let h_log = log_tx.clone();
    thread::spawn(move || {
        run_health_monitor(h_state, h_buffer, h_log);
    });

    let c_state = Arc::clone(&state);
    let c_downlink_buffer = Arc::clone(&uplink_buffer);
    let c_uplink_buffer = Arc::clone(&uplink_buffer);
    let c_log = log_tx.clone();
    thread::spawn(move || {
        run_command_executor(c_state, c_downlink_buffer, c_uplink_buffer, c_log);
    });

    let n_state = Arc::clone(&state);
    let n_downlink_buffer = Arc::clone(&downlink_buffer);
    let n_uplink_buffer = Arc::clone(&uplink_buffer);
    let n_log = log_tx.clone();
    thread::spawn(move || {
        run_network_thread(n_state, n_downlink_buffer, n_uplink_buffer, n_log);
    });

    log_tx.send(Log {
        source: LogSource::Main,
        event: Event {
            task_id: TaskID::GlobalSystem,
            event_id: EventID::Startup,
            data: EventData::None,
            timestamp: state.uptime_ms(),
        }
    }).ok();

    state.cpu_active_ms.fetch_add(state.uptime_ms() - now, Ordering::SeqCst);

    while state.is_running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(MAIN_MS));
    }

    log_tx.send(Log {
        source: LogSource::Main,
        event: Event {
            task_id: TaskID::GlobalSystem,
            event_id: EventID::Shutdown,
            data: EventData::None,
            timestamp: state.uptime_ms(),
        }
    }).ok();

    drop(log_tx); // Drop Sender so the Receiver Know There is No More Logs

    logger_handle.join().unwrap(); // Wait until All Logs printed
}
