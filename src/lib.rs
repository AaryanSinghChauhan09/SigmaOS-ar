// SPDX-License-Identifier: Apache-2.0
//!
//! SigmaOS-ar — audio recorder / capture session manager.
//!
//! Manages recording profiles (source, format, sample rate, channels,
//! bit depth, duration limit) and recording sessions (start/stop,
//! level metering, file output). Capture itself is delegated to a
//! `CaptureBackend` — the real CLI wires pipewire/pulseaudio tools;
//! tests use a deterministic fake. Level metering is computed from
//! PCM samples so VU-meter logic is genuinely testable.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SampleFormat { S16, S24, S32, F32 }
impl SampleFormat {
    pub fn label(self) -> &'static str { match self { Self::S16=>"s16", Self::S24=>"s24", Self::S32=>"s32", Self::F32=>"f32" } }
    pub fn parse(s: &str) -> Option<Self> {
        match s { "s16"|"16"=>Some(Self::S16), "s24"|"24"=>Some(Self::S24),
                  "s32"|"32"=>Some(Self::S32), "f32"|"float"=>Some(Self::F32), _=>None }
    }
    pub fn bytes_per_sample(self) -> usize { match self { Self::S16=>2, Self::S24=>3, Self::S32|Self::F32=>4 } }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SourceKind { Microphone, LineIn, Loopback, App, Mixed }
impl SourceKind {
    pub fn label(self) -> &'static str { match self { Self::Microphone=>"microphone", Self::LineIn=>"line-in", Self::Loopback=>"loopback", Self::App=>"app", Self::Mixed=>"mixed" } }
    pub fn parse(s: &str) -> Option<Self> {
        match s { "mic"|"microphone"=>Some(Self::Microphone), "line"|"line-in"=>Some(Self::LineIn),
                  "loop"|"loopback"=>Some(Self::Loopback), "app"=>Some(Self::App), "mixed"=>Some(Self::Mixed), _=>None }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub source_kind: SourceKind,
    /// source identifier (device name / node / app id)
    pub source: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub format: SampleFormat,
    /// max recording seconds (0 = unlimited)
    pub max_duration: u64,
    /// auto-stop on silence (seconds of silence tolerated)
    pub silence_timeout: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RecState { Idle, Recording, Paused, Stopped }
impl RecState {
    pub fn label(self) -> &'static str { match self { Self::Idle=>"idle", Self::Recording=>"recording", Self::Paused=>"paused", Self::Stopped=>"stopped" } }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Session {
    pub id: u64,
    pub profile: String,
    pub output: PathBuf,
    pub state: RecState,
    pub started: u64,
    pub stopped: Option<u64>,
    /// cumulative peak level 0.0-1.0 observed
    pub peak: f32,
    /// seconds of audio captured (fed by backend)
    pub captured_secs: u64,
    pub bytes_written: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArError {
    DuplicateProfile(String),
    NoSuchProfile(String),
    DuplicateSession(u64),
    NoSuchSession(u64),
    BadRate(u32),
    BadChannels(u16),
    BadState(String),
    AlreadyRecording,
    NotRecording,
    DurationExceeded,
    BackendFailed(String),
}
impl fmt::Display for ArError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateProfile(id)=>write!(f,"profile {id} exists"),
            Self::NoSuchProfile(id)=>write!(f,"no profile {id}"),
            Self::DuplicateSession(id)=>write!(f,"session {id} exists"),
            Self::NoSuchSession(id)=>write!(f,"no session {id}"),
            Self::BadRate(r)=>write!(f,"bad sample rate {r}"),
            Self::BadChannels(c)=>write!(f,"bad channel count {c}"),
            Self::BadState(s)=>write!(f,"bad state {s}"),
            Self::AlreadyRecording=>write!(f,"already recording"),
            Self::NotRecording=>write!(f,"not recording"),
            Self::DurationExceeded=>write!(f,"max duration exceeded"),
            Self::BackendFailed(e)=>write!(f,"backend: {e}"),
        }
    }
}
impl std::error::Error for ArError {}

/// Capture backend abstraction.
pub trait CaptureBackend {
    /// Start capture; returns backend session token.
    fn start(&self, profile: &Profile, output: &Path) -> Result<String, String>;
    fn stop(&self, token: &str) -> Result<u64, String>; // returns bytes written
}

/// Deterministic fake backend for tests.
pub struct FakeBackend {
    pub started: std::cell::RefCell<Vec<(String, PathBuf)>>,
    pub stopped: std::cell::RefCell<Vec<String>>,
    pub fail_start: bool,
}
impl FakeBackend {
    pub fn new() -> Self { Self { started: std::cell::RefCell::new(vec![]), stopped: std::cell::RefCell::new(vec![]), fail_start: false } }
}
impl CaptureBackend for FakeBackend {
    fn start(&self, profile: &Profile, output: &Path) -> Result<String, String> {
        if self.fail_start { return Err("injected".into()); }
        let tok = format!("fake-{}", self.started.borrow().len()+1);
        self.started.borrow_mut().push((profile.id.clone(), output.to_path_buf()));
        Ok(tok)
    }
    fn stop(&self, token: &str) -> Result<u64, String> {
        self.stopped.borrow_mut().push(token.into());
        Ok(4096)
    }
}

pub struct SigmaOSAr {
    pub profiles: BTreeMap<String, Profile>,
    pub sessions: BTreeMap<u64, Session>,
    next_session: u64,
    /// backend tokens: session id -> token
    tokens: BTreeMap<u64, String>,
}

impl SigmaOSAr {
    pub fn new() -> Self {
        let mut m = Self { profiles: BTreeMap::new(), sessions: BTreeMap::new(), next_session: 1, tokens: BTreeMap::new() };
        m.seed_profiles();
        m
    }

