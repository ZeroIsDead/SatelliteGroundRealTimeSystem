use std::time::{Duration};
use crate::state::SatelliteState;
use crate::buffer::{BoundedBuffer};
use crate::types::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender};
use std::thread;
use crate::config::{DEGRADED_SKIPPED_SENSOR_CYCLES, DEGRADED_TO_NORMAL_THRESHOLD, FAULT_RECOVERY_MS, MONITOR_MS, MONITOR_PRIORITY, NETWORK_MS, NORMAL_TO_DEGRADED_THRESHOLD, NUMBER_OF_CORES, SENSOR_FAULT_NOT_CONFIRMED, SEQUENCE_NOT_CONFIRMED, TIMESTAMP_NOT_CONFIRMED, VISIBILITY_WINDOW_CYCLE_MS};
use thread_priority::*;

pub fn run_health_monitor(
    state: Arc<SatelliteState>, 
    downlink_buffer: Arc<BoundedBuffer>,
    log_tx: SyncSender<Log>
) {
    set_current_thread_priority(ThreadPriority::Crossplatform(MONITOR_PRIORITY.try_into().unwrap())).unwrap();

    while state.is_running.load(Ordering::SeqCst) {
        let now = state.uptime_ms();

        for sensor in &state.sensors {
            let last_seen = sensor.heartbeat.load(Ordering::Acquire);
            
            // "3 consecutive missed cycles" limit
            let limit: u64 = if state.degraded_mode.load(Ordering::Acquire) {
                sensor.period * 3 * DEGRADED_SKIPPED_SENSOR_CYCLES // If Degraded and Non-Critical, Task Period Multipies by Constant
            } else {
                sensor.period * 3 // 3 Cycles
            };

            if now.saturating_sub(last_seen) > limit {
                let fault_timestamp = sensor.fault_timestamp.load(Ordering::Acquire);
                let recovery_time = now - fault_timestamp;

                let safety_alert_event = Event {
                    task_id: sensor.task_id,
                    event_id: EventID::TaskFault,
                    data: EventData::FaultRecovery { recovery_time },
                    timestamp: now,
                };

                downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                    TelemetryPacket{
                    priority: Priority::Critical,
                    creation_time: state.uptime_ms(),
                    payload: SatelliteMessage::Telemetry {
                        event: safety_alert_event,
                    },
                    sequence_no: SEQUENCE_NOT_CONFIRMED,
                }, 
                &state, &log_tx, &downlink_buffer);

                if recovery_time > FAULT_RECOVERY_MS && fault_timestamp != TIMESTAMP_NOT_CONFIRMED {
                    transmit_mission_abort_and_shutdown(&state, &downlink_buffer, &log_tx, recovery_time, now);
                }

                // RESET FAULTS
                sensor.fault.store(SENSOR_FAULT_NOT_CONFIRMED, Ordering::Release);
                sensor.fault_timestamp.store(TIMESTAMP_NOT_CONFIRMED, Ordering::Release);
            }
        }

        for subsystem in &state.subsystem_health {
            let fault_timestamp = subsystem.fault_timestamp.load(Ordering::Acquire);

            if subsystem.fault.load(Ordering::Acquire) && 
              fault_timestamp != TIMESTAMP_NOT_CONFIRMED {
                let recovery_time = now.saturating_sub(fault_timestamp);
                
                if !subsystem.fault_reported.swap(true, Ordering::Relaxed) {
                    downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                        TelemetryPacket{
                        priority: Priority::Emergency,
                        creation_time: state.uptime_ms(),
                        payload: SatelliteMessage::Telemetry {
                                event: Event {
                                    task_id: TaskID::GlobalSystem,
                                    event_id: EventID::SubsystemFault,
                                    data: EventData::Subsystem { subsystem_id: subsystem.id },
                                    timestamp: now,
                                },
                        },
                        sequence_no: SEQUENCE_NOT_CONFIRMED,
                    }, 
                    &state, &log_tx, &downlink_buffer);
                }

                if recovery_time > FAULT_RECOVERY_MS && fault_timestamp != TIMESTAMP_NOT_CONFIRMED {
                    transmit_mission_abort_and_shutdown(&state, &downlink_buffer, &log_tx, recovery_time, now);
                }
            }
        }

        let current_len = downlink_buffer.len();
        let capacity = downlink_buffer.capacity;
        
        let fill_rate_percent = ((current_len as f32 / capacity as f32) * 100.0) as u32;
        
        state.buffer_fill_rate.store(fill_rate_percent, Ordering::Relaxed);

        if fill_rate_percent >= NORMAL_TO_DEGRADED_THRESHOLD {
            if !state.degraded_mode.swap(true, Ordering::Release) {
                 downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                    TelemetryPacket{
                    priority: Priority::Normal,
                    creation_time: state.uptime_ms(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                                task_id: TaskID::DownlinkNetworkService,
                                event_id: EventID::DegradedMode,
                                data: EventData::QueuePerformance {
                                    latency_ms: downlink_buffer.metrics.last_latency_ms.load(Ordering::Relaxed),
                                    jitter_ms: downlink_buffer.metrics.last_jitter_ms.load(Ordering::Relaxed),
                                    buffer_fill_rate: fill_rate_percent,
                                    sample_count: downlink_buffer.metrics.number_of_samples.load(Ordering::Relaxed)
                                },
                                timestamp: now,
                            }
                    },
                    sequence_no: SEQUENCE_NOT_CONFIRMED,
                }, 
                &state, &log_tx, &downlink_buffer);
            }
        } else if fill_rate_percent < DEGRADED_TO_NORMAL_THRESHOLD {
            if state.degraded_mode.swap(false, Ordering::Release) {
                 downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                    TelemetryPacket{
                    priority: Priority::Normal,
                    creation_time: state.uptime_ms(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                                task_id: TaskID::DownlinkNetworkService,
                                event_id: EventID::NormalMode,
                                data: EventData::QueuePerformance {
                                    latency_ms: downlink_buffer.metrics.last_latency_ms.load(Ordering::Relaxed),
                                    jitter_ms: downlink_buffer.metrics.last_jitter_ms.load(Ordering::Relaxed),
                                    buffer_fill_rate: fill_rate_percent,
                                    sample_count: downlink_buffer.metrics.number_of_samples.load(Ordering::Relaxed)
                                },
                                timestamp: now,
                            }
                    },
                    sequence_no: SEQUENCE_NOT_CONFIRMED,
                }, 
                &state, &log_tx, &downlink_buffer);
            }
        }

        let current_uptime = state.uptime_ms();
        state.cpu_active_ms.fetch_add(current_uptime - now, Ordering::SeqCst);
        let total_cpu_active_ms = state.cpu_active_ms.load(Ordering::SeqCst) as f32;
        let total_possible_uptime: f32 = (current_uptime * NUMBER_OF_CORES).max(1) as f32; // If Below 1 then set to 1

        let cpu_active_ms = (current_uptime as f32 * (total_cpu_active_ms / total_possible_uptime)) as u64;

        let _ = log_tx.try_send(Log {
            source: LogSource::HealthMonitor, 
            event: Event {
                task_id: TaskID::GlobalSystem,
                event_id: EventID::ResourceUtilization,
                data: EventData::SystemStats {
                    active_ms: cpu_active_ms,
                    inactive_ms: current_uptime - cpu_active_ms,
                },
                timestamp: state.uptime_ms(),
            }
        });

        thread::sleep(Duration::from_micros(MONITOR_MS));
    }
}

