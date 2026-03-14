use std::collections::BinaryHeap;
use std::cmp::Ordering as StdOrdering;
use std::time::Instant;
use crate::types::{Command, DISPATCH_DEADLINE_MS};
use std::net::TcpStream;

#[derive(Debug)]
struct ScheduledCommand {
    priority: u8, // 0 = Low, 255 = Critical
    cmd: Command,
    created_at: Instant,
}

// Implement traits so BinaryHeap knows how to sort by priority
impl Ord for ScheduledCommand {
    fn cmp(&self, other: &Self) -> StdOrdering { self.priority.cmp(&other.priority) }
}
impl PartialOrd for ScheduledCommand {
    fn partial_cmp(&self, other: &Self) -> Option<StdOrdering> { Some(self.cmp(other)) }
}
impl PartialEq for ScheduledCommand {
    fn eq(&self, other: &Self) -> bool { self.priority == other.priority }
}
impl Eq for ScheduledCommand {}

pub fn run_command_scheduler(mut stream: TcpStream, cmd_rx: Receiver<Command>, interlocks: Arc<InterlockManager>) {
    let mut queue = BinaryHeap::new();

    loop {
        // 1. Pull all waiting commands from the channel into our Priority Queue
        while let Ok(cmd) = cmd_rx.try_recv() {
            let priority = if cmd.is_urgent() { 255 } else { 10 };
            queue.push(ScheduledCommand { priority, cmd, created_at: Instant::now() });
        }

        // 2. Process the highest priority command
        if let Some(scheduled) = queue.pop() {
            // Requirement 3: Validate against Interlocks
            if interlocks.is_safe(&scheduled.cmd) {
                let start_dispatch = Instant::now();

                // Serialization and Framing
                let bytes = bincode::serialize(&scheduled.cmd).unwrap();
                let len = (bytes.len() as u16).to_be_bytes();
                
                let _ = stream.write_all(&len);
                let _ = stream.write_all(&bytes);

                // Requirement 2: Log Deadline Adherence
                let dispatch_time = start_dispatch.elapsed().as_millis();
                if dispatch_time > DISPATCH_DEADLINE_MS {
                    println!("[LOG] Urgent Deadline Missed: Dispatch took {}ms", dispatch_time);
                }
            } else {
                // Requirement 3: Document rejection reasons
                println!("[REJECTED] Command {:?} blocked by active interlock.", scheduled.cmd);
            }
        }
        
        thread::sleep(Duration::from_millis(1)); // High-frequency polling
    }
}