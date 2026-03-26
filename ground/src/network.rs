use std::sync::mpsc::SyncSender;
use std::time::Duration;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use bincode;
use thread_priority::*;

use crate::config::{
    NETWORK_PORT, NETWORK_PRIORITY, NETWORK_READ_TIMEOUT, NETWORK_WRITE_TIMEOUT,
    NETWORK_MS, VISIBILITY_WINDOW_LIMIT_MS, SEQUENCE_NOT_CONFIRMED,
    PACKET_HISTORY_BUFFER_CAPACITY, SYNC_INTERVAL_WINDOWS, SYNC_CALIBRATED_INTERVAL_WINDOWS,
    DECODE_DEADLINE_MS, COMMAND_DISPATCH_DEADLINE_MS,
};
use crate::state::GroundState;
use crate::buffer::BoundedBuffer;
use crate::types::*;

type PacketHistory = [Option<TelemetryPacket>; PACKET_HISTORY_BUFFER_CAPACITY];

pub fn run_network_thread(
    state: Arc<GroundState>,
    uplink_buffer: Arc<BoundedBuffer>,
    log_tx: SyncSender<Log>,
) {
    set_current_thread_priority(ThreadPriority::Crossplatform(
        NETWORK_PRIORITY.try_into().unwrap()
    )).unwrap();

    let listener = TcpListener::bind(NETWORK_PORT)
        .expect("GCS failed to bind TCP listener");

    listener.set_nonblocking(false).unwrap();

    let mut history: PacketHistory = [None; PACKET_HISTORY_BUFFER_CAPACITY];
    let mut history_idx: usize = 0;

    while state.is_running.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                configure_stream(&mut stream);
                let connect_time = state.uptime_ms();

                log_tx.send(Log {
                    source: LogSource::Network,
                    event: Event {
                        task_id: TaskID::NetworkService,
                        event_id: EventID::ConnectionStart,
                        data: EventData::None,
                        timestamp: connect_time,
                    },
                }).ok();

                handle_visibility_window(
                    &state, &uplink_buffer, &log_tx,
                    &mut stream, &mut history, &mut history_idx,
                );

                let disconnect_time = state.uptime_ms();

                log_tx.send(Log {
                    source: LogSource::Network,
                    event: Event {
                        task_id: TaskID::NetworkService,
                        event_id: EventID::ConnectionEnd,
                        data: EventData::None,
                        timestamp: disconnect_time,
                    },
                }).ok();


                drain_stale_uplink(&uplink_buffer, &log_tx, &state);
            }
            Err(e) => {
                eprintln!("[GCS] accept() error: {}", e);
                thread::sleep(Duration::from_micros(NETWORK_MS));
            }
        }
    }
}

fn configure_stream(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_micros(NETWORK_READ_TIMEOUT)));
    let _ = stream.set_write_timeout(Some(Duration::from_micros(NETWORK_WRITE_TIMEOUT)));
    let _ = stream.set_nodelay(true);
}

fn drain_stale_uplink(
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    state: &Arc<GroundState>,
) {
    let mut drained = 0u32;
    while let Some(stale) = uplink_buffer.pop() {
        drained += 1;
        log_tx.try_send(Log {
            source: LogSource::Network,
            event: Event {
                task_id: stale.payload.command_task_id(),
                event_id: EventID::DataLoss,
                data: EventData::None,
                timestamp: state.uptime_ms(),
            },
        }).ok();
    }
    if drained > 0 {
        log_tx.try_send(Log {
            source: LogSource::Network,
            event: Event {
                task_id: TaskID::NetworkService,
                event_id: EventID::DataLoss,
                data: EventData::PacketDrain { count: drained },
                timestamp: state.uptime_ms(),
            },
        }).ok();
    }
}

fn handle_visibility_window(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    stream: &mut TcpStream,
    history: &mut PacketHistory,
    history_idx: &mut usize,
) {
    let pass_start = state.uptime_ms();
    state.link.consecutive_missing.store(0, Ordering::Release);

    maybe_queue_sync_request(state, uplink_buffer, log_tx);

    while state.uptime_ms() - pass_start < VISIBILITY_WINDOW_LIMIT_MS
        && state.is_running.load(Ordering::SeqCst)
    {
        send_uplink(state, uplink_buffer, log_tx, stream, history, history_idx);
        receive_downlink(state, uplink_buffer, log_tx, stream);
    }

    state.cpu_active_ms.fetch_add(state.uptime_ms() - pass_start, Ordering::SeqCst);
}

