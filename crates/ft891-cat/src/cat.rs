//! Yaesu FT-891 CAT protocol (the ASCII "new CAT" command set).
//!
//! Every command is two letters, optional parameters and a `;` terminator,
//! e.g. `FA;` → `FA014074000;`. Read commands get an answer; set commands
//! get none. A `?;` answer means the radio rejected the command.
//!
//! Safety model: nothing leaves this crate unless [`is_allowed`] accepts it.
//! The allow-list holds a fixed set of read commands plus a small set of
//! validated set commands (tuning, mode, band, VFO, a few receive toggles and
//! power level). Commands that can key the transmitter (TX, MX, KY, KM, AC
//! tune start, VX), write memories or menus (EX, MW, MA, ...) or reset the
//! radio are never allowed, whatever the caller asks for.
//!
//! Sources: Yaesu "FT-891 CAT Operation Reference Book", cross-checked with
//! Hamlib's newcat/ft891 driver (mode table, IF layout, meter calibration).

use serde::{Deserialize, Serialize};

pub const RADIO_ID: &str = "0650";

// ------------------------------------------------------------------ modes

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Lsb,
    Usb,
    CwU,
    Fm,
    Am,
    RttyL,
    CwL,
    DataL,
    RttyU,
    DataFm,
    FmN,
    DataU,
    AmN,
}

impl Mode {
    pub fn from_code(c: char) -> Option<Mode> {
        Some(match c {
            '1' => Mode::Lsb,
            '2' => Mode::Usb,
            '3' => Mode::CwU,
            '4' => Mode::Fm,
            '5' => Mode::Am,
            '6' => Mode::RttyL,
            '7' => Mode::CwL,
            '8' => Mode::DataL,
            '9' => Mode::RttyU,
            'A' => Mode::DataFm,
            'B' => Mode::FmN,
            'C' => Mode::DataU,
            'D' => Mode::AmN,
            _ => return None,
        })
    }
    pub fn code(self) -> char {
        match self {
            Mode::Lsb => '1',
            Mode::Usb => '2',
            Mode::CwU => '3',
            Mode::Fm => '4',
            Mode::Am => '5',
            Mode::RttyL => '6',
            Mode::CwL => '7',
            Mode::DataL => '8',
            Mode::RttyU => '9',
            Mode::DataFm => 'A',
            Mode::FmN => 'B',
            Mode::DataU => 'C',
            Mode::AmN => 'D',
        }
    }
    /// Short label in the style of the radio's LCD ("USB", "CWL", "D-U", "R-L").
    pub fn lcd_label(self) -> &'static str {
        match self {
            Mode::Lsb => "LSB",
            Mode::Usb => "USB",
            Mode::CwU => "CWU",
            Mode::Fm => "FM",
            Mode::Am => "AM",
            Mode::RttyL => "R-L",
            Mode::CwL => "CWL",
            Mode::DataL => "D-L",
            Mode::RttyU => "R-U",
            Mode::DataFm => "D-FM",
            Mode::FmN => "FM-N",
            Mode::DataU => "D-U",
            Mode::AmN => "AM-N",
        }
    }
    pub fn from_label(s: &str) -> Option<Mode> {
        ALL_MODES.iter().copied().find(|m| m.lcd_label() == s)
    }
}

/// Modes offered by the mode picker (DATA-FM left out: the CAT manual marks
/// code A as undefined for this radio).
pub const ALL_MODES: [Mode; 12] = [
    Mode::Lsb,
    Mode::Usb,
    Mode::CwU,
    Mode::CwL,
    Mode::Am,
    Mode::AmN,
    Mode::Fm,
    Mode::FmN,
    Mode::RttyL,
    Mode::RttyU,
    Mode::DataL,
    Mode::DataU,
];

// ------------------------------------------------------------------ bands

pub fn band_name(hz: u64) -> &'static str {
    match hz {
        1_800_000..=2_000_000 => "160m",
        3_500_000..=4_000_000 => "80m",
        5_250_000..=5_450_000 => "60m",
        7_000_000..=7_300_000 => "40m",
        10_100_000..=10_150_000 => "30m",
        14_000_000..=14_350_000 => "20m",
        18_068_000..=18_168_000 => "17m",
        21_000_000..=21_450_000 => "15m",
        24_890_000..=24_990_000 => "12m",
        28_000_000..=29_700_000 => "10m",
        50_000_000..=54_000_000 => "6m",
        _ => "GEN",
    }
}

