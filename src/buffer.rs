use std::{collections::BinaryHeap, sync::mpsc::SyncSender};
use std::sync::{Mutex, Arc};
use crate::{state::SatelliteState, types::{TaskID, EventID, Log, Event,SatelliteMessage, LogSource, EventData, Metrics, Priority, TelemetryPacket}};
use std::sync::atomic::{AtomicU32};

pub struct BoundedBuffer {
    heap: Mutex<BinaryHeap<TelemetryPacket>>,
    pub capacity: usize,
    pub metrics: Metrics,
}

impl BoundedBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            heap: Mutex::new(BinaryHeap::with_capacity(capacity)),
            capacity: capacity,
            metrics: Metrics {
                last_latency_ms: AtomicU32::new(0),
                jitter_ms: AtomicU32::new(0),
            }
        }
    }

    fn push(&self, item: TelemetryPacket) -> Option<TelemetryPacket> {
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
                if item > data[idx] {
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

    pub fn push_and_log(&self, source: LogSource, item: TelemetryPacket, state: &Arc<SatelliteState>, log_tx: &SyncSender<Log>, downlink_buffer: &Arc<BoundedBuffer>) {
        if let Some(dropped) = &self.push(item) { // Buffer Drop Packet Logic
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
                priority: Priority::Critical, 
                creation_time: state.uptime_ms(), 
                payload: SatelliteMessage::Telemetry {
                    event: Event {
                        task_id: task_id,
                        event_id: EventID::DataLoss,
                        data: EventData::None,
                        timestamp: state.get_synchronized_timestamp(),
                    }
                }, 
                sequence_no: 0, 
            });
        }
    }

    pub fn pop(&self) -> Option<TelemetryPacket> {
        let mut heap = self.heap.lock().unwrap();
        heap.pop() 
    }
    
    pub fn len(&self) -> usize {
        self.heap.lock().unwrap().len()
    }
}