use std::{sync::atomic::Ordering};
use std::sync::Arc;
use crate::{config::{MAX_SENSORS, SENSOR_DATA_CORRUPTION, SIMULATION_PRIORITY}, state::SatelliteState, types::EventID};
use std::thread;
use std::time::Duration;
use crate::config::{MAX_SUBSYSTEM, SENSOR_FAULT_INJECTION_MS, SUBSYSTEM_FAULT_INJECTION_MS, TICK_RATE, VISIBILITY_WINDOW_CYCLE_MS, VISIBILITY_WINDOW_LIMIT_MS};
use rand::Rng;
use thread_priority::*;



pub fn run_simulation(state: Arc<SatelliteState>) {
    set_current_thread_priority(ThreadPriority::Crossplatform(SIMULATION_PRIORITY.try_into().unwrap())).unwrap();

    while state.is_running.load(Ordering::SeqCst) {
        let now = state.uptime_ms();

        for sensor in &state.sensors {
            let mut rng = rand::thread_rng();

            let val: u32 = rng.gen_range(0..101);
            let is_addition: bool = rand::random();

            if is_addition {
                sensor.value.fetch_add(val, Ordering::Relaxed);
            } else {
                sensor.value.fetch_sub(val, Ordering::Relaxed);
            }

            if !sensor.has_valid_value() {
                sensor.fault.store(EventID::DataCorruption as u16, Ordering::Release);
                sensor.fault_timestamp.store(state.uptime_ms(), Ordering::Release);
            }
        }

        if now % VISIBILITY_WINDOW_CYCLE_MS == 0 {
            state.is_visible.store(true, Ordering::Release);
        } else if now % VISIBILITY_WINDOW_CYCLE_MS == VISIBILITY_WINDOW_LIMIT_MS {
            state.is_visible.store(false, Ordering::Release);
        }

        if now % SENSOR_FAULT_INJECTION_MS == 0 {
            let sensor_index = rand::thread_rng().gen_range(0..MAX_SENSORS);

            let fault = rand::thread_rng().gen_range(EventID::StartDelay as u16..EventID::DataCorruption as u16 + 1);

            state.sensors[sensor_index].fault.store(fault, Ordering::Release);
        
            if fault == EventID::DataCorruption as u16 {
                state.sensors[sensor_index].value.store(SENSOR_DATA_CORRUPTION, Ordering::Relaxed);
            }

            state.sensors[sensor_index].fault_timestamp.store(state.uptime_ms(), Ordering::SeqCst);
        }

        if now % SUBSYSTEM_FAULT_INJECTION_MS == 0 {
            let subsystem_index = rand::thread_rng().gen_range(0..MAX_SUBSYSTEM);

            state.subsystem_health[subsystem_index].fault.store(true, Ordering::Release);
            state.subsystem_health[subsystem_index].fault_interlock.store(true, Ordering::Release);
            state.subsystem_health[subsystem_index].fault_timestamp.store(state.uptime_ms(), Ordering::Release);
        }

        state.cpu_active_ms.fetch_add(state.uptime_ms() - now, Ordering::SeqCst);

        thread::sleep(Duration::from_millis(TICK_RATE));
    }
}