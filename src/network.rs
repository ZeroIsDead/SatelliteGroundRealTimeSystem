use std::sync::mpsc::{SyncSender};
use std::time::{Duration};
use crate::types::{SatelliteMessage, TelemetryPacket, Log, *};
use crate::config::{PACKET_HISTORY_BUFFER_CAPACITY, INIT_HANDSHAKE_LIMIT_MS, NETWORK_MS, NETWORK_PORT, NETWORK_PRIORITY, VISIBILITY_WINDOW_LIMIT_MS};
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

    let mut history: [Option<TelemetryPacket>; PACKET_HISTORY_BUFFER_CAPACITY] = [None; PACKET_HISTORY_BUFFER_CAPACITY]; // Fixed size array = no heap allocation jitter!
    let mut history_idx = 0;

    while state.is_running.load(Ordering::Relaxed) {
        let is_visible = state.is_visible.load(Ordering::Acquire);

        if is_visible && !was_visible {
            let pass_start = state.uptime_ms();
            
            if let Ok(mut stream) = TcpStream::connect_timeout(&NETWORK_PORT.parse().unwrap(), Duration::from_millis(INIT_HANDSHAKE_LIMIT_MS)) {
                let _ = stream.set_write_timeout(Some(Duration::from_millis(10)));
                let _ = stream.set_read_timeout(Some(Duration::from_millis(1)));
                let mut length_buf = [0u8; 2];

                while state.uptime_ms() - pass_start < VISIBILITY_WINDOW_LIMIT_MS {
                    
                    if let Some(packet) = downlink_buffer.pop() {
                        let queue_latency_ms = (state.uptime_ms() - packet.creation_time) as u32;
                        let queue_jitter_ms = downlink_buffer.metrics.last_latency_ms.load(Ordering::Relaxed) - queue_latency_ms;

                        let current_sequence_no = state.packet_sequence_no.load(Ordering::Relaxed);
                      
                        downlink_buffer.push_and_log(LogSource::Network, 
                            TelemetryPacket{
                            priority: packet.priority,
                            creation_time: state.get_synchronized_timestamp(),
                            payload: SatelliteMessage::Telemetry {
                                event: Event {
                                    task_id: TaskID::NetworkService,
                                    event_id: EventID::QueuePerformance,
                                    data: EventData::QueuePerformance {
                                        latency_ms: queue_latency_ms,
                                        jitter_ms: queue_jitter_ms,
                                    },
                                    timestamp: state.uptime_ms(),
                                },
                            },
                            sequence_no: current_sequence_no,
                        },
                        &state, &log_tx, &downlink_buffer);

                        downlink_buffer.metrics.last_latency_ms.store(queue_latency_ms, Ordering::Relaxed);
                        downlink_buffer.metrics.jitter_ms.store(queue_jitter_ms, Ordering::Relaxed);
                    
                        let outgoing_telemetry = TelemetryPacket {
                            priority: packet.priority,
                            creation_time: state.uptime_ms(),
                            payload: packet.payload,
                            sequence_no: state.packet_sequence_no.load(Ordering::Relaxed),
                        };

                        state.packet_sequence_no.fetch_add(1, Ordering::Relaxed);

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
                        let incoming_telemetry = TelemetryPacket {
                            priority: packet.priority,
                            creation_time: network_arrival_time,
                            payload: packet.payload,
                            sequence_no: packet.sequence_no,
                        };

                        let found = match packet.payload {
                            SatelliteMessage::SyncRequest => {
                                let current_sequence_no = state.packet_sequence_no.load(Ordering::Relaxed);

                                downlink_buffer.push_and_log(LogSource::Network, 
                                    TelemetryPacket{
                                    priority: Priority::Critical,
                                    creation_time: state.get_synchronized_timestamp(),
                                    payload: SatelliteMessage::SyncResponse {
                                        ground_sent: packet.creation_time,
                                        satellite_receive: network_arrival_time,
                                    },
                                    sequence_no: current_sequence_no,
                                }, 
                                &state, &log_tx, &downlink_buffer);

                                state.packet_sequence_no.fetch_add(1, Ordering::Relaxed);

                                if state.clock_sync.is_calibrated.load(Ordering::Relaxed) {
                                    let _ = log_tx.send(Log {
                                        source: LogSource::Network,
                                        event: Event {
                                            task_id: TaskID::NetworkService,
                                            event_id: EventID::SyncStart,
                                            data: EventData::None,
                                            timestamp: network_arrival_time,
                                        }
                                    });

                                    state.clock_sync.is_calibrated.swap(false, Ordering::Relaxed);
                                }

                                true
                            },
                            SatelliteMessage::SyncResult { offset} => {
                                state.clock_sync.total_offset_ms.fetch_add(offset, Ordering::Relaxed);
                                state.clock_sync.number_of_sample.fetch_add(1, Ordering::Relaxed);

                                let average = state.clock_sync.total_offset_ms.load(Ordering::Relaxed) / state.clock_sync.number_of_sample.load(Ordering::Relaxed) as u64;

                                state.clock_sync.average_offset_ms.store(average, Ordering::Relaxed);

                                if state.clock_sync.number_of_sample.load(Ordering::Relaxed) % 10 == 0 {
                                    state.clock_sync.is_calibrated.swap(true, Ordering::Relaxed);
                                    let _ = log_tx.send(Log {
                                        source: LogSource::Network,
                                        event: Event {
                                            task_id: TaskID::NetworkService,
                                            event_id: EventID::SyncCompleted,
                                            data: EventData::TimeSync { offset: offset },
                                            timestamp: network_arrival_time,
                                        }
                                    });
                                }

                                true
                            }
                            SatelliteMessage::Command { command, sent_at: _} => {
                                if let Command::RequestRetransmit { sequence_no } = command {
                                    let current_sequence_no = state.packet_sequence_no.load(Ordering::Relaxed);

                                    if current_sequence_no - sequence_no >= 100 {
                                        downlink_buffer.push_and_log(LogSource::Network, 
                                            TelemetryPacket{
                                            priority: packet.priority,
                                            creation_time: state.get_synchronized_timestamp(),
                                            payload: SatelliteMessage::Telemetry {
                                                event: Event {
                                                    task_id: TaskID::NetworkService,
                                                    event_id: EventID::PacketTooOld,
                                                    data: EventData::None,
                                                    timestamp: state.uptime_ms(),
                                                }
                                            },
                                            sequence_no: current_sequence_no,
                                        }, 
                                        &state, &log_tx, &downlink_buffer);

                                        state.packet_sequence_no.fetch_add(1, Ordering::Relaxed);
                                    }
                                    
                                    let target_index = sequence_no % PACKET_HISTORY_BUFFER_CAPACITY as u32;

                                    if let Some(lost_packet) = history[target_index as usize] {
                                        let outgoing_history_packet = TelemetryPacket {
                                            priority: Priority::Critical,
                                            creation_time: network_arrival_time,
                                            payload: lost_packet.payload,
                                            sequence_no: lost_packet.sequence_no,
                                        };

                                        downlink_buffer.push_and_log(LogSource::Network, outgoing_history_packet, &state, &log_tx, &downlink_buffer);
                                        }
                                }
                                
                                true
                            }
                            _ => false
                        };


                        if found {
                            continue;
                        }

                        uplink_buffer.push_and_log(LogSource::Network, incoming_telemetry, &state, &log_tx, &downlink_buffer);
                    }
                }
            } else {
                downlink_buffer.push_and_log(LogSource::Network, 
                    TelemetryPacket{
                    priority: Priority::Critical,
                    creation_time: state.get_synchronized_timestamp(),
                    payload: SatelliteMessage::Telemetry {
                            event: Event {
                            task_id: TaskID::NetworkService,
                            event_id: EventID::MissedCommunication,
                            data: EventData::None,
                            timestamp: state.uptime_ms(),
                        },
                    },
                    sequence_no: 0,
                }, 
                &state, &log_tx, &downlink_buffer);
            } 

            state.cpu_active_ms.fetch_add(state.uptime_ms() - pass_start, Ordering::Relaxed);
        }


        was_visible = is_visible;
        thread::sleep(Duration::from_millis(NETWORK_MS)); // Polling interval
    }
}