    fn seed_profiles(&mut self) {
        let seeds = [
            ("voice", "Voice memo", SourceKind::Microphone, "default", 48000, 1, SampleFormat::S16, 3600, None),
            ("meeting", "Meeting capture", SourceKind::Mixed, "all-sources", 48000, 2, SampleFormat::S16, 7200, Some(30)),
            ("music", "Music loopback", SourceKind::Loopback, "output-monitor", 44100, 2, SampleFormat::S24, 0, None),
            ("quick", "Quick note", SourceKind::Microphone, "usb-mic", 16000, 1, SampleFormat::S16, 300, Some(10)),
        ];
        for (id, name, sk, src, rate, ch, fmt, dur, sil) in seeds {
            self.profiles.insert(id.into(), Profile {
                id: id.into(), name: name.into(), source_kind: sk, source: src.into(),
                sample_rate: rate, channels: ch, format: fmt, max_duration: dur, silence_timeout: sil,
            });
        }
    }

    // ---- profiles ----
    pub fn add_profile(&mut self, id: &str, name: &str, sk: SourceKind, source: &str,
                       rate: u32, channels: u16, fmt: SampleFormat, max_duration: u64,
                       silence_timeout: Option<u64>) -> Result<(), ArError> {
        if self.profiles.contains_key(id) { return Err(ArError::DuplicateProfile(id.into())); }
        if !(8000..=384000).contains(&rate) { return Err(ArError::BadRate(rate)); }
        if !(1..=8).contains(&channels) { return Err(ArError::BadChannels(channels)); }
        self.profiles.insert(id.into(), Profile {
            id: id.into(), name: name.into(), source_kind: sk, source: source.into(),
            sample_rate: rate, channels, format: fmt, max_duration, silence_timeout,
        });
        Ok(())
    }