fn maybe_queue_sync_request(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
) {
    let windows = state.link.windows_since_sync.fetch_add(1, Ordering::Relaxed);
    let sync_interval = if state.clock_sync.is_calibrated.load(Ordering::Acquire) {
        SYNC_CALIBRATED_INTERVAL_WINDOWS
    } else {
        SYNC_INTERVAL_WINDOWS
    };

    if windows % sync_interval != 0 {
        return;
    }

    if let Some(dropped) = uplink_buffer.push(TelemetryPacket {
        priority: Priority::Emergency,
        creation_time: 0,
        payload: SatelliteMessage::SyncRequest,
        sequence_no: SEQUENCE_NOT_CONFIRMED,
    }) {
        log_uplink_drop(log_tx, &dropped, state.uptime_ms());
    }

    log_tx.try_send(Log {
        source: LogSource::Network,
        event: Event {
            task_id: TaskID::NetworkService,
            event_id: EventID::SyncStart,
            data: EventData::None,
            timestamp: state.uptime_ms(),
        },
    }).ok();
}

fn send_uplink(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    stream: &mut TcpStream,
    history: &mut PacketHistory,
    history_idx: &mut usize,
) {
    let mut packet = match uplink_buffer.pop() {
        Some(p) => p,
        None => return,
    };

    let wire_time = state.uptime_ms();
    packet.creation_time = wire_time;

    let is_sync = packet.payload == SatelliteMessage::SyncRequest;
    if is_sync {
        
        state.clock_sync.last_sent_at.store(wire_time, Ordering::Release);
    }

    if !is_sync && packet.creation_time > 0 {
        let latency = state.uptime_ms().saturating_sub(packet.creation_time);
        state.command_dispatch_latency.insert_new_metric(latency);

        if latency > COMMAND_DISPATCH_DEADLINE_MS {
            log_tx.try_send(Log {
                source: LogSource::Network,
                event: Event {
                    task_id: TaskID::NetworkService,
                    event_id: EventID::CompletionDelay,
                    data: EventData::SchedulingDrift {
                        drift_ms: latency.saturating_sub(COMMAND_DISPATCH_DEADLINE_MS) as u32,
                    },
                    timestamp: state.uptime_ms(),
                },
            }).ok();
        }
    }

    history[*history_idx] = Some(packet);
    *history_idx = (*history_idx + 1) % PACKET_HISTORY_BUFFER_CAPACITY;

    if let Ok(bytes) = bincode::serialize(&packet) {
        let length = bytes.len() as u16;
        if stream.write_all(&length.to_be_bytes()).is_err() { return; }
        let _ = stream.write_all(&bytes);
    }
}

