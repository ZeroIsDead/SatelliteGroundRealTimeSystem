use serde::{Deserialize, Serialize};

// --- CONSTANTS FOR RUBRIC REQUIREMENTS ---
pub const DECODE_DEADLINE_MS: u128 = 3;
pub const DISPATCH_DEADLINE_MS: u128 = 2;
pub const FAULT_RESPONSE_LIMIT_MS: u128 = 100;
pub const MAX_LOSS_GAP: u32 = 3;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum Command {
    // Low Priority (Routine)
    RotateAntenna { target_angle: u16 },
    SetPowerMode { mode: u8 },
    // High Priority (Urgent)
    ClearFault { subsystem_id: u8 },
    RequestRetransmit { sequence_no: u32 },
    SystemReboot,
}

impl Command {
    // Helper to determine if a command needs to bypass the normal queue
    pub fn is_urgent(&self) -> bool {
        matches!(self, Command::ClearFault {..} | Command::RequestRetransmit {..} | Command::SystemReboot)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SatelliteMessage {
    Uplink(Command),
    Downlink(TelemetryPacket),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TelemetryPacket {
    pub sequence_no: u32,
    pub timestamp: u64, // Satellite uptime when sent
    pub payload: PayloadType,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum PayloadType {
    RoutineTelemetry { antenna_angle: u16, power_draw: u8 },
    FaultInjected { subsystem_id: u8, code: u8 },
    FaultResolved { subsystem_id: u8 },
    RetransmitFailed { sequence_no: u32 }, // If the 1024 buffer overwrote it
}