pub fn transmit_mission_abort_and_shutdown(
    state: &Arc<SatelliteState>,
    downlink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    recovery_time: u64,
    now: u64,
) {
    downlink_buffer.clear(); // Clear all Queued Messages

    downlink_buffer.push_and_log(LogSource::HealthMonitor, // Mission Abort
        TelemetryPacket{
        priority: Priority::Emergency,
        creation_time: state.uptime_ms(),
        payload: SatelliteMessage::Telemetry {
                event: Event {
                    task_id: TaskID::GlobalSystem,
                    event_id: EventID::MissionAbort,
                    data: EventData::FaultRecovery { recovery_time: recovery_time },
                    timestamp: now,
                },
        },
        sequence_no: SEQUENCE_NOT_CONFIRMED,
    }, 
    &state, &log_tx, &downlink_buffer);

    let wait_start = state.uptime_ms();
    let timeout = VISIBILITY_WINDOW_CYCLE_MS * 2;

    while downlink_buffer.len() > 0 {
        downlink_buffer.push(TelemetryPacket{ // Sends more of MissionAbort if the previous one was lost
            priority: Priority::Emergency,
            creation_time: state.uptime_ms(),
            payload: SatelliteMessage::Telemetry {
                    event: Event {
                        task_id: TaskID::GlobalSystem,
                        event_id: EventID::MissionAbort,
                        data: EventData::FaultRecovery { recovery_time: recovery_time },
                        timestamp: now,
                    },
            },
            sequence_no: SEQUENCE_NOT_CONFIRMED,
        });

        if state.uptime_ms() - wait_start > timeout {
            break;
        }
        thread::sleep(Duration::from_micros(NETWORK_MS));
    }

    state.is_running.store(false, Ordering::SeqCst);
}
