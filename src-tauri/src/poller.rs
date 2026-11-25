//! Polling loops. Each runs on its own thread until `stop` is set, pushes
//! RigState snapshots to `emit` (about 30 per second at most) and carries out
//! control requests that arrive on `controls` between polls.
//!
//! Control requests are refused while the radio is transmitting, and tuning
//! requests are refused while the radio's dial LOCK is on.

use crate::cat::{self, Control, Meter, Mode};
use crate::state::{Link, RigState};
use crate::transport::{CatPort, DemoCat, SerialCat, StopBits};
use serde::Deserialize;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase", tag = "kind")]
pub enum Source {
    Serial {
        port: String,
        baud: u32,
        #[serde(default)]
        stop_bits: StopBits,
        #[serde(default = "yes")]
        rts_cts: bool,
    },
    Rigctld {
        host: String,
        port: u16,
    },
    Demo,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct MonitorConfig {
    pub source: Source,
    /// Allow control requests at all (the display can be made view-only).
    #[serde(default = "yes")]
    pub allow_control: bool,
}

const EMIT_EVERY: Duration = Duration::from_millis(33);
const MID_EVERY: Duration = Duration::from_millis(250);
const SLOW_EVERY: Duration = Duration::from_millis(1000);
const CYCLE_MIN: Duration = Duration::from_millis(30);
const RETRY_SECS: u32 = 5;

struct Rate {
    count: u32,
    since: Instant,
    value: f32,
}

impl Rate {
    fn new() -> Self {
        Self { count: 0, since: Instant::now(), value: 0.0 }
    }
    fn tick(&mut self) -> f32 {
        self.count += 1;
        let dt = self.since.elapsed().as_secs_f32();
        if dt >= 1.0 {
            self.value = self.count as f32 / dt;
            self.count = 0;
            self.since = Instant::now();
        }
        self.value
    }
}

fn sleep_unless_stopped(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end && !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn begin_attempt(st: &mut RigState, what: &str, emit: &impl Fn(&RigState)) {
    st.attempt += 1;
    st.retry_in = None;
    st.link = Link::Connecting;
    st.message = if st.attempt > 1 { format!("{what} (attempt {})…", st.attempt) } else { format!("{what}…") };
    emit(st);
}

fn link_up(st: &mut RigState, msg: &str) {
    st.link = Link::Up;
    st.message = msg.into();
    st.error = None;
    st.attempt = 0;
    st.retry_in = None;
}

fn fail_and_wait(st: &mut RigState, error: String, stop: &AtomicBool, emit: &impl Fn(&RigState)) {
    st.link = Link::Lost;
    st.error = Some(error);
    for left in (1..=RETRY_SECS).rev() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        st.retry_in = Some(left);
        st.message = format!("Next attempt in {left} s");
        emit(st);
        sleep_unless_stopped(stop, Duration::from_secs(1));
    }
    st.retry_in = None;
}

/// Tuning-type requests the dial LOCK should block.
fn is_tuning(c: &Control) -> bool {
    matches!(
        c,
        Control::SetFreq { .. }
            | Control::SetFreqB { .. }
            | Control::SetMode { .. }
            | Control::BandUp
            | Control::BandDown
            | Control::SwapAb
            | Control::CopyAToB
    )
}

/// Common checks before any control request reaches a radio.
fn precheck(c: &Control, st: &RigState, allow: bool) -> Result<(), String> {
    if !allow {
        return Err("Controls are turned off in Settings".into());
    }
    if st.link != Link::Up {
        return Err("Not connected".into());
    }
    if st.tx {
        return Err("Refused: the radio is transmitting".into());
    }
    if st.lock == Some(true) && is_tuning(c) && !c.bypasses_lock() {
        return Err("Refused: the radio's dial LOCK is on".into());
    }
    Ok(())
}

pub fn run(cfg: MonitorConfig, stop: Arc<AtomicBool>, controls: Receiver<Control>, emit: impl Fn(&RigState)) {
    match cfg.source.clone() {
        Source::Rigctld { host, port } => run_rigctld(&cfg, &host, port, &stop, &controls, &emit),
        Source::Serial { port, baud, stop_bits, rts_cts } => {
            let label = format!("Serial {port} @ {baud}");
            run_cat(&cfg, &label, &stop, &controls, &emit, || {
                SerialCat::open(&port, baud, stop_bits, rts_cts).map(|p| Box::new(p) as Box<dyn CatPort>)
            })
        }
        Source::Demo => run_cat(&cfg, "Demo (simulated FT-891)", &stop, &controls, &emit, || {
            Ok(Box::new(DemoCat::new()) as Box<dyn CatPort>)
        }),
    }
    let st = RigState { link: Link::Stopped, message: "Stopped".into(), ..Default::default() };
    emit(&st);
}

// ---------------------------------------------------------------- direct CAT

fn bad_data(e: cat::DecodeError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.0)
}

