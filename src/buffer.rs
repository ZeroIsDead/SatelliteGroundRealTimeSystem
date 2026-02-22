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
            capacity,
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
            if item.priority == Priority::Low {
                return Some(item); 
            }

            let mut vec = heap.drain().collect::<Vec<_>>();
            
            vec.sort_unstable_by(|a, b| a.cmp(b).reverse()); 
            
            let dropped_item = vec.remove(0); 
            
            vec.push(item); 
            
            *heap = BinaryHeap::from(vec);

            Some(dropped_item) 
        }
    }

    pub fn push_and_log(&self, source: LogSource, item: TelemetryPacket, state: &Arc<SatelliteState>, log_tx: &SyncSender<Log>, downlink_buffer: &Arc<BoundedBuffer>) {
        if let Some(dropped) = &self.push(item) { // Buffer Drop Packet Logic
            let task_id: TaskID = match dropped.payload {
                SatelliteMessage::Telemetry{event} => event.task_id,
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