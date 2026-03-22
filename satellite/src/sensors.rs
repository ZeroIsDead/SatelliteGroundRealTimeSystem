use std::sync::Arc;
use std::sync::mpsc::SyncSender;
use std::time::{Duration};
use std::sync::atomic::Ordering;

use crate::config::{FAULT_RECOVERY_MS, SEQUENCE_NOT_CONFIRMED, SENSOR_DELAY_MS, SENSOR_FAULT_MS, SENSOR_FAULT_NOT_CONFIRMED, TIMESTAMP_NOT_CONFIRMED};
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

    while state.is_running.load(Ordering::SeqCst) {
        
        let fault_event = sensor.fault.load(Ordering::Acquire);

        if fault_event == EventID::TaskFault as u16 {
            thread::sleep(Duration::from_micros(SENSOR_FAULT_MS));
            continue;
        }

        if fault_event == EventID::StartDelay as u16 {
            thread::sleep(Duration::from_micros(SENSOR_DELAY_MS));
        }

        let task_start = state.uptime_ms();
        if next_wake_time < task_start {
            downlink_buffer.push_and_log(LogSource::Sensor, 
                TelemetryPacket{
                priority: Priority::Normal,
                creation_time: state.get_synchronized_timestamp(),
                payload: SatelliteMessage::Telemetry {
                        event: Event {
                            task_id: sensor.task_id,
                            event_id: EventID::StartDelay,
                            data: EventData::SchedulingDrift { drift_ms: (task_start - next_wake_time) as u32},
                            timestamp: state.uptime_ms(),
                        },
                },
                sequence_no: SEQUENCE_NOT_CONFIRMED,
            }, 
            &state, &log_tx, &downlink_buffer);

            next_wake_time = task_start;
        }


        let current_value = sensor.value.load(Ordering::Relaxed);

        if !sensor.has_valid_value()  {
            let fault_recovery_timestamp = state.uptime_ms();

            let fault_timestamp = sensor.fault_timestamp.load(Ordering::Acquire);

            let recovery_time = fault_recovery_timestamp - fault_timestamp;

            downlink_buffer.push_and_log(LogSource::Sensor, 
                TelemetryPacket{
                priority: Priority::Critical,
                creation_time: state.get_synchronized_timestamp(),
                payload: SatelliteMessage::Telemetry {
                        event: Event {
                            task_id: sensor.task_id,
                            event_id: EventID::DataCorruption,
                            data: EventData::CorruptedHardware { value: current_value, recovery_time: recovery_time },
                            timestamp: fault_recovery_timestamp,
                        },
                },
                sequence_no: SEQUENCE_NOT_CONFIRMED,
            }, 
            &state, &log_tx, &downlink_buffer);

            if recovery_time > FAULT_RECOVERY_MS && fault_timestamp != TIMESTAMP_NOT_CONFIRMED {
                downlink_buffer.push_and_log(LogSource::Sensor, 
                    TelemetryPacket{
                    priority: Priority::Emergency,
                    creation_time: state.get_synchronized_timestamp(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                                task_id: sensor.task_id,
                                event_id: EventID::MissionAbort,
                                data: EventData::FaultRecovery { recovery_time: recovery_time },
                                timestamp: fault_recovery_timestamp,
                            },
                    },
                    sequence_no: SEQUENCE_NOT_CONFIRMED,
                }, 
                &state, &log_tx, &downlink_buffer);

                state.is_running.store(false, Ordering::SeqCst);
            }

            let reset_value = (sensor.min_data + sensor.max_data) / 2;

            sensor.value.store(reset_value, Ordering::Relaxed);
            sensor.fault.store(SENSOR_FAULT_NOT_CONFIRMED, Ordering::Release);
            sensor.fault_timestamp.store(TIMESTAMP_NOT_CONFIRMED, Ordering::Release);
        }

        if fault_event == EventID::CompletionDelay as u16 {
            thread::sleep(Duration::from_micros(SENSOR_DELAY_MS));
        }
        
        let completion_time = state.uptime_ms();
        sensor.heartbeat.store(completion_time, Ordering::Release);
        let latency = completion_time - task_start;
        sensor.metrics.insert_new_metric(latency);
        let jitter = sensor.metrics.get_jitter();

        let internal_msg = TelemetryPacket {
            priority: sensor.data_priority,
            creation_time: state.get_synchronized_timestamp(),
            payload: SatelliteMessage::Telemetry {
                event: Event {
                    task_id: sensor.task_id,
                    event_id: EventID::TaskCompletion,
                    data: EventData::Hardware { 
                        value: current_value, 
                        latency_ms: latency, 
                        average_latency_ms: sensor.metrics.get_average_latency(), 
                        jitter_ms: jitter,
                        sample_count: sensor.metrics.number_of_samples.load(Ordering::Relaxed)
                    },
                    timestamp: state.uptime_ms(),
                },
            },
            sequence_no: SEQUENCE_NOT_CONFIRMED,
        };

        downlink_buffer.push_and_log(LogSource::Sensor, 
            internal_msg, &state, &log_tx, &downlink_buffer);

        if sensor.priority == Priority::Critical && jitter >= 1000 { // Critical Sensors Jitter < 1ms
            downlink_buffer.push_and_log(LogSource::Sensor, 
                TelemetryPacket{
                priority: Priority::Normal,
                creation_time: state.get_synchronized_timestamp(),
                payload: SatelliteMessage::Telemetry {
                        event: Event {
                            task_id: sensor.task_id,
                            event_id: EventID::CompletionDelay,
                            data: EventData::Hardware { 
                                value: current_value, 
                                latency_ms: latency, 
                                average_latency_ms: sensor.metrics.get_average_latency(), 
                                jitter_ms: jitter,
                                sample_count: sensor.metrics.number_of_samples.load(Ordering::Relaxed)
                            },
                            timestamp: state.uptime_ms(),
                        },
                },
                sequence_no: SEQUENCE_NOT_CONFIRMED,
            }, 
            &state, &log_tx, &downlink_buffer);
        }

        if sensor.priority != Priority::Critical && state.degraded_mode.load(Ordering::Acquire) {
            next_wake_time += interval; 
        }

        state.cpu_active_ms.fetch_add(state.uptime_ms() - task_start, Ordering::SeqCst);

        let now = state.uptime_ms();
        if next_wake_time > now {
            thread::sleep(Duration::from_micros(next_wake_time - now));
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
                sequence_no: SEQUENCE_NOT_CONFIRMED,
            }, 
            &state, &log_tx, &downlink_buffer);

            next_wake_time = now;
        }

        next_wake_time += interval;
    }
}