fn flag(port: &mut Box<dyn CatPort>, cmd: &str, prefix: &str) -> io::Result<bool> {
    let r = port.query(cmd)?;
    Ok(cat::decode_flag(prefix, &r).map_err(bad_data)? != '0')
}

/// Every cycle: transmit state, busy, the active meter, VFO-A frequency.
fn poll_fast(port: &mut Box<dyn CatPort>, st: &mut RigState, meter: Option<Meter>) -> io::Result<()> {
    let tx = port.query("TX;")?;
    // TX0 = receive; TX1 = CAT TX on; TX2 = radio TX on (PTT/mic/data).
    st.tx = cat::decode_flag("TX", &tx).map_err(bad_data)? != '0';
    if st.tx {
        let m = meter.unwrap_or(Meter::Po);
        let sel = m.rm_selector();
        let r = port.query(&format!("RM{sel};"))?;
        st.meter_raw = Some(cat::decode_level(&format!("RM{sel}"), &r).map_err(bad_data)?);
        let po = if m == Meter::Po {
            st.meter_raw.unwrap()
        } else {
            cat::decode_level("RM5", &port.query("RM5;")?).map_err(bad_data)?
        };
        st.po_watts = Some(cat::po_watts(po));
        st.busy = Some(false);
    } else {
        let s = cat::decode_level("SM0", &port.query("SM0;")?).map_err(bad_data)?;
        st.s_raw = Some(s);
        st.s_label = Some(cat::s_label(s));
        st.busy = Some(flag(port, "BY;", "BY")?);
        st.meter_raw = Some(0);
        st.po_watts = None;
    }
    let hz = cat::decode_freq("FA", &port.query("FA;")?).map_err(bad_data)?;
    st.freq_hz = Some(hz);
    st.band = Some(cat::band_name(hz).into());
    Ok(())
}

/// About 4× a second: IF (mode, memory, clarifier), VFO-B, meter, split, lock.
fn poll_mid(port: &mut Box<dyn CatPort>, st: &mut RigState) -> io::Result<Option<Meter>> {
    let info = cat::decode_if(&port.query("IF;")?).map_err(bad_data)?;
    st.mode = info.mode.map(|m| m.lcd_label().to_string());
    st.vfo_mem = Some(info.vfo_label().into());
    st.mem_channel = Some(info.channel.trim_start_matches('0').to_string());
    st.clar_on = Some(info.clar_on);
    st.clar_hz = Some(info.clar_hz);
    // OI: the same layout as IF, for VFO-B (frequency and mode).
    let oi = port.query("OI;")?;
    let b = cat::decode_if(&oi.replacen("OI", "IF", 1)).map_err(bad_data)?;
    st.freq_b_hz = Some(b.hz);
    st.mode_b = b.mode.map(|m| m.lcd_label().to_string());
    let ms = cat::decode_flag("MS", &port.query("MS;")?).map_err(bad_data)?;
    let meter = Meter::from_ms(ms);
    st.meter = meter.map(|m| m.label().to_string());
    st.split = Some(flag(port, "ST;", "ST")?);
    st.lock = Some(flag(port, "LK;", "LK")?);
    if st.tx {
        // SWR shown numerically in the footer while transmitting.
        let r = port.query("RM6;")?;
        st.swr = Some(cat::swr_ratio(cat::decode_level("RM6", &r).map_err(bad_data)?));
    } else {
        st.swr = None;
    }
    Ok(meter)
}

