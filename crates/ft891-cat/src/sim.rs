//! A simulated FT-891 that answers CAT commands like the real radio, for the
//! demo mode and tests. It runs an FT8-style cycle on 20 m: 15 s periods,
//! transmitting every other period, with a moving S-meter in between.

use crate::cat::{self, Info, Mode};
use std::time::Instant;

pub struct SimRadio {
    pub vfo_a: u64,
    pub vfo_b: u64,
    pub mode: Mode,
    pub mode_b: Mode,
    pub split: bool,
    pub lock: bool,
    pub fast: bool,
    pub nb: bool,
    pub nr: bool,
    pub narrow: bool,
    pub att: bool,
    pub amp: bool,
    pub prc: bool,
    pub mon: bool,
    pub vox: bool,
    pub tuner: bool,
    pub power: u32,
    pub meter_sel: char,
    pub clar_hz: i32,
    pub clar_on: bool,
    /// Simulated transmit cycle on/off (demo shows both RX and TX).
    pub auto_tx: bool,
    /// Force transmit state (tests).
    pub force_tx: Option<bool>,
    /// Every command the simulator received, for tests.
    pub log: Vec<String>,
    start: Instant,
}

impl Default for SimRadio {
    fn default() -> Self {
        Self::new()
    }
}

const BANDS: [u64; 11] = [
    1_840_000, 3_573_000, 7_074_000, 10_136_000, 14_074_000, 18_100_000, 21_074_000, 24_915_000, 28_074_000,
    50_313_000, 5_357_000,
];

impl SimRadio {
    pub fn new() -> Self {
        Self {
            vfo_a: 14_074_000,
            vfo_b: 14_076_500,
            mode: Mode::DataU,
            mode_b: Mode::DataU,
            split: false,
            lock: false,
            fast: false,
            nb: false,
            nr: true,
            narrow: false,
            att: false,
            amp: true,
            prc: false,
            mon: false,
            vox: false,
            tuner: true,
            power: 50,
            meter_sel: '2',
            clar_hz: 0,
            clar_on: false,
            auto_tx: true,
            force_tx: None,
            log: Vec::new(),
            start: Instant::now(),
        }
    }

    fn t(&self) -> f32 {
        self.start.elapsed().as_secs_f32()
    }

    pub fn transmitting(&self) -> bool {
        if let Some(f) = self.force_tx {
            return f;
        }
        if !self.auto_tx {
            return false;
        }
        // 15 s FT8 periods: TX from 0.5 s to 13.1 s of every second period.
        let t = self.t() % 30.0;
        t > 15.5 && t < 28.1
    }

    fn s_raw(&self) -> u8 {
        let t = self.t();
        let slow = (t * 0.35).sin() * 0.5 + 0.5;
        let fast = ((t * 7.3).sin() * (t * 3.1).cos()).abs();
        let mut v = 55.0 + slow * 70.0 + fast * 35.0;
        if self.att {
            v -= 30.0;
        }
        if !self.amp {
            v -= 15.0;
        }
        v.clamp(0.0, 255.0) as u8
    }

    fn tx_level(&self, sel: char) -> u8 {
        let t = self.t();
        let wob = ((t * 9.0).sin() * 0.5 + 0.5) as f64;
        let po = 75.0 + (self.power as f64) * 1.0 + wob * 6.0; // ~50 W at 50 %
        let v = match sel {
            '3' => 40.0 + wob * 60.0,  // COMP
            '4' => 20.0 + wob * 40.0,  // ALC
            '5' => po,                 // PO
            '6' => 28.0 + wob * 4.0,   // SWR ~1.2
            '7' => 90.0 + wob * 20.0,  // ID
            _ => 0.0,
        };
        v.clamp(0.0, 255.0) as u8
    }

    fn info(&self) -> Info {
        Info {
            channel: "001".into(),
            hz: self.vfo_a,
            clar_hz: self.clar_hz,
            clar_on: self.clar_on,
            mode: Some(self.mode),
            vfo_mem: '0',
            ctcss: '0',
            shift: '0',
        }
    }

    fn band_step(&mut self, up: bool) {
        let mut sorted = BANDS;
        sorted.sort();
        let cur = self.vfo_a;
        let next = if up {
            sorted.iter().copied().find(|&f| f > cur + 100_000).unwrap_or(sorted[0])
        } else {
            sorted.iter().rev().copied().find(|&f| f + 100_000 < cur).unwrap_or(sorted[sorted.len() - 1])
        };
        self.vfo_a = next;
        self.mode = if next < 10_000_000 && next != 5_357_000 { Mode::Lsb } else { Mode::Usb };
        if next != 5_357_000 && [1_840_000, 3_573_000, 7_074_000].contains(&next) {
            self.mode = Mode::DataU;
        }
    }