    pub fn remove_profile(&mut self, id: &str) -> Result<(), ArError> {
        if self.profiles.remove(id).is_none() { return Err(ArError::NoSuchProfile(id.into())); }
        Ok(())
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> { self.profiles.get(id) }

    // ---- sessions ----
    pub fn active_session(&self) -> Option<&Session> {
        self.sessions.values().find(|s| matches!(s.state, RecState::Recording|RecState::Paused))
    }

    pub fn start<B: CaptureBackend>(&mut self, profile_id: &str, output: &Path, backend: &B) -> Result<u64, ArError> {
        if self.active_session().is_some() { return Err(ArError::AlreadyRecording); }
        let p = self.profiles.get(profile_id).ok_or_else(|| ArError::NoSuchProfile(profile_id.into()))?;
        let token = backend.start(p, output).map_err(ArError::BackendFailed)?;
        let id = self.next_session; self.next_session += 1;
        self.sessions.insert(id, Session {
            id, profile: profile_id.into(), output: output.to_path_buf(),
            state: RecState::Recording, started: now_unix(), stopped: None,
            peak: 0.0, captured_secs: 0, bytes_written: 0,
        });
        self.tokens.insert(id, token);
        Ok(id)
    }

    pub fn stop<B: CaptureBackend>(&mut self, id: u64, backend: &B) -> Result<(), ArError> {
        let s = self.sessions.get(&id).ok_or(ArError::NoSuchSession(id))?;
        if !matches!(s.state, RecState::Recording|RecState::Paused) { return Err(ArError::BadState(s.state.label().into())); }
        // Token may be absent when the session was restored from disk —
        // the backend process cannot outlive the CLI invocation, so
        // finalize the session as stopped rather than erroring.
        let bytes = match self.tokens.get(&id).cloned() {
            Some(token) => backend.stop(&token).map_err(ArError::BackendFailed)?,
            None => 0,
        };
        let s = self.sessions.get_mut(&id).unwrap();
        s.state = RecState::Stopped; s.stopped = Some(now_unix());
        if bytes > 0 { s.bytes_written = bytes; }
        self.tokens.remove(&id);
        Ok(())
    }

    pub fn pause(&mut self, id: u64) -> Result<(), ArError> {
        let s = self.sessions.get_mut(&id).ok_or(ArError::NoSuchSession(id))?;
        if s.state != RecState::Recording { return Err(ArError::BadState(s.state.label().into())); }
        s.state = RecState::Paused;
        Ok(())
    }

    pub fn resume(&mut self, id: u64) -> Result<(), ArError> {
        let s = self.sessions.get_mut(&id).ok_or(ArError::NoSuchSession(id))?;
        if s.state != RecState::Paused { return Err(ArError::BadState(s.state.label().into())); }
        s.state = RecState::Recording;
        Ok(())
    }

    /// Feed captured audio duration + peak level (called by backend pump).
    pub fn feed_levels(&mut self, id: u64, secs: u64, peak: f32) -> Result<(), ArError> {
        let s = self.sessions.get_mut(&id).ok_or(ArError::NoSuchSession(id))?;
        s.captured_secs += secs;
        if peak > s.peak { s.peak = peak; }
        // enforce max duration
        let prof = self.profiles.get(&s.profile).cloned();
        if let Some(p) = prof {
            if p.max_duration > 0 && s.captured_secs >= p.max_duration {
                s.state = RecState::Stopped;
                s.stopped = Some(now_unix());
                return Err(ArError::DurationExceeded);
            }
        }
        Ok(())
    }

    /// Compute RMS + peak from s16 LE PCM samples (real DSP, testable).
    pub fn pcm_levels(samples: &[i16]) -> (f32, f32) {
        if samples.is_empty() { return (0.0, 0.0); }
        let mut sum_sq = 0.0f64;
        let mut peak = 0.0f32;
        for &s in samples {
            let v = s as f32 / 32768.0;
            sum_sq += (v*v) as f64;
            let a = v.abs();
            if a > peak { peak = a; }
        }
        let rms = (sum_sq / samples.len() as f64).sqrt() as f32;
        (rms, peak)
    }

    pub fn session(&self, id: u64) -> Option<&Session> { self.sessions.get(&id) }
    pub fn sessions(&self) -> impl Iterator<Item = &Session> { self.sessions.values() }

    pub fn status_line(&self) -> String {
        let active = self.active_session().map(|s| format!("active={}({})", s.id, s.state.label())).unwrap_or_else(|| "idle".into());
        format!("ar: {} profiles, {} sessions, {}", self.profiles.len(), self.sessions.len(), active)
    }
}

impl Default for SigmaOSAr { fn default() -> Self { Self::new() } }

fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0)
}

// ---- persistence (profiles + session metadata; audio files stay on disk) ----
#[derive(serde::Serialize, serde::Deserialize)]
struct ArFile { next_session: u64, profiles: Vec<Profile>, sessions: Vec<Session> }