/// Once a second: the LCD indicators and footer extras.
fn poll_slow(port: &mut Box<dyn CatPort>, st: &mut RigState) -> io::Result<()> {
    st.vox = Some(flag(port, "VX;", "VX")?);
    st.ipo = Some(!flag(port, "PA0;", "PA0")?);
    st.att = Some(flag(port, "RA0;", "RA0")?);
    st.nar = Some(flag(port, "NA0;", "NA0")?);
    st.nb = Some(flag(port, "NB0;", "NB0")?);
    st.dnr = Some(flag(port, "NR0;", "NR0")?);
    st.dnf = Some(flag(port, "BC0;", "BC0")?);
    st.prc = Some(flag(port, "PR0;", "PR0")?);
    st.fst = Some(flag(port, "FS;", "FS")?);
    // ML0 answers ML0nnn (000 off, 001 on).
    let ml = cat::payload("ML0", &port.query("ML0;")?).map_err(bad_data)?.to_string();
    st.mon = Some(ml.trim_start_matches('0') != "");
    // AC answers AC00n: 0 tuner off, 1 on, 2 tuning.
    let ac = cat::payload("AC", &port.query("AC;")?).map_err(bad_data)?.to_string();
    st.tnr = Some(ac.ends_with('1') || ac.ends_with('2'));
    let gt = cat::decode_flag("GT0", &port.query("GT0;")?).map_err(bad_data)?;
    st.agc = Some(cat::agc_label(gt).into());
    let pc = cat::payload("PC", &port.query("PC;")?).map_err(bad_data)?.to_string();
    st.power_w = pc.parse().ok();
    Ok(())
}

fn run_cat(
    cfg: &MonitorConfig,
    label: &str,
    stop: &AtomicBool,
    controls: &Receiver<Control>,
    emit: &impl Fn(&RigState),
    open: impl Fn() -> io::Result<Box<dyn CatPort>>,
) {
    let mut st = RigState { source: label.to_string(), can_control: cfg.allow_control, ..Default::default() };
    while !stop.load(Ordering::Relaxed) {
        begin_attempt(&mut st, "Connecting", emit);
        let mut port = match open() {
            Ok(p) => p,
            Err(e) => {
                fail_and_wait(
                    &mut st,
                    format!("Cannot open the serial port: {e}. Check the USB cable, that the radio is on, and that no other program is using the port."),
                    stop,
                    emit,
                );
                continue;
            }
        };

        // Identify the radio and make sure auto-information is off, so every
        // answer belongs to the command that asked for it.
        let hello = port.query("ID;").and_then(|id| {
            if id != format!("ID{};", cat::RADIO_ID) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("this doesn't look like an FT-891 (it answered {id})"),
                ));
            }
            port.send("AI0;")
        });
        if let Err(e) = hello {
            drop(port);
            let hint = if e.kind() == io::ErrorKind::TimedOut {
                "No answer from the radio. Check that you picked the Enhanced COM port (not the Standard one), and that the baud rate matches menu 05-06 CAT RATE."
            } else {
                "Check that the radio on this port is an FT-891."
            };
            fail_and_wait(&mut st, format!("{e}. {hint}"), stop, emit);
            continue;
        }

        let mut lost: Option<String> = None;
        let mut last_mid = Instant::now() - MID_EVERY;
        let mut last_slow = Instant::now() - SLOW_EVERY;
        let mut last_emit = Instant::now() - EMIT_EVERY;
        let mut note_until: Option<Instant> = None;
        let mut rate = Rate::new();
        let mut fails = 0u32;
        let mut meter: Option<Meter> = None;

        while !stop.load(Ordering::Relaxed) {
            let cycle_start = Instant::now();

            // Control requests first, so the display shows the result quickly.
            while let Ok(c) = controls.try_recv() {
                let outcome = precheck(&c, &st, cfg.allow_control)
                    .and_then(|_| c.command())
                    .and_then(|cmd| port.send(&cmd).map_err(|e| format!("Refused: {e}")));
                match outcome {
                    Ok(()) => {
                        // Read everything again right away.
                        last_mid = Instant::now() - MID_EVERY;
                        last_slow = Instant::now() - SLOW_EVERY;
                    }
                    Err(msg) => {
                        st.control_note = Some(msg);
                        note_until = Some(Instant::now() + Duration::from_secs(3));
                        last_emit = Instant::now() - EMIT_EVERY;
                    }
                }
            }
            if note_until.is_some_and(|t| Instant::now() > t) {
                st.control_note = None;
                note_until = None;
            }

            let mut result = poll_fast(&mut port, &mut st, meter);
            if result.is_ok() && last_mid.elapsed() >= MID_EVERY {
                result = poll_mid(&mut port, &mut st).map(|m| meter = m);
                last_mid = Instant::now();
            }
            if result.is_ok() && last_slow.elapsed() >= SLOW_EVERY {
                result = poll_slow(&mut port, &mut st);
                last_slow = Instant::now();
            }
            match result {
                Ok(()) => {
                    fails = 0;
                    if st.link != Link::Up {
                        link_up(&mut st, "Connected");
                        last_emit = Instant::now() - EMIT_EVERY;
                    }
                    st.polls_per_sec = rate.tick();
                }
                Err(e) => {
                    fails += 1;
                    st.errors += 1;
                    if fails >= 3 {
                        lost = Some(format!(
                            "No reply from the radio ({e}). Check the cable, that the radio is on, and that the baud rate matches menu 05-06 CAT RATE."
                        ));
                        break;
                    }
                }
            }
            if last_emit.elapsed() >= EMIT_EVERY {
                emit(&st);
                last_emit = Instant::now();
            }
            if let Some(rest) = CYCLE_MIN.checked_sub(cycle_start.elapsed()) {
                std::thread::sleep(rest);
            }
        }
        drop(port);
        if let Some(err) = lost {
            fail_and_wait(&mut st, err, stop, emit);
        }
    }
}

