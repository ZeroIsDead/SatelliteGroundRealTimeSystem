use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::time::Duration;
use std::thread;
use thread_priority::*;

use crate::config::{COMMAND_MS, COMMAND_PRIORITY, SEQUENCE_NOT_CONFIRMED, COMMAND_DISPATCH_DEADLINE_MS};
use crate::state::GroundState;
use crate::buffer::BoundedBuffer;
use crate::types::*;

pub fn run_command_scheduler(
    state: Arc<GroundState>,
    uplink_buffer: Arc<BoundedBuffer>,
    log_tx: SyncSender<Log>,
) {
    set_current_thread_priority(ThreadPriority::Crossplatform(
        COMMAND_PRIORITY.try_into().unwrap()
    )).unwrap();

    while state.is_running.load(Ordering::SeqCst) {
        let now = state.uptime_ms();

        let schedule = state.command_schedule.lock().unwrap();
        for entry in schedule.iter() {
            if !entry.enabled.load(Ordering::Acquire) {
                continue;
            }

            let next = entry.next_send_time.load(Ordering::Acquire);

            if next == 0 {
                entry.next_send_time.store(now + entry.interval_ms, Ordering::Release);
                continue;
            }

            if now < next {
                continue;
            }

            if now > next {
                log_scheduling_drift(&log_tx, &entry.command, now, now - next);
            }

            if interlock_blocks(&state, &uplink_buffer, &log_tx, &entry.command) {
                log_interlock_rejection(&log_tx, &state, &entry.command, now);
                entry.next_send_time.store(now + entry.interval_ms, Ordering::Release);
                continue;
            }

            dispatch_command(&state, &uplink_buffer, &log_tx, &entry.command, entry.priority, now);
            entry.next_send_time.store(now + entry.interval_ms, Ordering::Release);
        }
        drop(schedule);

        thread::sleep(Duration::from_micros(COMMAND_MS));
    }
}

fn interlock_blocks(state: &Arc<GroundState>, uplink_buffer: &Arc<BoundedBuffer>, log_tx: &SyncSender<Log>, command: &Command) -> bool {
    if let Some(required_subsystem) = command.required_health() {
        if let Some(sub) = state.find_subsystem(required_subsystem) {
            let interlock = sub.interlock.load(Ordering::Acquire);

            if interlock {
                let packet = TelemetryPacket {
                    priority: Priority::Emergency,
                    creation_time: state.uptime_ms(),
                    payload: SatelliteMessage::Command { 
                        command: Command::ClearSubsystemFault { 
                            subsystem_id: required_subsystem 
                        }, 
                        sent_at: state.uptime_ms(), 
                    },
                    sequence_no: SEQUENCE_NOT_CONFIRMED,
                };

                if let Some(dropped) = uplink_buffer.push(packet) {
                    log_tx.try_send(Log {
                        source: LogSource::CommandScheduler,
                        event: Event {
                            task_id: dropped.payload.command_task_id(),
                            event_id: EventID::DataLoss,
                            data: EventData::None,
                            timestamp: state.uptime_ms(),
                        },
                    }).ok();
                }
            }

            return interlock;
        }
    }
    false
}

fn dispatch_command(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    command: &Command,
    priority: Priority,
    enqueued_at: u64,
) {
    let task_id = command.task_id();

    let packet = TelemetryPacket {
        priority,
        creation_time: state.uptime_ms(),
        payload: SatelliteMessage::Command {
            command: *command,
            sent_at: state.uptime_ms(),
        },
        sequence_no: SEQUENCE_NOT_CONFIRMED,
    };

    if let Some(dropped) = uplink_buffer.push(packet) {
        log_tx.try_send(Log {
            source: LogSource::CommandScheduler,
            event: Event {
                task_id: dropped.payload.command_task_id(),
                event_id: EventID::DataLoss,
                data: EventData::None,
                timestamp: state.uptime_ms(),
            },
        }).ok();
    }

    let dispatch_time = state.uptime_ms();
    let latency = dispatch_time.saturating_sub(enqueued_at);
    state.command_dispatch_latency.insert_new_metric(latency);

    if latency > COMMAND_DISPATCH_DEADLINE_MS {
        log_tx.try_send(Log {
            source: LogSource::CommandScheduler,
            event: Event {
                task_id,
                event_id: EventID::CompletionDelay,
                data: EventData::SchedulingDrift {
                    drift_ms: latency.saturating_sub(COMMAND_DISPATCH_DEADLINE_MS) as u32,
                },
                timestamp: dispatch_time,
            },
        }).ok();
    }

    log_tx.try_send(Log {
        source: LogSource::CommandScheduler,
        event: Event {
            task_id,
            event_id: EventID::TaskCompletion,
            data: EventData::Hardware {
                value: 0,
                latency_ms: latency,
                jitter_ms: state.command_dispatch_latency.last_jitter_ms.load(Ordering::Relaxed),
                sample_count: state.command_dispatch_latency
                    .number_of_samples.load(Ordering::Relaxed),
            },
            timestamp: dispatch_time,
        },
    }).ok();
}

fn log_scheduling_drift(log_tx: &SyncSender<Log>, command: &Command, now: u64, drift: u64) {
    log_tx.try_send(Log {
        source: LogSource::CommandScheduler,
        event: Event {
            task_id: command.task_id(),
            event_id: EventID::StartDelay,
            data: EventData::SchedulingDrift { drift_ms: drift as u32 },
            timestamp: now,
        },
    }).ok();
}

fn log_interlock_rejection(
    log_tx: &SyncSender<Log>,
    state: &Arc<GroundState>,
    command: &Command,
    now: u64,
) {
    if let Some(subsystem_id) = command.required_health() {
        if let Some(sub) = state.find_subsystem(subsystem_id) {
            let detected_at = sub.fault_detected_at.load(Ordering::Acquire);
            if detected_at > 0 {
                log_tx.try_send(Log {
                    source: LogSource::CommandScheduler,
                    event: Event {
                        task_id: TaskID::GlobalSystem,
                        event_id: EventID::CommandNotFound,
                        data: EventData::Subsystem { subsystem_id },
                        timestamp: now,
                    },
                }).ok();
            }
        }
    }
}