fn receive_downlink(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    stream: &mut TcpStream,
) {
    let mut length_buf = [0u8; 2];
    if stream.read_exact(&mut length_buf).is_err() { return; }

    let length = u16::from_be_bytes(length_buf) as usize;
    let mut payload_buf = vec![0u8; length];
    if stream.read_exact(&mut payload_buf).is_err() { return; }

    let receive_time = state.uptime_ms();

    let packet = match bincode::deserialize::<TelemetryPacket>(&payload_buf) {
        Ok(packet) => {
            packet
        },
        Err(_) => {
            return;
        },
    };

    let decode_latency = state.uptime_ms().saturating_sub(receive_time);

    if state.clock_sync.samples.load(Ordering::Relaxed) > 0 {
        let network_latency = receive_time.saturating_sub(packet.creation_time);

        state.telemetry_reception_latency.insert_new_metric(network_latency);
        
        log_tx.try_send(Log {
            source: LogSource::Network,
            event: Event {
                task_id: TaskID::NetworkService,
                event_id: EventID::NetworkPerformance,
                data: EventData::NetworkPerformance {
                    priority: packet.priority,
                    latency_ms: state.telemetry_reception_latency.last_latency_ms.load(Ordering::Relaxed),
                    jitter_ms: state.telemetry_reception_latency.last_jitter_ms.load(Ordering::Relaxed),
                    sample_count: state.telemetry_reception_latency.number_of_samples.load(Ordering::Relaxed),
                },
                timestamp: receive_time,
            }
        }).ok();
    }

    if decode_latency > DECODE_DEADLINE_MS {
        log_tx.try_send(Log {
            source: LogSource::Network,
            event: Event {
                task_id: TaskID::NetworkService,
                event_id: EventID::CompletionDelay,
                data: EventData::SchedulingDrift {
                    drift_ms: decode_latency.saturating_sub(DECODE_DEADLINE_MS) as u32,
                },
                timestamp: receive_time,
            },
        }).ok();
    }

    log_tx.try_send(Log {
        source: LogSource::Network,
        event: Event {
            task_id: TaskID::NetworkService,
            event_id: EventID::QueuePerformance,
            data: EventData::QueuePerformance {
                latency_ms: decode_latency,
                jitter_ms: state.telemetry_reception_latency.last_jitter_ms.load(Ordering::Relaxed),
                buffer_fill_rate: state.buffer_fill_rate.load(Ordering::Relaxed),
                sample_count: state.telemetry_reception_latency
                    .number_of_samples.load(Ordering::Relaxed),
            },
            timestamp: receive_time,
        },
    }).ok();

    check_sequence_gaps(state, uplink_buffer, log_tx, &packet);
    route_packet(state, uplink_buffer, log_tx, packet, receive_time);
}

fn route_packet(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    packet: TelemetryPacket,
    receive_time: u64,
) {
    match packet.payload {
        SatelliteMessage::SyncResponse { ground_sent, satellite_receive } => {
            handle_sync_response(state, uplink_buffer, log_tx, ground_sent, satellite_receive, receive_time);
        }
        SatelliteMessage::Telemetry { event } => {
            handle_telemetry(state, log_tx, uplink_buffer, event, receive_time);
        }
        _ => {}
    }
}

fn handle_sync_response(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    ground_sent: u64,
    satellite_receive: u64,
    receive_time: u64,
) {
    let actual_sent = state.clock_sync.last_sent_at.load(Ordering::Acquire);
    let g_sent = if ground_sent > 0 { ground_sent } else { actual_sent };

    let rtt = receive_time.saturating_sub(g_sent);
    let one_way = rtt / 2;

    let offset = (g_sent + one_way).saturating_sub(satellite_receive);

    if let Some(dropped) = uplink_buffer.push(TelemetryPacket {
        priority: Priority::Emergency,
        creation_time: state.uptime_ms(),
        payload: SatelliteMessage::SyncResult { offset },
        sequence_no: SEQUENCE_NOT_CONFIRMED,
    }) {
        log_uplink_drop(log_tx, &dropped, state.uptime_ms());
    }

    state.clock_sync.samples.fetch_add(1, Ordering::Relaxed);

    log_tx.try_send(Log {
        source: LogSource::Network,
        event: Event {
            task_id: TaskID::NetworkService,
            event_id: EventID::SyncCompleted,
            data: EventData::TimeSync { offset },
            timestamp: receive_time,
        },
    }).ok();

}

fn handle_telemetry(
    state: &Arc<GroundState>,
    log_tx: &SyncSender<Log>,
    uplink_buffer: &Arc<BoundedBuffer>,
    event: Event,
    receive_time: u64,
) {
    log_tx.try_send(Log {
        source: LogSource::External,
        event,
    }).ok();

    match event.event_id {
        EventID::SubsystemFault     => handle_subsystem_fault(state, log_tx, uplink_buffer, event, receive_time),
        EventID::CommandCompletion  => handle_command_completion(state, log_tx, event, receive_time),
        EventID::SyncCompleted      => {
            state.clock_sync.is_calibrated.store(true, Ordering::Release);
        }
        EventID::MissionAbort => {
            state.is_running.store(false, Ordering::SeqCst);
        }
        _ => {}
    }
}