// ---------------------------------------------------------------- rigctld

struct Rigctld {
    r: BufReader<TcpStream>,
    w: TcpStream,
}

impl Rigctld {
    fn connect(host: &str, port: u16) -> io::Result<Self> {
        let addr = (host, port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::other("cannot resolve host"))?;
        let s = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        s.set_read_timeout(Some(Duration::from_millis(1500)))?;
        s.set_nodelay(true)?;
        Ok(Self { r: BufReader::new(s.try_clone()?), w: s })
    }

    fn line(&mut self) -> io::Result<String> {
        let mut l = String::new();
        if self.r.read_line(&mut l)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "rigctld closed the connection"));
        }
        Ok(l.trim().to_string())
    }

    /// Read command: `lines` reply lines. `Ok(None)`: not supported.
    fn cmd(&mut self, c: &str, lines: usize) -> io::Result<Option<Vec<String>>> {
        self.w.write_all(format!("{c}\n").as_bytes())?;
        let mut out = Vec::with_capacity(lines);
        for _ in 0..lines {
            let l = self.line()?;
            if l.starts_with("RPRT") {
                return Ok(None);
            }
            out.push(l);
        }
        Ok(Some(out))
    }

    fn num(&mut self, c: &str) -> io::Result<Option<f64>> {
        Ok(self.cmd(c, 1)?.and_then(|v| v[0].parse::<f64>().ok()))
    }

    fn flag(&mut self, c: &str) -> io::Result<Option<bool>> {
        Ok(self.num(c)?.map(|v| v != 0.0))
    }

    /// Set command: rigctld answers `RPRT 0` on success.
    fn set(&mut self, c: &str) -> io::Result<Result<(), String>> {
        self.w.write_all(format!("{c}\n").as_bytes())?;
        let l = self.line()?;
        Ok(if l == "RPRT 0" { Ok(()) } else { Err(format!("rigctld refused \"{c}\" ({l})")) })
    }
}

