use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::time::Duration;
use std::thread;
use thread_priority::*;

use crate::config::{
    MONITOR_MS, MONITOR_PRIORITY, LOSS_OF_CONTACT_THRESHOLD,
    FAULT_ALERT_THRESHOLD_MS, NUMBER_OF_CORES,
};
use crate::state::GroundState;
use crate::buffer::BoundedBuffer;
use crate::types::*;

pub fn run_fault_monitor(
    state: Arc<GroundState>,
    uplink_buffer: Arc<BoundedBuffer>,
    log_tx: SyncSender<Log>,
) {
    set_current_thread_priority(ThreadPriority::Crossplatform(
        MONITOR_PRIORITY.try_into().unwrap()
    )).unwrap();

    while state.is_running.load(Ordering::SeqCst) {
        let now = state.uptime_ms();

        check_loss_of_contact(&state, &log_tx, now);
        check_subsystem_interlock_alerts(&state, &log_tx, now);
        report_buffer_fill_rate(&state, &uplink_buffer);
        report_cpu_utilization(&state, &log_tx, now);
        report_queue_performance(&state, &uplink_buffer, &log_tx, now);

        thread::sleep(Duration::from_micros(MONITOR_MS));
    }
}

fn check_loss_of_contact(state: &Arc<GroundState>, log_tx: &SyncSender<Log>, now: u64) {
    let consecutive = state.link.consecutive_missing.load(Ordering::Acquire);
    if consecutive < LOSS_OF_CONTACT_THRESHOLD {
        return;
    }
    
    log_tx.try_send(Log {
        source: LogSource::HealthMonitor,
        event: Event {
            task_id: TaskID::NetworkService,
            event_id: EventID::MissedCommunication,
            data: EventData::SchedulingDrift { drift_ms: consecutive },
            timestamp: now,
        },
    }).ok();
}

fn check_subsystem_interlock_alerts(state: &Arc<GroundState>, log_tx: &SyncSender<Log>, now: u64) {
    for subsystem in &state.subsystem_health {
        if !subsystem.interlock.load(Ordering::Acquire) {
            continue;
        }
        let detected_at = subsystem.fault_detected_at.load(Ordering::Acquire);
        if detected_at == 0 {
            continue;
        }
        let fault_duration = now.saturating_sub(detected_at);
        if fault_duration > FAULT_ALERT_THRESHOLD_MS
            && !subsystem.alert_sent.load(Ordering::Acquire)
        {
            subsystem.alert_sent.store(true, Ordering::Release);
            log_tx.try_send(Log {
                source: LogSource::HealthMonitor,
                event: Event {
                    task_id: TaskID::GlobalSystem,
                    event_id: EventID::MissionAbort,
                    data: EventData::FaultRecovery { recovery_time: fault_duration },
                    timestamp: now,
                },
            }).ok();
        }
    }
}

fn report_buffer_fill_rate(state: &Arc<GroundState>, uplink_buffer: &Arc<BoundedBuffer>) {
    let fill_rate = ((uplink_buffer.len() as f32 / uplink_buffer.capacity as f32) * 100.0) as u32;
    state.buffer_fill_rate.store(fill_rate, Ordering::Relaxed);
}

fn report_cpu_utilization(state: &Arc<GroundState>, log_tx: &SyncSender<Log>, now: u64) {
    let current_uptime = state.uptime_ms();
    let total_active = state.cpu_active_ms.fetch_add(current_uptime - now, Ordering::SeqCst) as f32;
    let total_possible = (current_uptime * NUMBER_OF_CORES).max(1) as f32;
    let cpu_active_ms = (current_uptime as f32 * (total_active / total_possible)) as u64;

    log_tx.try_send(Log {
        source: LogSource::HealthMonitor,
        event: Event {
            task_id: TaskID::GlobalSystem,
            event_id: EventID::ResourceUtilization,
            data: EventData::SystemStats {
                active_ms: cpu_active_ms,
                inactive_ms: current_uptime.saturating_sub(cpu_active_ms),
            },
            timestamp: now,
        },
    }).ok();
}

fn report_queue_performance(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    now: u64,
) {
    let metrics = &uplink_buffer.metrics;
    let sample_count = metrics.number_of_samples.load(Ordering::Relaxed);
    if sample_count == 0 {
        return;
    }

    log_tx.try_send(Log {
        source: LogSource::HealthMonitor,
        event: Event {
            task_id: TaskID::NetworkService,
            event_id: EventID::QueuePerformance,
            data: EventData::QueuePerformance {
                latency_ms: metrics.last_latency_ms.load(Ordering::Relaxed),
                average_latency_ms: metrics.get_average_latency(),
                jitter_ms: metrics.get_jitter(),
                buffer_fill_rate: state.buffer_fill_rate.load(Ordering::Relaxed),
                sample_count,
            },
            timestamp: now,
        },
    }).ok();
}