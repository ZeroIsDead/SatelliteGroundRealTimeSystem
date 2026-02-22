use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::time::{Duration};
use std::sync::atomic::Ordering;

use crate::config::{FAULT_RECOVERY_MS, SENSOR_DELAY_MS, SENSOR_FAULT_MS};
use crate::types::{Event, EventData, EventID, Log, LogSource, Priority, SatelliteMessage, TelemetryPacket};
use crate::state::SatelliteState;
use crate::buffer::BoundedBuffer;
use std::thread;
use thread_priority::*;

pub fn run_sensor_task(
    state: Arc<SatelliteState>, 
    sensor_index: usize,
    downlink_buffer: Arc<BoundedBuffer>, 
    log_tx: SyncSender<Log>
) {
    let sensor = &state.sensors[sensor_index];
    set_current_thread_priority(ThreadPriority::Crossplatform((sensor.priority as u8).try_into().unwrap())).unwrap();

    let interval = sensor.period;
    let mut next_wake_time = state.uptime_ms() + interval;

    while state.is_running.load(Ordering::Relaxed) {
        
        let fault_event = sensor.fault.load(Ordering::Acquire);

        if fault_event == EventID::TaskFault as u16 {
            thread::sleep(Duration::from_millis(SENSOR_FAULT_MS));
            continue;
        }

        if fault_event == EventID::StartDelay as u16 {
            thread::sleep(Duration::from_millis(SENSOR_DELAY_MS));
        }

        let now = state.uptime_ms();
        if next_wake_time < now {
            downlink_buffer.push_and_log(LogSource::Sensor, 
                TelemetryPacket{
                priority: Priority::Critical,
                creation_time: state.get_synchronized_timestamp(),
                payload: SatelliteMessage::Telemetry {
                        event: Event {
                            task_id: sensor.task_id,
                            event_id: EventID::StartDelay,
                            data: EventData::SchedulingDrift { drift_ms: (now - next_wake_time) as u32},
                            timestamp: state.uptime_ms(),
                        },
                },
                sequence_no: 0,
            }, 
            &state, &log_tx, &downlink_buffer);

            next_wake_time = now;
        }


        let current_value = sensor.value.load(Ordering::Relaxed);

        if sensor.min_data > current_value || current_value > sensor.max_data  {
            let now = state.uptime_ms();

            let recovery_time = now - sensor.fault_timestamp.load(Ordering::Relaxed);

            downlink_buffer.push_and_log(LogSource::Sensor, 
                TelemetryPacket{
                priority: Priority::Critical,
                creation_time: state.get_synchronized_timestamp(),
                payload: SatelliteMessage::Telemetry {
                        event: Event {
                            task_id: sensor.task_id,
                            event_id: EventID::DataCorruption,
                            data: EventData::Hardware { value: current_value },
                            timestamp: now,
                        },
                },
                sequence_no: 0,
            }, 
            &state, &log_tx, &downlink_buffer);

            if recovery_time > FAULT_RECOVERY_MS {
                downlink_buffer.push_and_log(LogSource::Sensor, 
                    TelemetryPacket{
                    priority: Priority::Critical,
                    creation_time: state.get_synchronized_timestamp(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                                task_id: sensor.task_id,
                                event_id: EventID::MissionAbort,
                                data: EventData::None,
                                timestamp: now,
                            },
                    },
                    sequence_no: 0,
                }, 
                &state, &log_tx, &downlink_buffer);

                state.is_running.swap(false, Ordering::Relaxed);
            }

            let reset_value = (sensor.min_data + sensor.max_data) / 2;

            sensor.value.store(reset_value, Ordering::Relaxed);
        }

        if fault_event == EventID::CompletionDelay as u16 {
            thread::sleep(Duration::from_millis(SENSOR_DELAY_MS));
        }
        
        sensor.heartbeat.store(state.uptime_ms(), Ordering::Release);

        let internal_msg = TelemetryPacket {
            priority: sensor.priority,
            creation_time: state.get_synchronized_timestamp(),
            payload: SatelliteMessage::Telemetry {
                event: Event {
                    task_id: sensor.task_id,
                    event_id: EventID::TaskCompletion,
                    data: EventData::Hardware { value: current_value },
                    timestamp: state.uptime_ms(),
                },
            },
            sequence_no: 0,
        };

        downlink_buffer.push_and_log(LogSource::Sensor, 
            internal_msg, &state, &log_tx, &downlink_buffer);

        if state.degraded_mode.load(Ordering::Relaxed) && sensor.priority != Priority::Critical {
            next_wake_time += interval; 
        }

        let now = state.uptime_ms();
        if next_wake_time > now {
            thread::sleep(Duration::from_millis(next_wake_time - now));
        } else {
            downlink_buffer.push_and_log(LogSource::Sensor, 
                TelemetryPacket{
                priority: Priority::Critical,
                creation_time: state.get_synchronized_timestamp(),
                payload: SatelliteMessage::Telemetry {
                        event: Event {
                            task_id: sensor.task_id,
                            event_id: EventID::CompletionDelay,
                            data: EventData::SchedulingDrift { drift_ms: (now - next_wake_time) as u32},
                            timestamp: state.uptime_ms(),
                        },
                },
                sequence_no: 0,
            }, 
            &state, &log_tx, &downlink_buffer);

            next_wake_time = now;
        }

        state.cpu_active_ms.fetch_add(state.uptime_ms() - now, Ordering::Relaxed);
        
        next_wake_time += interval;
    }
}