fn hamlib_to_lcd(m: &str) -> String {
    match m {
        "LSB" => "LSB",
        "USB" => "USB",
        "CW" => "CWU",
        "CWR" => "CWL",
        "AM" => "AM",
        "AMN" => "AM-N",
        "FM" => "FM",
        "FMN" => "FM-N",
        "RTTY" => "R-L",
        "RTTYR" => "R-U",
        "PKTLSB" => "D-L",
        "PKTUSB" => "D-U",
        "PKTFM" => "D-FM",
        other => other,
    }
    .into()
}

fn lcd_to_hamlib(m: Mode) -> &'static str {
    match m {
        Mode::Lsb => "LSB",
        Mode::Usb => "USB",
        Mode::CwU => "CW",
        Mode::CwL => "CWR",
        Mode::Am => "AM",
        Mode::AmN => "AMN",
        Mode::Fm => "FM",
        Mode::FmN => "FMN",
        Mode::RttyL => "RTTY",
        Mode::RttyU => "RTTYR",
        Mode::DataL => "PKTLSB",
        Mode::DataU => "PKTUSB",
        Mode::DataFm => "PKTFM",
    }
}

/// The rigctld command for a control request. Every request is still
/// validated through `Control::command` first (same ranges as direct CAT).
fn rigctld_command(c: &Control) -> Result<String, String> {
    c.command()?;
    let on = |b: bool| if b { 1 } else { 0 };
    Ok(match c {
        Control::SetFreq { hz } => format!("F {hz}"),
        Control::SetMode { mode } => format!("M {} 0", lcd_to_hamlib(Mode::from_label(mode).unwrap())),
        Control::BandUp => "G BAND_UP".into(),
        Control::BandDown => "G BAND_DOWN".into(),
        Control::SwapAb => "G XCHG".into(),
        Control::CopyAToB => "G CPY".into(),
        Control::Split { on: b } => format!("S {} VFOB", on(*b)),
        Control::Lock { on: b } => format!("U LOCK {}", on(*b)),
        Control::NoiseBlanker { on: b } => format!("U NB {}", on(*b)),
        Control::NoiseReduction { on: b } => format!("U NR {}", on(*b)),
        Control::Power { watts } => format!("L RFPOWER {:.2}", *watts as f32 / 100.0),
        _ => return Err("Not available through rigctld; use a direct serial connection".into()),
    })
}