impl SigmaOSAr {
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let j = serde_json::to_string_pretty(&ArFile {
            next_session: self.next_session,
            profiles: self.profiles.values().cloned().collect(),
            sessions: self.sessions.values().cloned().collect(),
        }).map_err(|e| e.to_string())?;
        if let Some(p) = path.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        std::fs::write(path, j).map_err(|e| e.to_string())
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let txt = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let f: ArFile = serde_json::from_str(&txt).map_err(|e| e.to_string())?;
        let mut m = Self { profiles: BTreeMap::new(), sessions: BTreeMap::new(), next_session: f.next_session, tokens: BTreeMap::new() };
        for p in f.profiles { m.profiles.insert(p.id.clone(), p); }
        for s in f.sessions { m.sessions.insert(s.id, s); }
        Ok(m)
    }
    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("SIGMAOS_REC_FILE") { return PathBuf::from(p); }
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".into()))
            .join(".local/share/sigmaos/recorder.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_management() {
        let mut m = SigmaOSAr::new();
        let seeded = m.profiles.len();
        assert!(seeded >= 4);
        m.add_profile("custom", "Custom", SourceKind::App, "firefox", 44100, 2, SampleFormat::F32, 0, None).unwrap();
        assert!(m.add_profile("custom", "dup", SourceKind::App, "x", 44100, 2, SampleFormat::S16, 0, None).is_err());
        assert!(m.add_profile("bad", "x", SourceKind::App, "x", 100, 2, SampleFormat::S16, 0, None).is_err()); // rate
        assert!(m.add_profile("bad2", "x", SourceKind::App, "x", 44100, 0, SampleFormat::S16, 0, None).is_err()); // channels
        assert!(m.remove_profile("custom").is_ok());
        assert!(m.remove_profile("custom").is_err());
    }

    #[test]
    fn session_lifecycle() {
        let mut m = SigmaOSAr::new();
        let be = FakeBackend::new();
        let id = m.start("voice", Path::new("/tmp/rec.wav"), &be).unwrap();
        assert_eq!(m.session(id).unwrap().state, RecState::Recording);
        // only one active at a time
        assert!(m.start("music", Path::new("/tmp/x.wav"), &be).is_err());
        m.pause(id).unwrap();
        assert!(m.pause(id).is_err()); // already paused
        m.resume(id).unwrap();
        m.stop(id, &be).unwrap();
        assert_eq!(m.session(id).unwrap().state, RecState::Stopped);
        assert_eq!(m.session(id).unwrap().bytes_written, 4096);
        assert_eq!(be.stopped.borrow().len(), 1);
        // can start again after stop
        let id2 = m.start("quick", Path::new("/tmp/q.wav"), &be).unwrap();
        assert_ne!(id, id2);
        m.stop(id2, &be).unwrap();
    }

    #[test]
    fn missing_profile_fails() {
        let mut m = SigmaOSAr::new();
        let be = FakeBackend::new();
        assert!(m.start("nope", Path::new("/x"), &be).is_err());
        assert!(m.stop(999, &be).is_err());
        assert!(m.pause(999).is_err());
    }

    #[test]
    fn max_duration_enforced() {
        let mut m = SigmaOSAr::new();
        let be = FakeBackend::new();
        let id = m.start("quick", Path::new("/tmp/q.wav"), &be).unwrap(); // max_duration=300
        m.feed_levels(id, 200, 0.5).unwrap();
        assert_eq!(m.session(id).unwrap().state, RecState::Recording);
        let r = m.feed_levels(id, 200, 0.6); // 400 > 300
        assert!(r.is_err());
        assert_eq!(m.session(id).unwrap().state, RecState::Stopped);
        assert!(m.session(id).unwrap().peak > 0.5);
    }

    #[test]
    fn pcm_level_metering() {
        // silence
        let (rms, peak) = SigmaOSAr::pcm_levels(&[0i16; 100]);
        assert_eq!(rms, 0.0);
        assert_eq!(peak, 0.0);
        // full-scale square wave
        let samples: Vec<i16> = (0..1000).map(|i| if i%2==0 {32767} else {-32768}).collect();
        let (rms, peak) = SigmaOSAr::pcm_levels(&samples);
        assert!(rms > 0.9);
        assert!(peak > 0.99);
        // half amplitude
        let quiet: Vec<i16> = vec![16384; 100];
        let (rms2, _) = SigmaOSAr::pcm_levels(&quiet);
        assert!((rms2 - 0.5).abs() < 0.01);
        // empty
        assert_eq!(SigmaOSAr::pcm_levels(&[]), (0.0, 0.0));
    }

    #[test]
    fn backend_failure_propagates() {
        let mut m = SigmaOSAr::new();
        let be = FakeBackend { fail_start: true, ..FakeBackend::new() };
        assert!(m.start("voice", Path::new("/x"), &be).is_err());
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = std::env::temp_dir().join(format!("sigmaos-ar-test-{}", std::process::id()));
        let p = dir.join("rec.json");
        let mut m = SigmaOSAr::new();
        let be = FakeBackend::new();
        let id = m.start("voice", Path::new("/tmp/v.wav"), &be).unwrap();
        m.stop(id, &be).unwrap();
        m.save(&p).unwrap();
        let m2 = SigmaOSAr::load(&p).unwrap();
        assert_eq!(m2.profiles.len(), m.profiles.len());
        assert_eq!(m2.sessions.len(), 1);
        assert_eq!(m2.session(id).unwrap().state, RecState::Stopped);
        assert_eq!(m2.session(id).unwrap().bytes_written, 4096);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_line() {
        let m = SigmaOSAr::new();
        let s = m.status_line();
        assert!(s.contains("profiles"));
        assert!(s.contains("idle"));
    }
}