pub const FREQ_MIN: u64 = 30_000;
pub const FREQ_MAX: u64 = 56_000_000;

// ------------------------------------------------------------------ meters

/// Meter selected with the radio's METER key (CAT `MS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Meter {
    Comp,
    Alc,
    Po,
    Swr,
    Id,
}

impl Meter {
    pub fn from_ms(c: char) -> Option<Meter> {
        Some(match c {
            '0' => Meter::Comp,
            '1' => Meter::Alc,
            '2' => Meter::Po,
            '3' => Meter::Swr,
            '4' => Meter::Id,
            _ => return None,
        })
    }
    /// `RM` selector for this meter.
    pub fn rm_selector(self) -> char {
        match self {
            Meter::Comp => '3',
            Meter::Alc => '4',
            Meter::Po => '5',
            Meter::Swr => '6',
            Meter::Id => '7',
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Meter::Comp => "COMP",
            Meter::Alc => "ALC",
            Meter::Po => "PO",
            Meter::Swr => "SWR",
            Meter::Id => "ID",
        }
    }
}

fn interp(table: &[(f32, f32)], raw: f32) -> f32 {
    if raw <= table[0].0 {
        return table[0].1;
    }
    for w in table.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if raw <= x1 {
            return y0 + (y1 - y0) * (raw - x0) / (x1 - x0);
        }
    }
    table[table.len() - 1].1
}

/// S-meter raw 0..255 → dB relative to S9 (Hamlib FT891_STR_CAL).
pub fn s_db(raw: u8) -> f32 {
    const T: [(f32, f32); 16] = [
        (0.0, -54.0),
        (12.0, -48.0),
        (27.0, -42.0),
        (40.0, -36.0),
        (55.0, -30.0),
        (65.0, -24.0),
        (80.0, -18.0),
        (95.0, -12.0),
        (112.0, -6.0),
        (130.0, 0.0),
        (150.0, 10.0),
        (172.0, 20.0),
        (190.0, 30.0),
        (220.0, 40.0),
        (240.0, 50.0),
        (255.0, 60.0),
    ];
    interp(&T, raw as f32)
}

/// "S7", "S9", "S9+20".
pub fn s_label(raw: u8) -> String {
    let db = s_db(raw);
    if db < 0.0 {
        let s = ((db + 54.0) / 6.0).round().clamp(0.0, 9.0) as i32;
        format!("S{s}")
    } else if db < 5.0 {
        "S9".into()
    } else {
        format!("S9+{}", ((db / 10.0).round() * 10.0) as i32)
    }
}

/// Power-output meter raw → watts (Hamlib FT891_RFPOWER_METER_CAL, 100 W radio).
pub fn po_watts(raw: u8) -> f32 {
    const T: [(f32, f32); 7] =
        [(0.0, 0.0), (10.0, 0.8), (50.0, 8.0), (100.0, 26.0), (150.0, 54.0), (200.0, 92.0), (250.0, 140.0)];
    interp(&T, raw as f32)
}

/// SWR meter raw → SWR ratio. Approximation in the style used for other
/// Yaesu newcat radios: raw 0 = 1.0, ~ 3.0 at mid scale, open circuit at top.
pub fn swr_ratio(raw: u8) -> f32 {
    const T: [(f32, f32); 6] = [(0.0, 1.0), (26.0, 1.2), (52.0, 1.5), (89.0, 2.0), (126.0, 3.0), (255.0, 9.9)];
    interp(&T, raw as f32)
}

// ------------------------------------------------------------------ allow-list

fn digits(s: &str, n: usize) -> Option<u64> {
    if s.len() == n && s.bytes().all(|b| b.is_ascii_digit()) {
        s.parse().ok()
    } else {
        None
    }
}

/// Read commands the monitor polls. Exact strings, nothing else.
pub const READS: &[&str] = &[
    "ID;", "IF;", "OI;", "FA;", "FB;", "MD0;", "SM0;", "TX;", "BY;", "MS;", "RM3;", "RM4;", "RM5;", "RM6;", "RM7;",
    "ST;", "LK;", "VX;", "PA0;", "RA0;", "NA0;", "NB0;", "NR0;", "BC0;", "PR0;", "ML0;", "AC;", "FS;", "GT0;",
    "PC;", "SH0;", "CT0;", "KS;", "AG0;", "RG0;", "SQ0;",
];