fn run_rigctld(
    cfg: &MonitorConfig,
    host: &str,
    port: u16,
    stop: &AtomicBool,
    controls: &Receiver<Control>,
    emit: &impl Fn(&RigState),
) {
    let mut st =
        RigState { source: format!("rigctld {host}:{port}"), can_control: cfg.allow_control, ..Default::default() };
    while !stop.load(Ordering::Relaxed) {
        begin_attempt(&mut st, "Connecting to rigctld", emit);
        let mut rc = match Rigctld::connect(host, port) {
            Ok(r) => r,
            Err(e) => {
                fail_and_wait(
                    &mut st,
                    format!("Cannot reach rigctld: {e}. Start it with: rigctld -m 1036 -r <port> -s 4800"),
                    stop,
                    emit,
                );
                continue;
            }
        };
        let mut rate = Rate::new();
        let mut last_slow = Instant::now() - Duration::from_secs(10);
        let mut note_until: Option<Instant> = None;

        let result: io::Result<()> = (|| {
            while !stop.load(Ordering::Relaxed) {
                while let Ok(c) = controls.try_recv() {
                    let outcome = match precheck(&c, &st, cfg.allow_control).and_then(|_| rigctld_command(&c)) {
                        Ok(cmd) => rc.set(&cmd)?,
                        Err(e) => Err(e),
                    };
                    match outcome {
                        Ok(()) => last_slow = Instant::now() - Duration::from_secs(10),
                        Err(msg) => {
                            st.control_note = Some(msg);
                            note_until = Some(Instant::now() + Duration::from_secs(3));
                        }
                    }
                }
                if note_until.is_some_and(|t| Instant::now() > t) {
                    st.control_note = None;
                    note_until = None;
                }

                if let Some(f) = rc.num("f")? {
                    let hz = f as u64;
                    st.freq_hz = Some(hz);
                    st.band = Some(cat::band_name(hz).into());
                }
                if let Some(v) = rc.cmd("m", 2)? {
                    st.mode = Some(hamlib_to_lcd(&v[0]));
                }
                st.tx = rc.flag("t")?.unwrap_or(false);
                if st.tx {
                    if let Some(p) = rc.num("l RFPOWER_METER_WATTS")? {
                        st.po_watts = Some(p as f32);
                    }
                    st.swr = rc.num("l SWR")?.map(|v| v as f32);
                    st.meter = Some("PO".into());
                    // Reverse the power calibration so the bar matches direct CAT.
                    st.meter_raw = st.po_watts.map(|w| {
                        (0..=255u16).find(|&r| cat::po_watts(r as u8) >= w).unwrap_or(255) as u8
                    });
                } else {
                    if let Some(raw) = rc.num("l RAWSTR")? {
                        let s = raw.clamp(0.0, 255.0) as u8;
                        st.s_raw = Some(s);
                        st.s_label = Some(cat::s_label(s));
                    }
                    st.busy = rc.flag("\\get_dcd")?;
                    st.meter_raw = Some(0);
                }
                if last_slow.elapsed() >= Duration::from_secs(1) {
                    if let Some(v) = rc.cmd("s", 2)? {
                        st.split = Some(v[0] == "1");
                    }
                    st.lock = rc.flag("u LOCK")?;
                    st.nb = rc.flag("u NB")?;
                    st.dnr = rc.flag("u NR")?;
                    st.vox = rc.flag("u VOX")?;
                    st.tnr = rc.flag("u TUNER")?;
                    st.prc = rc.flag("u COMP")?;
                    st.mon = rc.flag("u MON")?;
                    st.att = rc.num("l ATT")?.map(|v| v > 0.0);
                    st.ipo = rc.num("l PREAMP")?.map(|v| v == 0.0);
                    st.power_w = rc.num("l RFPOWER")?.map(|v| (v * 100.0).round() as u32);
                    last_slow = Instant::now();
                }
                if st.link != Link::Up {
                    link_up(&mut st, "Connected (shared via rigctld)");
                }
                st.polls_per_sec = rate.tick();
                emit(&st);
                // Gentle: other programs share this radio.
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(())
        })();

        if let Err(e) = result {
            st.errors += 1;
            fail_and_wait(&mut st, format!("rigctld connection lost: {e}"), stop, emit);
        }
    }
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::sync::Mutex;

    fn demo(run_for: Duration, actions: Vec<(Duration, Control)>) -> Vec<RigState> {
        let stop = Arc::new(AtomicBool::new(false));
        let seen: Arc<Mutex<Vec<RigState>>> = Arc::default();
        let (tx, rx) = mpsc::channel();
        let (s2, seen2) = (stop.clone(), seen.clone());
        let h = std::thread::spawn(move || {
            let cfg = MonitorConfig { source: Source::Demo, allow_control: true };
            run(cfg, s2, rx, |st| seen2.lock().unwrap().push(st.clone()));
        });
        let start = Instant::now();
        for (at, c) in actions {
            if let Some(w) = at.checked_sub(start.elapsed()) {
                std::thread::sleep(w);
            }
            tx.send(c).unwrap();
        }
        if let Some(w) = run_for.checked_sub(start.elapsed()) {
            std::thread::sleep(w);
        }
        stop.store(true, Ordering::Relaxed);
        h.join().unwrap();
        let v = seen.lock().unwrap().clone();
        v
    }

    #[test]
    fn demo_end_to_end_produces_decoded_state() {
        let v = demo(Duration::from_millis(1500), vec![]);
        let up = v.iter().rev().find(|s| s.link == Link::Up).expect("link came up");
        assert_eq!(up.freq_hz, Some(14_074_000));
        assert_eq!(up.freq_b_hz, Some(14_076_500));
        assert_eq!(up.mode.as_deref(), Some("D-U"));
        assert_eq!(up.band.as_deref(), Some("20m"));
        assert_eq!(up.vfo_mem.as_deref(), Some("VFO"));
        assert_eq!(up.meter.as_deref(), Some("PO"));
        assert_eq!(up.tnr, Some(true));
        assert_eq!(up.dnr, Some(true));
        assert_eq!(up.ipo, Some(false));
        assert_eq!(up.agc.as_deref(), Some("AUTO-F"));
        assert_eq!(up.power_w, Some(50));
        assert!(up.polls_per_sec > 5.0, "rate {}", up.polls_per_sec);
        assert_eq!(v.last().unwrap().link, Link::Stopped);
    }

    #[test]
    fn controls_reach_the_radio_and_lock_blocks_tuning() {
        let ms = Duration::from_millis;
        let v = demo(
            ms(2600),
            vec![
                (ms(500), Control::SetFreq { hz: 7_074_000 }),
                (ms(500), Control::NoiseBlanker { on: true }),
                (ms(900), Control::Lock { on: true }),
                (ms(1500), Control::SetFreq { hz: 3_573_000 }),
            ],
        );
        let up: Vec<_> = v.iter().filter(|s| s.link == Link::Up).collect();
        let last = up.last().unwrap();
        assert_eq!(last.freq_hz, Some(7_074_000), "locked dial refused the second change");
        assert_eq!(last.nb, Some(true));
        assert_eq!(last.lock, Some(true));
        assert!(up.iter().any(|s| s.control_note.as_deref().unwrap_or("").contains("LOCK")));
    }

    #[test]
    fn view_only_refuses_controls() {
        let stop = Arc::new(AtomicBool::new(false));
        let seen: Arc<Mutex<Vec<RigState>>> = Arc::default();
        let (tx, rx) = mpsc::channel();
        let (s2, seen2) = (stop.clone(), seen.clone());
        let h = std::thread::spawn(move || {
            let cfg = MonitorConfig { source: Source::Demo, allow_control: false };
            run(cfg, s2, rx, |st| seen2.lock().unwrap().push(st.clone()));
        });
        std::thread::sleep(Duration::from_millis(400));
        tx.send(Control::SetFreq { hz: 7_074_000 }).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        stop.store(true, Ordering::Relaxed);
        h.join().unwrap();
        let v = seen.lock().unwrap();
        assert!(v.iter().filter(|s| s.link == Link::Up).all(|s| s.freq_hz == Some(14_074_000)));
        assert!(v.iter().any(|s| s.control_note.is_some()));
    }

    #[test]
    fn connection_error_stays_visible_through_retries() {
        let stop = Arc::new(AtomicBool::new(false));
        let seen: Arc<Mutex<Vec<RigState>>> = Arc::default();
        let (_tx, rx) = mpsc::channel();
        let (s2, seen2) = (stop.clone(), seen.clone());
        let h = std::thread::spawn(move || {
            let cfg = MonitorConfig {
                source: Source::Serial {
                    port: "/nonexistent/port".into(),
                    baud: 4800,
                    stop_bits: StopBits::Two,
                    rts_cts: true,
                },
                allow_control: true,
            };
            run(cfg, s2, rx, |st| seen2.lock().unwrap().push(st.clone()));
        });
        std::thread::sleep(Duration::from_millis(5600));
        stop.store(true, Ordering::Relaxed);
        h.join().unwrap();
        let v = seen.lock().unwrap();
        let countdown: Vec<u32> = v.iter().filter_map(|s| s.retry_in).take(5).collect();
        assert_eq!(countdown, vec![5, 4, 3, 2, 1]);
        let second = v.iter().find(|s| s.attempt == 2 && s.link == Link::Connecting).expect("second attempt");
        assert!(second.error.as_deref().unwrap_or("").contains("Cannot open the serial port"));
    }

    #[test]
    fn rigctld_commands_are_validated_first() {
        assert_eq!(rigctld_command(&Control::SetFreq { hz: 14_074_000 }).unwrap(), "F 14074000");
        assert!(rigctld_command(&Control::SetFreq { hz: 99_000_000 }).is_err());
        assert_eq!(rigctld_command(&Control::SetMode { mode: "D-U".into() }).unwrap(), "M PKTUSB 0");
        assert!(rigctld_command(&Control::Narrow { on: true }).is_err());
    }
}