    /// Answer one complete command (with its `;`). Returns "" for set
    /// commands, like the radio, and "?;" for anything it doesn't know.
    pub fn respond(&mut self, cmd: &str) -> String {
        self.log.push(cmd.to_string());
        let f = |on: bool| if on { '1' } else { '0' };
        let tx = self.transmitting();
        match cmd {
            "ID;" => format!("ID{};", cat::RADIO_ID),
            "IF;" => cat::encode_if(&self.info()),
            "OI;" => {
                let mut i = self.info();
                i.hz = self.vfo_b;
                i.mode = Some(self.mode_b);
                i.clar_on = false;
                i.clar_hz = 0;
                cat::encode_if(&i).replacen("IF", "OI", 1)
            }
            "FA;" => format!("FA{:09};", self.vfo_a),
            "FB;" => format!("FB{:09};", self.vfo_b),
            "MD0;" => format!("MD0{};", self.mode.code()),
            "SM0;" => format!("SM0{:03};", if tx { 0 } else { self.s_raw() }),
            "TX;" => format!("TX{};", if tx { '2' } else { '0' }),
            "BY;" => format!("BY{}0;", f(!tx && self.s_raw() > 110)),
            "MS;" => format!("MS{};", self.meter_sel),
            "ST;" => format!("ST{};", f(self.split)),
            "LK;" => format!("LK{};", f(self.lock)),
            "VX;" => format!("VX{};", f(self.vox)),
            "FS;" => format!("FS{};", f(self.fast)),
            "PA0;" => format!("PA0{};", f(self.amp)),
            "RA0;" => format!("RA0{};", f(self.att)),
            "NA0;" => format!("NA0{};", f(self.narrow)),
            "NB0;" => format!("NB0{};", f(self.nb)),
            "NR0;" => format!("NR0{};", f(self.nr)),
            "BC0;" => "BC00;".into(),
            "PR0;" => format!("PR0{};", f(self.prc)),
            "ML0;" => format!("ML0{:03};", if self.mon { 1 } else { 0 }),
            "AC;" => format!("AC00{};", f(self.tuner)),
            "GT0;" => "GT04;".into(),
            "PC;" => format!("PC{:03};", self.power),
            "SH0;" => "SH0000;".into(),
            "CT0;" => "CT00;".into(),
            "KS;" => "KS020;".into(),
            "AG0;" => "AG0128;".into(),
            "RG0;" => "RG0255;".into(),
            "SQ0;" => "SQ0000;".into(),
            "RM3;" | "RM4;" | "RM5;" | "RM6;" | "RM7;" => {
                let sel = cmd.as_bytes()[2] as char;
                format!("RM{sel}{:03}000;", if tx { self.tx_level(sel) } else { 0 })
            }
            _ => self.set(cmd),
        }
    }

    fn set(&mut self, cmd: &str) -> String {
        let Some(body) = cmd.strip_suffix(';') else { return "?;".into() };
        if body.len() < 2 {
            return "?;".into();
        }
        let (op, p) = body.split_at(2);
        let on = p.ends_with('1');
        let locked_out = self.lock && !matches!(op, "LK" | "AI");
        if locked_out {
            // The real radio's LOCK only locks the dial; we mirror that for tuning.
            if matches!(op, "FA" | "FB") {
                return String::new();
            }
        }
        match op {
            "AI" => {}
            "FA" => match p.parse() {
                Ok(hz) => self.vfo_a = hz,
                Err(_) => return "?;".into(),
            },
            "FB" => match p.parse() {
                Ok(hz) => self.vfo_b = hz,
                Err(_) => return "?;".into(),
            },
            "MD" => match p.chars().nth(1).and_then(Mode::from_code) {
                Some(m) => self.mode = m,
                None => return "?;".into(),
            },
            "BU" => self.band_step(true),
            "BD" => self.band_step(false),
            "SV" => {
                std::mem::swap(&mut self.vfo_a, &mut self.vfo_b);
                std::mem::swap(&mut self.mode, &mut self.mode_b);
            }
            "AB" => {
                self.vfo_b = self.vfo_a;
                self.mode_b = self.mode;
            }
            "BA" => {
                self.vfo_a = self.vfo_b;
                self.mode = self.mode_b;
            }
            "ST" => self.split = on,
            "LK" => self.lock = on,
            "FS" => self.fast = on,
            "NB" => self.nb = on,
            "NR" => self.nr = on,
            "NA" => self.narrow = on,
            "RA" => self.att = on,
            "PA" => self.amp = on,
            "PC" => match p.parse() {
                Ok(w) => self.power = w,
                Err(_) => return "?;".into(),
            },
            _ => return "?;".into(),
        }
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_reads_in_radio_format() {
        let mut r = SimRadio::new();
        r.force_tx = Some(false);
        assert_eq!(r.respond("FA;"), "FA014074000;");
        assert_eq!(r.respond("MD0;"), "MD0C;");
        assert_eq!(r.respond("ID;"), "ID0650;");
        let i = cat::decode_if(&r.respond("IF;")).unwrap();
        assert_eq!(i.hz, 14_074_000);
        assert!(cat::decode_level("SM0", &r.respond("SM0;")).is_ok());
        assert_eq!(r.respond("XX;"), "?;");
    }

    #[test]
    fn applies_sets() {
        let mut r = SimRadio::new();
        assert_eq!(r.respond("FA007074000;"), "");
        assert_eq!(r.vfo_a, 7_074_000);
        r.respond("SV;");
        assert_eq!(r.vfo_a, 14_076_500);
        r.respond("NB01;");
        assert!(r.nb);
        r.respond("LK1;");
        r.respond("FA003573000;");
        assert_eq!(r.vfo_a, 14_076_500, "dial lock holds the frequency");
    }
}