/// The single gate every outgoing command passes through.
pub fn is_allowed(cmd: &str) -> bool {
    if READS.contains(&cmd) {
        return true;
    }
    let Some(body) = cmd.strip_suffix(';') else { return false };
    if body.len() < 2 || !body.is_ascii() {
        return false;
    }
    let (op, p) = body.split_at(2);
    match op {
        // Auto-information off only (keeps the reply stream request/response).
        "AI" => p == "0",
        "FA" | "FB" => digits(p, 9).is_some_and(|hz| (FREQ_MIN..=FREQ_MAX).contains(&hz)),
        // MD0 + mode code; DATA-FM ('A') is undefined on this radio.
        "MD" => {
            let b = p.as_bytes();
            b.len() == 2 && b[0] == b'0' && b[1] != b'A' && Mode::from_code(b[1] as char).is_some()
        }
        "BU" | "BD" => p == "0",
        // Swap VFOs, VFO-A→B, VFO-B→A: no parameters.
        "SV" | "AB" | "BA" => p.is_empty(),
        "ST" => p == "0" || p == "1",
        "LK" | "FS" => p == "0" || p == "1",
        "NB" | "NR" | "NA" | "RA" | "PA" => p == "00" || p == "01",
        "PC" => digits(p, 3).is_some_and(|w| (5..=100).contains(&w)),
        _ => false,
    }
}

// ------------------------------------------------------------------ control

/// Something the user asked the radio to do. Each maps to exactly one
/// allow-listed command (or none, if invalid).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Control {
    SetFreq { hz: u64 },
    SetFreqB { hz: u64 },
    SetMode { mode: String },
    BandUp,
    BandDown,
    SwapAb,
    CopyAToB,
    Split { on: bool },
    Lock { on: bool },
    Fast { on: bool },
    NoiseBlanker { on: bool },
    NoiseReduction { on: bool },
    Narrow { on: bool },
    Attenuator { on: bool },
    /// true = AMP (preamp on), false = IPO.
    Preamp { on: bool },
    Power { watts: u32 },
}

fn b(on: bool) -> char {
    if on {
        '1'
    } else {
        '0'
    }
}

impl Control {
    /// The CAT command for this action, or an error saying why it was refused.
    pub fn command(&self) -> Result<String, String> {
        let c = match self {
            Control::SetFreq { hz } | Control::SetFreqB { hz } => {
                if !(FREQ_MIN..=FREQ_MAX).contains(hz) {
                    return Err(format!("{hz} Hz is outside the FT-891 range (0.03–56 MHz)"));
                }
                let op = if matches!(self, Control::SetFreq { .. }) { "FA" } else { "FB" };
                format!("{op}{hz:09};")
            }
            Control::SetMode { mode } => {
                let m = Mode::from_label(mode).ok_or_else(|| format!("unknown mode {mode}"))?;
                format!("MD0{};", m.code())
            }
            Control::BandUp => "BU0;".into(),
            Control::BandDown => "BD0;".into(),
            Control::SwapAb => "SV;".into(),
            Control::CopyAToB => "AB;".into(),
            Control::Split { on } => format!("ST{};", b(*on)),
            Control::Lock { on } => format!("LK{};", b(*on)),
            Control::Fast { on } => format!("FS{};", b(*on)),
            Control::NoiseBlanker { on } => format!("NB0{};", b(*on)),
            Control::NoiseReduction { on } => format!("NR0{};", b(*on)),
            Control::Narrow { on } => format!("NA0{};", b(*on)),
            Control::Attenuator { on } => format!("RA0{};", b(*on)),
            Control::Preamp { on } => format!("PA0{};", b(*on)),
            Control::Power { watts } => {
                if !(5..=100).contains(watts) {
                    return Err("power must be 5–100 W".into());
                }
                format!("PC{watts:03};")
            }
        };
        if !is_allowed(&c) {
            return Err(format!("refused by allow-list: {c}"));
        }
        Ok(c)
    }

    /// Changes that are safe while the lock is on: the lock itself.
    pub fn bypasses_lock(&self) -> bool {
        matches!(self, Control::Lock { .. })
    }
}

// ------------------------------------------------------------------ decoding

#[derive(Debug)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn bad(what: &str, r: &str) -> DecodeError {
    DecodeError(format!("unexpected {what} reply: {r:?}"))
}

