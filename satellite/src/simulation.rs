use std::{sync::atomic::Ordering};
use std::sync::Arc;
use crate::{config::{MAX_SENSORS, SENSOR_DATA_CORRUPTION, SIMULATION_PRIORITY}, state::SatelliteState, types::EventID};
use std::thread;
use std::time::Duration;
use crate::config::{SENSOR_INCREMENT_MAX, MAX_SUBSYSTEM, SENSOR_FAULT_INJECTION_MS, SUBSYSTEM_FAULT_INJECTION_MS, TICK_RATE, VISIBILITY_WINDOW_CYCLE_MS, VISIBILITY_WINDOW_LIMIT_MS};
use rand::Rng;
use thread_priority::*;



pub fn run_simulation(state: Arc<SatelliteState>) {
    set_current_thread_priority(ThreadPriority::Crossplatform(SIMULATION_PRIORITY.try_into().unwrap())).unwrap();

    let mut subsystem_fault_interval = SUBSYSTEM_FAULT_INJECTION_MS;
    let mut sensor_fault_interval = SENSOR_FAULT_INJECTION_MS;

    while state.is_running.load(Ordering::SeqCst) {
        let now = state.uptime_ms();

        for sensor in &state.sensors {
            let mut rng = rand::thread_rng();

            let val: u32 = rng.gen_range(0..SENSOR_INCREMENT_MAX);
            let is_addition: bool = rand::random();


            let current = sensor.value.load(Ordering::Relaxed);

            let new_value = if is_addition {
                current.saturating_add(val).min(sensor.max_data)
            } else {
                current.saturating_sub(val).max(sensor.min_data)
            };

            sensor.value.store(new_value, Ordering::Relaxed);

            if !sensor.has_valid_value() {
                sensor.fault.store(EventID::DataCorruption as u16, Ordering::Release);
                sensor.fault_timestamp.store(state.uptime_ms(), Ordering::Release);
            }
        }

    
        state.network.is_visible.store(now % VISIBILITY_WINDOW_CYCLE_MS < VISIBILITY_WINDOW_LIMIT_MS, Ordering::Release);

        if now >= sensor_fault_interval {
            let sensor_index = rand::thread_rng().gen_range(0..MAX_SENSORS);

            let fault = rand::thread_rng().gen_range(EventID::StartDelay as u16..EventID::DataCorruption as u16 + 1);

            state.sensors[sensor_index].fault_timestamp.store(state.uptime_ms(), Ordering::SeqCst);

            state.sensors[sensor_index].fault.store(fault, Ordering::Release);
        
            if fault == EventID::DataCorruption as u16 {
                state.sensors[sensor_index].value.store(SENSOR_DATA_CORRUPTION, Ordering::Relaxed);
            }

            sensor_fault_interval = now + SENSOR_FAULT_INJECTION_MS;
        }

        if now >= subsystem_fault_interval {
            let subsystem_index = rand::thread_rng().gen_range(0..MAX_SUBSYSTEM);

            state.subsystem_health[subsystem_index].fault_timestamp.store(state.uptime_ms(), Ordering::Release);
            state.subsystem_health[subsystem_index].fault.store(true, Ordering::Release);
            state.subsystem_health[subsystem_index].fault_interlock.store(true, Ordering::Release);

            subsystem_fault_interval = now + SUBSYSTEM_FAULT_INJECTION_MS;
        }

        state.cpu_active_ms.fetch_add(state.uptime_ms() - now, Ordering::SeqCst);

        thread::sleep(Duration::from_micros(TICK_RATE));
    }
}