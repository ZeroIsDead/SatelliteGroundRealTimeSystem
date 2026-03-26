use std::{collections::BinaryHeap, sync::mpsc::SyncSender};
use std::sync::{Mutex, Arc};
use crate::{state::SatelliteState, types::{TaskID, EventID, Log, Event,SatelliteMessage, LogSource, EventData, Metrics, Priority, TelemetryPacket}};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use crate::config::{SEQUENCE_NOT_CONFIRMED};

pub struct BoundedBuffer {
    heap: Mutex<BinaryHeap<TelemetryPacket>>,
    pub capacity: usize,
    pub metrics: Metrics,
    pub packet_dropped: AtomicU32,
    pub stale_packet_dropped: AtomicU32,
    pub low_packet_dropped: AtomicU32,
    pub normal_packet_dropped: AtomicU32,
    pub critical_packet_dropped: AtomicU32,
    pub emergency_packet_dropped: AtomicU32,
}

impl BoundedBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            heap: Mutex::new(BinaryHeap::with_capacity(capacity)),
            capacity: capacity,
            metrics: Metrics {
                last_latency_ms: AtomicU64::new(0),
                total_latency_ms: AtomicU64::new(0),
                max_latency_ms: AtomicU64::new(0),
                min_latency_ms: AtomicU64::new(0),
                last_jitter_ms: AtomicU64::new(0),
                total_jitter_ms: AtomicU64::new(0),
                max_jitter: AtomicU64::new(0),
                min_jitter: AtomicU64::new(0),
                number_of_samples: AtomicU32::new(0),
            },
            packet_dropped: AtomicU32::new(0),
            stale_packet_dropped: AtomicU32::new(0),
            low_packet_dropped: AtomicU32::new(0),
            normal_packet_dropped: AtomicU32::new(0),
            critical_packet_dropped: AtomicU32::new(0),
            emergency_packet_dropped: AtomicU32::new(0),
        }
    }

    fn push_inner(&self, item: TelemetryPacket) -> Option<TelemetryPacket> {
        let mut heap = self.heap.lock().unwrap();

        if heap.len() < self.capacity {
            heap.push(item);
            None
        } else {
            let mut data = std::mem::take(&mut *heap).into_vec();

            let min_idx = data.iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.cmp(b))
                .map(|(idx, _)| idx);

            if let Some(idx) = min_idx {
                if item >= data[idx] {
                    let dropped = data.swap_remove(idx); 
                    data.push(item);                    
                    *heap = BinaryHeap::from(data);     
                    Some(dropped)
                } else {
                    *heap = BinaryHeap::from(data);
                    Some(item) 
                }
            } else {
                Some(item)
            }
        }
    }

    pub fn push(&self, item: TelemetryPacket) -> Option<TelemetryPacket> {
        if let Some(dropped) = self.push_inner(item) {
            self.packet_dropped.fetch_add(1, Ordering::Relaxed);

            match dropped.priority {
                Priority::Emergency => {
                    self.emergency_packet_dropped.fetch_add(1, Ordering::Relaxed);}
                Priority::Critical => {
                    self.critical_packet_dropped.fetch_add(1, Ordering::Relaxed);}
                Priority::Normal => {
                    self.normal_packet_dropped.fetch_add(1, Ordering::Relaxed);}
                Priority::Low => {
                    self.low_packet_dropped.fetch_add(1, Ordering::Relaxed);}
            }
            
            return Some(dropped);
        }

        None
    }

    // downlink_buffer and self could be the same pointer
    pub fn push_and_log(&self, source: LogSource, item: TelemetryPacket, state: &Arc<SatelliteState>, log_tx: &SyncSender<Log>, downlink_buffer: &Arc<BoundedBuffer>) {
        if let Some(dropped) = self.push(item) { // Buffer Drop Packet Logic
            let task_id: TaskID = match dropped.payload {
                SatelliteMessage::Telemetry{event} => {
                    let _ = log_tx.try_send(Log {
                        source: source,
                        event: event,
                    });

                    event.task_id
                },
                _ => TaskID::NetworkService
            };

            let _ = log_tx.try_send(Log {
                source: source,
                event: Event {
                    task_id: task_id,
                    event_id: EventID::DataLoss,
                    data: EventData::None,
                    timestamp: state.uptime_ms(),
                },
            });

            downlink_buffer.push(TelemetryPacket { 
                priority: Priority::Normal, 
                creation_time: state.uptime_ms(), 
                payload: SatelliteMessage::Telemetry {
                    event: Event {
                        task_id: task_id,
                        event_id: EventID::DataLoss,
                        data: EventData::None,
                        timestamp: state.uptime_ms(),
                    }
                }, 
                sequence_no: SEQUENCE_NOT_CONFIRMED, 
            });
        } else {
            if let SatelliteMessage::Telemetry { event } = item.payload {
                let _ = log_tx.try_send(Log { source, event });
            }
        }
    }

    pub fn pop(&self) -> Option<TelemetryPacket> {
        let mut heap = self.heap.lock().unwrap();
        heap.pop()
    }
    
    pub fn len(&self) -> usize {
        self.heap.lock().unwrap().len()
    }

    pub fn clear(&self) {
        let mut heap = self.heap.lock().unwrap();
        heap.clear();
    }
}