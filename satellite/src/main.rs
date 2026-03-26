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
use crate::monitor::{run_health_monitor, transmit_mission_abort_and_shutdown};
use crate::command::run_command_executor;
use crate::config::{DATA_BUFFER_CAPACITY, LOG_BUFFER_CAPACITY, MAIN_MS, MAX_SENSORS};

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

    for i in 0..MAX_SENSORS {
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
    let c_downlink_buffer = Arc::clone(&downlink_buffer);
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

    log_tx.try_send(Log {
        source: LogSource::Main,
        event: Event {
            task_id: TaskID::GlobalSystem,
            event_id: EventID::Startup,
            data: EventData::None,
            timestamp: state.uptime_ms(),
        }
    }).ok();

    state.cpu_active_ms.fetch_add(state.uptime_ms() - now, Ordering::SeqCst);

    let h_state = Arc::clone(&state);
    ctrlc::set_handler(move || {
        println!("\nCtrl+C detected! Shutting down...");
        h_state.is_shutdown.store(true, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    while state.is_running.load(Ordering::SeqCst) && !state.is_shutdown.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_micros(MAIN_MS));
    }

    log_tx.try_send(Log {
        source: LogSource::Main,
        event: Event {
            task_id: TaskID::GlobalSystem,
            event_id: EventID::Shutdown,
            data: EventData::None,
            timestamp: state.uptime_ms(),
        }
    }).ok();

    transmit_mission_abort_and_shutdown(&state, &downlink_buffer, &log_tx, 0, state.uptime_ms());


    display_summary(&state, &downlink_buffer, &uplink_buffer);

    drop(log_tx); // Drop Sender so the Receiver Know There is No More Logs

    logger_handle.join().unwrap(); // Wait until All Logs printed
}

pub fn display_summary(state: &SatelliteState, downlink_buffer: &BoundedBuffer, uplink_buffer: &BoundedBuffer) {
    println!("--------------------------------------SUMMARY--------------------------------------");

    for sensor in &state.sensors {
        let sensor_name = match sensor.task_id {
            TaskID::ThermalSensor => "Thermal Sensor",
            TaskID::PitchAndYawSensor => "Pitch & Yaw Sensor",
            TaskID::MoistureSensor => "Moisture Sensor",
            _ => "",
        };

        println!("{} METRICS: [{:?}]\n", sensor_name, sensor.metrics);
    }

    println!();

    println!("GENERAL METRICS: [ACTIVE_MS: {}, CPU_UTILIZATION: {:.2}%, DEGRADED: {}]", 
                state.cpu_active_ms.load(Ordering::Relaxed), 
                state.cpu_active_ms.load(Ordering::Relaxed) as f64 / state.uptime_ms() as f64 * 100.0, 
                state.degraded_mode.load(Ordering::Relaxed));

    println!();

    println!("NETWORK METRICS: [{:?}]", state.network.metrics);

    println!();

    println!("UPLINK BUFFER METRICS: [{:?}]", uplink_buffer.metrics);
    println!("UPLINK BUFFER PACKET DROPPED METRICS: [TOTAL: {}, STALE: {}, EMERGENCY: {}, CRITICAL: {}, NORMAL: {}, LOW: {}]", 
                uplink_buffer.packet_dropped.load(Ordering::Relaxed), 
                uplink_buffer.stale_packet_dropped.load(Ordering::Relaxed), 
                uplink_buffer.emergency_packet_dropped.load(Ordering::Relaxed), 
                uplink_buffer.critical_packet_dropped.load(Ordering::Relaxed), 
                uplink_buffer.normal_packet_dropped.load(Ordering::Relaxed), 
                uplink_buffer.low_packet_dropped.load(Ordering::Relaxed)
            );

    println!();

    println!("DOWNLINK BUFFER METRICS: [{:?}, Fill Rate: {}, Packets Sent: {}]", downlink_buffer.metrics, state.buffer_fill_rate.load(Ordering::Relaxed), state.network.packet_sequence_no.load(Ordering::Relaxed));
    println!("DOWNLINK BUFFER PACKET DROPPED METRICS: [TOTAL: {}, STALE: {}, EMERGENCY: {}, CRITICAL: {}, NORMAL: {}, LOW: {}]", 
                downlink_buffer.packet_dropped.load(Ordering::Relaxed), 
                downlink_buffer.stale_packet_dropped.load(Ordering::Relaxed),
                downlink_buffer.emergency_packet_dropped.load(Ordering::Relaxed), 
                downlink_buffer.critical_packet_dropped.load(Ordering::Relaxed), 
                downlink_buffer.normal_packet_dropped.load(Ordering::Relaxed), 
                downlink_buffer.low_packet_dropped.load(Ordering::Relaxed)
            );

}