/// Strip the echo prefix and terminator: ("FA", "FA014074000;") → "014074000".
pub fn payload<'a>(prefix: &str, reply: &'a str) -> Result<&'a str, DecodeError> {
    reply
        .strip_prefix(prefix)
        .and_then(|r| r.strip_suffix(';'))
        .ok_or_else(|| bad(prefix, reply))
}

pub fn decode_freq(prefix: &str, reply: &str) -> Result<u64, DecodeError> {
    let p = payload(prefix, reply)?;
    digits(p, 9).ok_or_else(|| bad(prefix, reply))
}

/// `MD0x;` → mode.
pub fn decode_mode(reply: &str) -> Result<Mode, DecodeError> {
    let p = payload("MD0", reply)?;
    p.chars().next().and_then(Mode::from_code).ok_or_else(|| bad("MD", reply))
}

/// Three-digit level after a fixed prefix (`SM0nnn;`, `RM5nnn;`). Some
/// newcat radios add three more digits to RM answers; those are ignored.
pub fn decode_level(prefix: &str, reply: &str) -> Result<u8, DecodeError> {
    let p = payload(prefix, reply)?;
    let v = p.get(..3).and_then(|d| digits(d, 3)).ok_or_else(|| bad(prefix, reply))?;
    Ok(v.min(255) as u8)
}

/// Single flag digit after a prefix (`LK1;`, `NB01;` with prefix "NB0").
pub fn decode_flag(prefix: &str, reply: &str) -> Result<char, DecodeError> {
    let p = payload(prefix, reply)?;
    p.chars().next().ok_or_else(|| bad(prefix, reply))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// "001".."099", "P1L".., or "EMG"; meaningful in memory mode.
    pub channel: String,
    pub hz: u64,
    /// Clarifier offset in Hz, signed.
    pub clar_hz: i32,
    pub clar_on: bool,
    pub mode: Option<Mode>,
    /// 0 VFO, 1 memory, 2 memory tune, 5 PMS.
    pub vfo_mem: char,
    pub ctcss: char,
    pub shift: char,
}

impl Info {
    pub fn vfo_label(&self) -> &'static str {
        match self.vfo_mem {
            '0' => "VFO",
            '1' => "MEM",
            '2' => "MT",
            '3' => "QMB",
            '5' => "PMS",
            _ => "VFO",
        }
    }
}

/// `IF` answer: IF + ch(3) + freq(9) + ±offset(5) + clar(1) + 0 + mode(1)
/// + vfo/mem(1) + ctcss(1) + 00 + shift(1) + `;`  (28 characters).
pub fn decode_if(reply: &str) -> Result<Info, DecodeError> {
    let p = payload("IF", reply)?;
    if p.len() != 25 || !p.is_ascii() {
        return Err(bad("IF", reply));
    }
    let hz = digits(&p[3..12], 9).ok_or_else(|| bad("IF", reply))?;
    let sign = &p[12..13];
    let off = digits(&p[13..17], 4).ok_or_else(|| bad("IF", reply))? as i32;
    let ch = |i: usize| p.as_bytes()[i] as char;
    Ok(Info {
        channel: p[0..3].to_string(),
        hz,
        clar_hz: if sign == "-" { -off } else { off },
        clar_on: ch(17) == '1',
        mode: Mode::from_code(ch(19)),
        vfo_mem: ch(20),
        ctcss: ch(21),
        shift: ch(24),
    })
}

pub fn encode_if(i: &Info) -> String {
    format!(
        "IF{}{:09}{}{:04}{}0{}{}{}00{};",
        i.channel,
        i.hz,
        if i.clar_hz < 0 { '-' } else { '+' },
        i.clar_hz.unsigned_abs(),
        if i.clar_on { '1' } else { '0' },
        i.mode.map(Mode::code).unwrap_or('2'),
        i.vfo_mem,
        i.ctcss,
        i.shift
    )
}

