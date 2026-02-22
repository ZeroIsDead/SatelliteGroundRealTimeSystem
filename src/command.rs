use std::time::{Duration};
use crate::state::SatelliteState;
use crate::buffer::{BoundedBuffer};
use crate::types::*;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender};
use std::thread;
use crate::config::{COMMAND_MS, COMMAND_PRIORITY};
use thread_priority::*;

pub fn run_command_executor(
    state: Arc<SatelliteState>,
    downlink_buffer: Arc<BoundedBuffer>,
    uplink_buffer: Arc<BoundedBuffer>,
    log_tx: SyncSender<Log>
) {
    set_current_thread_priority(ThreadPriority::Crossplatform(COMMAND_PRIORITY.try_into().unwrap())).unwrap();

    while state.is_running.load(Ordering::Relaxed) {
        if let Some(packet) = uplink_buffer.pop() {
            let start_time = state.uptime_ms();

            match packet.payload {
                SatelliteMessage::Command { command, .. } => {
                    if let Some(requirements) = command.required_health() {
                        if state.subsystem_health[requirements as usize].fault_interlock.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                    execute_instruction(command, &state, &log_tx, &downlink_buffer);
                }
                _ => {}
            }

            state.cpu_active_ms.fetch_add(state.uptime_ms() - start_time, Ordering::Relaxed);
        }
        
        thread::sleep(Duration::from_millis(COMMAND_MS));
    }
}

fn execute_instruction(command: Command, state: &Arc<SatelliteState>, log_tx: &SyncSender<Log>, downlink_buffer: &Arc<BoundedBuffer>) {
    let task_id = match command {
        Command::ClearSubsystemFault { subsystem_id } => {
            for subsystem in &state.subsystem_health {
                if subsystem.id == subsystem_id && subsystem.fault.load(Ordering::Relaxed) {
                    subsystem.fault_interlock.swap(false, Ordering::Relaxed);
                }
            }

            TaskID::ClearSubsystemFault
        },
        Command::RotateAntenna { target_angle } => {
            state.subsystem_health[SubsystemID::Antenna as usize].value.store(target_angle as u32, Ordering::Relaxed);
        
            TaskID::RotateAntenna
            
        },
        Command::SetPowerMode { mode } => {
            state.subsystem_health[SubsystemID::Power as usize].value.store(mode as u32, Ordering::Relaxed);
        
            TaskID::SetPowerMode
        },
        _ => {
            

            TaskID::None

        }
    };

    if task_id == TaskID::None {
        downlink_buffer.push_and_log(LogSource::CommandExecutor, 
            TelemetryPacket{
            priority: Priority::Critical,
            creation_time: state.get_synchronized_timestamp(),
            payload: SatelliteMessage::Telemetry {
                    event: Event {
                    task_id: TaskID::GlobalSystem,
                    event_id: EventID::CommandNotFound,
                    data: EventData::None,
                    timestamp: state.uptime_ms(),
                },
            },
            sequence_no: 0,
        }, 
        state, log_tx, downlink_buffer);

        return;
    }

    downlink_buffer.push_and_log(LogSource::CommandExecutor, 
        TelemetryPacket{
        priority: Priority::Critical,
        creation_time: state.get_synchronized_timestamp(),
        payload: SatelliteMessage::Telemetry {
                event: Event {
                    task_id: task_id,
                    event_id: EventID::CommandCompletion,
                    data: EventData::None,
                    timestamp: state.uptime_ms(),
                },
        },
        sequence_no: 0,
    }, 
    state, log_tx, downlink_buffer);

    
}