use std::time::{Duration};
use crate::state::SatelliteState;
use crate::buffer::{BoundedBuffer};
use crate::types::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender};
use std::thread;
use crate::config::{DEGRADED_TO_NORMAL_THRESHOLD, FAULT_RECOVERY_MS, MONITOR_MS, MONITOR_PRIORITY, NORMAL_TO_DEGRADED_THRESHOLD, NUMBER_OF_CORES};
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
            let limit: u64 = if state.degraded_mode.load(Ordering::Acquire) && sensor.priority != Priority::Critical {
                sensor.period * 3 * 2 // If Degraded and Non-Critical, Task Period Doubles
            } else {
                sensor.period * 3 // 3 Cycles
            };

            if now.saturating_sub(last_seen) > limit {
                let safety_alert_event = Event {
                    task_id: sensor.task_id,
                    event_id: EventID::TaskFault,
                    data: EventData::None,
                    timestamp: now,
                };
                
                if sensor.fault.load(Ordering::Acquire) != 0 {
                    let fault_timestamp = sensor.fault_timestamp.load(Ordering::Acquire);
                    let recovery_time = now - fault_timestamp;

                    let _ = log_tx.try_send(Log {
                        source: LogSource::HealthMonitor,
                        event: safety_alert_event
                    });

                    if recovery_time > FAULT_RECOVERY_MS && fault_timestamp != 0 {
                        downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                            TelemetryPacket{
                            priority: Priority::Critical,
                            creation_time: state.get_synchronized_timestamp(),
                            payload: SatelliteMessage::Telemetry {
                                    event: Event {
                                        task_id: sensor.task_id,
                                        event_id: EventID::MissionAbort,
                                        data: EventData::FaultRecovery { recovery_time: recovery_time as u32 },
                                        timestamp: now,
                                    },
                            },
                            sequence_no: 0,
                        }, 
                        &state, &log_tx, &downlink_buffer);

                        state.is_running.store(false, Ordering::SeqCst);
                    }

                    sensor.fault.store(0, Ordering::Release);
                    sensor.fault_timestamp.store(0, Ordering::Release);
                }

                downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                    TelemetryPacket{
                    priority: Priority::Critical,
                    creation_time: state.get_synchronized_timestamp(),
                    payload: SatelliteMessage::Telemetry {
                        event: safety_alert_event,
                    },
                    sequence_no: 0,
                }, 
                &state, &log_tx, &downlink_buffer);
            }
        }

        for subsystem in &state.subsystem_health {
            let fault_timestamp = subsystem.fault_timestamp.load(Ordering::Acquire);
            let recovery_time = now - fault_timestamp;

            if subsystem.fault.load(Ordering::Acquire) && 
            subsystem.fault_timestamp.load(Ordering::Acquire) != 0 {
                downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                    TelemetryPacket{
                    priority: Priority::Critical,
                    creation_time: state.get_synchronized_timestamp(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                                task_id: TaskID::GlobalSystem,
                                event_id: EventID::SubsystemFault,
                                data: EventData::SubsystemFault { subsystem_id: subsystem.id },
                                timestamp: now,
                            },
                    },
                    sequence_no: 0,
                }, 
                &state, &log_tx, &downlink_buffer);

                if recovery_time > FAULT_RECOVERY_MS && fault_timestamp != 0 {
                    downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                        TelemetryPacket{
                        priority: Priority::Critical,
                        creation_time: state.get_synchronized_timestamp(),
                        payload: SatelliteMessage::Telemetry {
                                event: Event {
                                    task_id: TaskID::GlobalSystem,
                                    event_id: EventID::MissionAbort,
                                    data: EventData::FaultRecovery { recovery_time: recovery_time as u32 },
                                    timestamp: now,
                                },
                        },
                        sequence_no: 0,
                    }, 
                    &state, &log_tx, &downlink_buffer);

                    state.is_running.store(false, Ordering::SeqCst);
                }

                subsystem.fault.store(false, Ordering::Release);
                subsystem.fault_timestamp.store(0, Ordering::Release);
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
                    priority: Priority::Critical,
                    creation_time: state.get_synchronized_timestamp(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                                task_id: TaskID::NetworkService,
                                event_id: EventID::DegradedMode,
                                data: EventData::QueuePerformance {
                                    latency_ms: downlink_buffer.metrics.last_latency_ms.load(Ordering::Relaxed),
                                    jitter_ms: downlink_buffer.metrics.jitter_ms.load(Ordering::Relaxed),
                                    buffer_fill_rate: fill_rate_percent,
                                },
                                timestamp: now,
                            }
                    },
                    sequence_no: 0,
                }, 
                &state, &log_tx, &downlink_buffer);
            }
        } else if fill_rate_percent < DEGRADED_TO_NORMAL_THRESHOLD {
            if state.degraded_mode.swap(false, Ordering::Release) {
                 downlink_buffer.push_and_log(LogSource::HealthMonitor, 
                    TelemetryPacket{
                    priority: Priority::Critical,
                    creation_time: state.get_synchronized_timestamp(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                                task_id: TaskID::NetworkService,
                                event_id: EventID::NormalMode,
                                data: EventData::QueuePerformance {
                                    latency_ms: downlink_buffer.metrics.last_latency_ms.load(Ordering::Relaxed),
                                    jitter_ms: downlink_buffer.metrics.jitter_ms.load(Ordering::Relaxed),
                                    buffer_fill_rate: fill_rate_percent,
                                },
                                timestamp: now,
                            }
                    },
                    sequence_no: 0,
                }, 
                &state, &log_tx, &downlink_buffer);
            }
        }

        state.cpu_active_ms.fetch_add(state.uptime_ms() - now, Ordering::SeqCst);

        let current_uptime = state.uptime_ms();
        let total_cpu_active_ms = state.cpu_active_ms.load(Ordering::SeqCst) as f32;
        let total_possible_uptime: f32 = (current_uptime * NUMBER_OF_CORES).max(1) as f32; // If Below 1 then set to 1

        let cpu_active_ms = (current_uptime as f32 * (total_cpu_active_ms / total_possible_uptime)) as u64;

        downlink_buffer.push_and_log(LogSource::HealthMonitor, 
            TelemetryPacket{
            priority: Priority::Critical,
            creation_time: state.get_synchronized_timestamp(),
            payload: SatelliteMessage::Telemetry {
                    event: Event {
                        task_id: TaskID::GlobalSystem,
                        event_id: EventID::ResourceUtilization,
                        data: EventData::SystemStats {
                            active_ms: cpu_active_ms,
                            inactive_ms: current_uptime - cpu_active_ms,
                        },
                        timestamp: now,
                    }
            },
            sequence_no: 0,
        }, 
        &state, &log_tx, &downlink_buffer);

        thread::sleep(Duration::from_millis(MONITOR_MS));
    }
}