pub fn agc_label(c: char) -> &'static str {
    match c {
        '0' => "OFF",
        '1' => "FAST",
        '2' => "MID",
        '3' => "SLOW",
        '4' => "AUTO-F",
        '5' => "AUTO-M",
        '6' => "AUTO-S",
        _ => "?",
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transmit_and_destructive_commands_are_never_allowed() {
        for c in [
            "TX1;", "TX2;", "TX0;", "MX1;", "KY0;", "KYhello;", "KM1abc;", "AC002;", "AC001;", "VX1;", "EX0506 3;",
            "EX05063;", "MW;", "MA;", "MC001;", "ZI;", "AI1;", "DA01010101;", "CN00000;", "CT01;", "FA;X", "FA1;",
            "FA100000000;", "FA000000001;", "MD0A;", "MD0E;", "MD1C;", "PC000;", "PC101;", "SV1;", "BU1;", "RS;", "",
            ";", "FA014074000", "FA014074000;;", "LK2;", "NB02;", "ST2;",
        ] {
            assert!(!is_allowed(c), "{c} must be refused");
        }
    }

    #[test]
    fn reads_and_validated_sets_are_allowed() {
        for c in READS {
            assert!(is_allowed(c), "{c}");
        }
        for c in ["AI0;", "FA014074000;", "FB007074000;", "MD0C;", "MD01;", "BU0;", "BD0;", "SV;", "AB;", "ST1;",
            "LK0;", "NB01;", "NR00;", "NA01;", "RA01;", "PA00;", "PC050;", "FS1;"]
        {
            assert!(is_allowed(c), "{c}");
        }
    }

    #[test]
    fn every_control_maps_to_an_allowed_command() {
        let cases = vec![
            Control::SetFreq { hz: 14_074_000 },
            Control::SetFreqB { hz: 7_074_000 },
            Control::SetMode { mode: "D-U".into() },
            Control::BandUp,
            Control::BandDown,
            Control::SwapAb,
            Control::CopyAToB,
            Control::Split { on: true },
            Control::Lock { on: false },
            Control::Fast { on: true },
            Control::NoiseBlanker { on: true },
            Control::NoiseReduction { on: true },
            Control::Narrow { on: false },
            Control::Attenuator { on: true },
            Control::Preamp { on: true },
            Control::Power { watts: 50 },
        ];
        for c in cases {
            let s = c.command().unwrap();
            assert!(is_allowed(&s), "{c:?} -> {s}");
        }
        assert_eq!(Control::SetFreq { hz: 14_074_000 }.command().unwrap(), "FA014074000;");
        assert_eq!(Control::SetMode { mode: "CWL".into() }.command().unwrap(), "MD07;");
        assert!(Control::SetFreq { hz: 60_000_000 }.command().is_err());
        assert!(Control::Power { watts: 0 }.command().is_err());
        assert!(Control::SetMode { mode: "C4FM".into() }.command().is_err());
    }

    #[test]
    fn decodes_if() {
        let i = decode_if("IF001014074000+012010C00000;").unwrap();
        assert_eq!(i.hz, 14_074_000);
        assert_eq!(i.clar_hz, 120);
        assert!(i.clar_on);
        assert_eq!(i.mode, Some(Mode::DataU));
        assert_eq!(i.vfo_mem, '0');
        assert_eq!(encode_if(&i), "IF001014074000+012010C00000;");
        let j = decode_if("IF012007150000-005000210001;").unwrap();
        assert_eq!(j.clar_hz, -50);
        assert!(!j.clar_on);
        assert_eq!(j.mode, Some(Mode::Usb));
        assert_eq!(j.vfo_label(), "MEM");
        assert_eq!(j.shift, '1');
        assert!(decode_if("IF001;").is_err());
    }

    #[test]
    fn decodes_levels_and_modes() {
        assert_eq!(decode_freq("FA", "FA014074000;").unwrap(), 14_074_000);
        assert_eq!(decode_mode("MD02;").unwrap(), Mode::Usb);
        assert_eq!(decode_level("SM0", "SM0130;").unwrap(), 130);
        assert_eq!(decode_level("RM5", "RM5100000;").unwrap(), 100);
        assert!(decode_level("SM0", "?;").is_err());
        assert_eq!(decode_flag("NB0", "NB01;").unwrap(), '1');
    }

    #[test]
    fn meter_calibration() {
        assert_eq!(s_label(0), "S0");
        assert_eq!(s_label(130), "S9");
        assert_eq!(s_label(172), "S9+20");
        assert_eq!(s_label(65), "S5");
        assert!((po_watts(250) - 140.0).abs() < 0.01);
        assert!((swr_ratio(0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn mode_labels_round_trip() {
        for m in ALL_MODES {
            assert_eq!(Mode::from_label(m.lcd_label()), Some(m));
            assert_eq!(Mode::from_code(m.code()), Some(m));
        }
    }
}