fn handle_subsystem_fault(
    state: &Arc<GroundState>,
    log_tx: &SyncSender<Log>,
    uplink_buffer: &Arc<BoundedBuffer>,
    event: Event,
    receive_time: u64,
) {
    if let EventData::Subsystem { subsystem_id } = event.data {
        if let Some(sub) = state.find_subsystem(subsystem_id) {
            sub.interlock.store(true, Ordering::Release);
            sub.fault_detected_at.store(receive_time, Ordering::Release);
            sub.alert_sent.store(false, Ordering::Release);

            if let Some(dropped) = uplink_buffer.push(TelemetryPacket {
                priority: Priority::Emergency,
                creation_time: state.uptime_ms(),
                payload: SatelliteMessage::Command {
                    command: Command::ClearSubsystemFault { subsystem_id },
                    sent_at: state.uptime_ms(),
                },
                sequence_no: SEQUENCE_NOT_CONFIRMED,
            }) {
                log_uplink_drop(log_tx, &dropped, state.uptime_ms());
            }

            log_tx.send(Log {
                source: LogSource::HealthMonitor,
                event: Event {
                    task_id: TaskID::GlobalSystem,
                    event_id: EventID::SubsystemFault,
                    data: EventData::Subsystem { subsystem_id },
                    timestamp: receive_time,
                },
            }).ok();
        }
    }
}

fn handle_command_completion(
    state: &Arc<GroundState>,
    log_tx: &SyncSender<Log>,
    event: Event,
    receive_time: u64,
) {
    if event.task_id != TaskID::ClearSubsystemFault {
        return;
    }

    if let EventData::Subsystem { subsystem_id } = event.data {
        if let Some(sub) = state.find_subsystem(subsystem_id) {
            if sub.id == subsystem_id && sub.interlock.load(Ordering::Acquire) {
                sub.clear();
                log_tx.send(Log {
                    source: LogSource::HealthMonitor,
                    event: Event {
                        task_id: TaskID::ClearSubsystemFault,
                        event_id: EventID::SubsystemFixed,
                        data: EventData::Subsystem { subsystem_id },
                        timestamp: receive_time,
                    },
                }).ok();
            }
        }
    }
}


fn check_sequence_gaps(
    state: &Arc<GroundState>,
    uplink_buffer: &Arc<BoundedBuffer>,
    log_tx: &SyncSender<Log>,
    packet: &TelemetryPacket,
) {
    if packet.sequence_no == SEQUENCE_NOT_CONFIRMED {
        return;
    }

    let last_seq = state.link.last_packet_sequence.load(Ordering::Acquire);

    if last_seq == 0 {
        state.link.last_packet_sequence.store(packet.sequence_no, Ordering::Release);
        state.link.last_packet_time.store(state.uptime_ms(), Ordering::Release);
        return;
    }

    if packet.sequence_no > last_seq + 1 {
        let missing_count = packet.sequence_no - (last_seq + 1);
        state.link.consecutive_missing.fetch_add(missing_count, Ordering::Relaxed);

        if missing_count <= 3 {
            for missing_seq in (last_seq + 1)..packet.sequence_no {
                let retransmit = TelemetryPacket {
                    priority: Priority::Critical,
                    creation_time: state.uptime_ms(),
                    payload: SatelliteMessage::Command {
                        command: Command::RequestRetransmit { sequence_no: missing_seq },
                        sent_at: state.uptime_ms(),
                    },
                    sequence_no: SEQUENCE_NOT_CONFIRMED,
                };
                if let Some(dropped) = uplink_buffer.push(retransmit) {
                    log_uplink_drop(log_tx, &dropped, state.uptime_ms());
                }
            }
        } else {
            log_tx.try_send(Log {
                source: LogSource::Network,
                event: Event {
                    task_id: TaskID::NetworkService,
                    event_id: EventID::DataLoss,
                    data: EventData::PacketDrain { count: missing_count },
                    timestamp: state.uptime_ms(),
                },
            }).ok();
        }
    } else {
        state.link.consecutive_missing.store(0, Ordering::Release);
    }

    state.link.last_packet_sequence.store(packet.sequence_no, Ordering::Release);
    state.link.last_packet_time.store(state.uptime_ms(), Ordering::Release);
}

fn log_uplink_drop(log_tx: &SyncSender<Log>, dropped: &TelemetryPacket, now: u64) {
    log_tx.try_send(Log {
        source: LogSource::Network,
        event: Event {
            task_id: dropped.payload.command_task_id(),
            event_id: EventID::DataLoss,
            data: EventData::None,
            timestamp: now,
        },
    }).ok();
}