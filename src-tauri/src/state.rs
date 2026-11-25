//! Snapshot of what the radio is doing, sent to the UI as JSON.
//! `None` means "not known from this connection type" and renders blank.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Link {
    #[default]
    Stopped,
    Connecting,
    Up,
    Lost,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RigState {
    pub link: Link,
    pub message: String,
    /// The last connection problem; kept through retries until the link is up.
    pub error: Option<String>,
    pub attempt: u32,
    pub retry_in: Option<u32>,
    pub source: String,
    pub polls_per_sec: f32,
    pub errors: u32,
    /// Result of the last control action ("Refused: …"), shown briefly.
    pub control_note: Option<String>,
    /// Whether this connection type supports control at all.
    pub can_control: bool,

    // VFO-A (main) and VFO-B rows
    pub freq_hz: Option<u64>,
    pub mode: Option<String>,
    pub band: Option<String>,
    pub freq_b_hz: Option<u64>,
    pub mode_b: Option<String>,
    /// "VFO", "MEM", "MT", "PMS".
    pub vfo_mem: Option<String>,
    pub mem_channel: Option<String>,
    pub clar_on: Option<bool>,
    pub clar_hz: Option<i32>,

    pub tx: bool,
    pub busy: Option<bool>,
    // receive
    pub s_raw: Option<u8>,
    pub s_label: Option<String>,
    // transmit
    /// Meter chosen with the radio's METER key: COMP, ALC, PO, SWR, ID.
    pub meter: Option<String>,
    /// Raw 0..255 for the selected meter while transmitting.
    pub meter_raw: Option<u8>,
    pub po_watts: Option<f32>,
    pub swr: Option<f32>,

    // indicators shown on the LCD
    pub tnr: Option<bool>,
    pub vox: Option<bool>,
    pub prc: Option<bool>,
    pub mon: Option<bool>,
    pub split: Option<bool>,
    pub lock: Option<bool>,
    pub ipo: Option<bool>,
    pub att: Option<bool>,
    pub nar: Option<bool>,
    pub nb: Option<bool>,
    pub fst: Option<bool>,
    pub dnr: Option<bool>,
    pub dnf: Option<bool>,
    // extra status (footer)
    pub agc: Option<String>,
    pub power_w: Option<u32>,
}
