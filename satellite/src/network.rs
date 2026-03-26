use std::sync::mpsc::{SyncSender};
use std::time::{Duration};
use crate::types::{SatelliteMessage, TelemetryPacket, Log, *};
use crate::config::{INIT_HANDSHAKE_LIMIT_MS, NETWORK_MS, NETWORK_PORT, NETWORK_PRIORITY, NETWORK_READ_TIMEOUT, NETWORK_WRITE_TIMEOUT, PACKET_HISTORY_BUFFER_CAPACITY, SEQUENCE_NOT_CONFIRMED, VISIBILITY_WINDOW_CYCLE_MS, VISIBILITY_WINDOW_LIMIT_MS};
use std::net::TcpStream;
use std::io::{Read, Write};
use std::thread;
use std::sync::atomic::{Ordering};
use std::sync::Arc;
use crate::state::SatelliteState;
use crate::buffer::BoundedBuffer;
use bincode;
use thread_priority::*;

pub fn run_network_thread(
    state: Arc<SatelliteState>, 
    downlink_buffer: Arc<BoundedBuffer>, 
    uplink_buffer: Arc<BoundedBuffer>,
    log_tx: SyncSender<Log>
) {
    set_current_thread_priority(ThreadPriority::Crossplatform(NETWORK_PRIORITY.try_into().unwrap())).unwrap();
    let mut was_visible = false;

    let mut history: Vec<Option<TelemetryPacket>> = vec![None; PACKET_HISTORY_BUFFER_CAPACITY]; // Fixed size array = no heap allocation jitter
    let mut history_idx = 0;

    while state.is_running.load(Ordering::SeqCst) {
        let is_visible = state.network.is_visible.load(Ordering::Acquire);

        if is_visible && !was_visible {
            let pass_start = state.uptime_ms();
            
            if let Ok(mut stream) = TcpStream::connect_timeout(&NETWORK_PORT.parse().unwrap(), Duration::from_micros(INIT_HANDSHAKE_LIMIT_MS)) {
                let _ = stream.set_write_timeout(Some(Duration::from_micros(NETWORK_WRITE_TIMEOUT)));
                let _ = stream.set_read_timeout(Some(Duration::from_micros(NETWORK_READ_TIMEOUT)));
                let _ = stream.set_nodelay(true);
                let mut length_buf = [0u8; 2];

                let _ = log_tx.try_send(Log {
                    source: LogSource::Network,
                    event: Event {
                        task_id: TaskID::NetworkService,
                        event_id: EventID::ConnectionStart,
                        data: EventData::None,
                        timestamp: state.uptime_ms(),
                    }
                });

                while state.uptime_ms() - pass_start < VISIBILITY_WINDOW_LIMIT_MS {
                    
                    if let Some(packet) = downlink_buffer.pop() {
                        let queue_latency_ms = state.uptime_ms().saturating_sub(packet.creation_time);

                        if queue_latency_ms > 10 * VISIBILITY_WINDOW_CYCLE_MS && packet.priority != Priority::Emergency {  // Packet Missed 10 Windows
                            downlink_buffer.stale_packet_dropped.fetch_add(1, Ordering::Relaxed);
                            downlink_buffer.packet_dropped.fetch_add(1, Ordering::Relaxed);

                            match packet.priority {
                                Priority::Critical => {
                                    downlink_buffer.critical_packet_dropped.fetch_add(1, Ordering::Relaxed);}
                                Priority::Normal => {
                                    downlink_buffer.normal_packet_dropped.fetch_add(1, Ordering::Relaxed);}
                                Priority::Low => {
                                    downlink_buffer.low_packet_dropped.fetch_add(1, Ordering::Relaxed);}
                                _ => {}
                            }
                            
                            continue;
                        }

                        downlink_buffer.metrics.insert_new_metric(queue_latency_ms);
 
                        let _ = log_tx.try_send(Log { 
                                source: LogSource::Network, 
                                event: Event {
                                    task_id: TaskID::DownlinkNetworkService,
                                    event_id: EventID::QueuePerformance,
                                    data: EventData::QueuePerformance {
                                        latency_ms: queue_latency_ms,
                                        jitter_ms: downlink_buffer.metrics.last_jitter_ms.load(Ordering::Relaxed),
                                        buffer_fill_rate: state.buffer_fill_rate.load(Ordering::Relaxed),
                                        sample_count: downlink_buffer.metrics.number_of_samples.load(Ordering::Relaxed)
                                    },
                                    timestamp: state.uptime_ms(),
                                },
                            });

                        let outgoing_telemetry = TelemetryPacket {
                            priority: packet.priority,
                            creation_time: state.get_synchronized_timestamp(),
                            payload: packet.payload,
                            sequence_no: if packet.sequence_no == SEQUENCE_NOT_CONFIRMED {
                               state.network.packet_sequence_no.fetch_add(1, Ordering::SeqCst)
                            } else {
                                packet.sequence_no
                            },
                        };

                        if let SatelliteMessage::Telemetry { mut event } = outgoing_telemetry.payload {
                            event.timestamp = state.synchronize_timestamp(event.timestamp);
                        }

                        history[history_idx] = Some(outgoing_telemetry);
                        history_idx = (history_idx + 1) % PACKET_HISTORY_BUFFER_CAPACITY;

                        // Serialize and Send
                        if let Ok(bytes) = bincode::serialize(&outgoing_telemetry) {
                            let length = bytes.len() as u16;

                            if stream.write_all(&length.to_be_bytes()).is_err() {
                                break;
                            }

                            if stream.write_all(&bytes).is_err() {
                                break;
                            }
                        }
                    }


                    if let Err(_) = stream.read_exact(&mut length_buf) { 
                        continue;
                    };

                    let length = u16::from_be_bytes(length_buf) as usize;

                    let mut payload_buf = vec![0u8; length];

                    if let Err(_) = stream.read_exact(&mut payload_buf) {
                        continue;
                    };

                    let network_arrival_time = state.uptime_ms();
                    if let Ok(packet) = bincode::deserialize::<TelemetryPacket>(&&payload_buf) {

                        if state.clock_sync.number_of_sample.load(Ordering::Relaxed) > 0 {
                            state.network.metrics.insert_new_metric(
                                packet.creation_time.abs_diff(state.get_synchronized_timestamp())
                            );
                        }
 
                        log_tx.try_send(Log {
                            source: LogSource::Network,
                            event: Event {
                                task_id: TaskID::UplinkNetworkService,
                                event_id: EventID::NetworkPerformance,
                                data: EventData::NetworkPerformance {
                                    priority: packet.priority,
                                    latency_ms: state.network.metrics.last_latency_ms.load(Ordering::Relaxed),
                                    jitter_ms: state.network.metrics.last_jitter_ms.load(Ordering::Relaxed),
                                    sample_count: state.network.metrics.number_of_samples.load(Ordering::Relaxed),
                                },
                                timestamp: network_arrival_time,
                            }
                        }).ok();

                        let incoming_telemetry = TelemetryPacket {
                            priority: packet.priority,
                            creation_time: network_arrival_time,
                            payload: packet.payload,
                            sequence_no: packet.sequence_no,
                        };

                        match packet.payload {
                            SatelliteMessage::SyncRequest => {
                                let current_sequence_no = state.network.packet_sequence_no.load(Ordering::SeqCst);

                                downlink_buffer.push_and_log(LogSource::Network, 
                                    TelemetryPacket{
                                    priority: Priority::Emergency,
                                    creation_time: state.uptime_ms(),
                                    payload: SatelliteMessage::SyncResponse {
                                        ground_sent: packet.creation_time,
                                        satellite_receive: network_arrival_time,
                                    },
                                    sequence_no: current_sequence_no,
                                }, 
                                &state, &log_tx, &downlink_buffer);

                                state.network.packet_sequence_no.fetch_add(1, Ordering::SeqCst);

                                if state.clock_sync.is_calibrated.load(Ordering::SeqCst) {
                                    let _ = log_tx.try_send(Log {
                                        source: LogSource::Network,
                                        event: Event {
                                            task_id: TaskID::UplinkNetworkService,
                                            event_id: EventID::SyncStart,
                                            data: EventData::None,
                                            timestamp: network_arrival_time,
                                        }
                                    });

                                    state.clock_sync.is_calibrated.store(false, Ordering::SeqCst);
                                }
                            },
                            SatelliteMessage::SyncResult { offset} => {
                                state.clock_sync.total_offset_ms.fetch_add(offset, Ordering::Relaxed);
                                state.clock_sync.number_of_sample.fetch_add(1, Ordering::Relaxed);

                                let average = state.clock_sync.total_offset_ms.load(Ordering::Relaxed) / state.clock_sync.number_of_sample.load(Ordering::Relaxed) as u64;

                                state.clock_sync.average_offset_ms.store(average, Ordering::Relaxed);

                                let _ = log_tx.try_send(Log {
                                        source: LogSource::Network,
                                        event: Event {
                                            task_id: TaskID::UplinkNetworkService,
                                            event_id: EventID::SyncOngoing,
                                            data: EventData::TimeSync { offset: offset },
                                            timestamp: network_arrival_time,
                                        }
                                    });

                                if state.clock_sync.number_of_sample.load(Ordering::Relaxed) % 10 == 0 { // 10 Packets = Synced
                                    state.clock_sync.is_calibrated.store(true, Ordering::SeqCst);
                                    let _ = log_tx.try_send(Log {
                                        source: LogSource::Network,
                                        event: Event {
                                            task_id: TaskID::UplinkNetworkService,
                                            event_id: EventID::SyncCompleted,
                                            data: EventData::TimeSync { offset: offset },
                                            timestamp: network_arrival_time,
                                        }
                                    });
                                }

                            }
                            SatelliteMessage::Command { command, sent_at: _} => {
                                if let Command::RequestRetransmit { sequence_no } = command {
                                    let current_sequence_no = state.network.packet_sequence_no.load(Ordering::SeqCst);

                                    if current_sequence_no - sequence_no >= PACKET_HISTORY_BUFFER_CAPACITY as u32 {
                                        downlink_buffer.push_and_log(LogSource::Network, 
                                            TelemetryPacket{
                                            priority: packet.priority,
                                            creation_time: state.uptime_ms(),
                                            payload: SatelliteMessage::Telemetry {
                                                event: Event {
                                                    task_id: TaskID::UplinkNetworkService,
                                                    event_id: EventID::RetransmitFailed,
                                                    data: EventData::None,
                                                    timestamp: state.uptime_ms(),
                                                }
                                            },
                                            sequence_no: current_sequence_no,
                                        }, 
                                        &state, &log_tx, &downlink_buffer);

                                        state.network.packet_sequence_no.fetch_add(1, Ordering::SeqCst);
                                    }
                                    
                                    let target_index = sequence_no % PACKET_HISTORY_BUFFER_CAPACITY as u32;

                                    if let Some(lost_packet) = history[target_index as usize] {
                                        let outgoing_history_packet = TelemetryPacket {
                                            priority: Priority::Normal,
                                            creation_time: network_arrival_time,
                                            payload: lost_packet.payload,
                                            sequence_no: lost_packet.sequence_no,
                                        };

                                        downlink_buffer.push_and_log(LogSource::Network, outgoing_history_packet, &state, &log_tx, &downlink_buffer);
                                        }
                                } else {
                                    uplink_buffer.push_and_log(LogSource::Network, incoming_telemetry, &state, &log_tx, &downlink_buffer);
                                }
                                
                            }
                            _ => {}
                        };
                        
                    }
                }

                let _ = log_tx.try_send(Log {
                    source: LogSource::Network,
                    event: Event {
                        task_id: TaskID::NetworkService,
                        event_id: EventID::ConnectionEnd,
                        data: EventData::None,
                        timestamp: state.uptime_ms(),
                    }
                });

            } else {
                downlink_buffer.push_and_log(LogSource::Network, 
                    TelemetryPacket{
                    priority: Priority::Normal,
                    creation_time: state.uptime_ms(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                            task_id: TaskID::NetworkService,
                            event_id: EventID::MissedCommunication,
                            data: EventData::None,
                            timestamp: state.uptime_ms(),
                        },
                    },
                    sequence_no: SEQUENCE_NOT_CONFIRMED,
                }, 
                &state, &log_tx, &downlink_buffer);
            } 

            state.cpu_active_ms.fetch_add(state.uptime_ms() - pass_start, Ordering::SeqCst);
        }


        was_visible = is_visible;
        thread::sleep(Duration::from_micros(NETWORK_MS)); // Polling interval
    }
}

