use std::collections::BinaryHeap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, AtomicU64};
use crate::types::{TelemetryPacket, Metrics};

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
                last_latency_ms: AtomicU64::new(0),
                total_latency_ms: AtomicU64::new(0),
                total_jitter_ms: AtomicU64::new(0),
                number_of_samples: AtomicU32::new(0),
            },
        }
    }

    pub fn push(&self, item: TelemetryPacket) -> Option<TelemetryPacket> {
        let mut heap = self.heap.lock().unwrap();
        if heap.len() < self.capacity {
            heap.push(item);
            return None;
        }

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

    pub fn pop(&self) -> Option<TelemetryPacket> {
        self.heap.lock().unwrap().pop()
    }

    pub fn len(&self) -> usize {
        self.heap.lock().unwrap().len()
    }
}