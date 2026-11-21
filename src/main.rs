//! SigmaOS-ar — audio recorder CLI.
//!
//! Usage:
//!   sigmaos-ar status|profiles|sessions|sources|formats
//!   sigmaos-ar prof-add <id> <name> <source-kind> <source> <rate> <ch> <fmt> [max-secs] [silence-secs]
//!   sigmaos-ar prof-del <id>
//!   sigmaos-ar rec <profile> <output.wav>     start recording (requires backend)
//!   sigmaos-ar stop <session-id> | pause <id> | resume <id>
//!   sigmaos-ar feed <id> <secs> <peak>        simulate backend level feed
//!   sigmaos-ar pcm <file.s16>                  compute rms/peak of raw s16 LE file
//!
//! NOTE: real capture needs a backend (pw-record/arecord). This CLI manages
//! profiles/sessions and metering; wire a CaptureBackend for live audio.

use sigmaos_ar::{FakeBackend, SigmaOSAr, SampleFormat, SourceKind};
use std::path::PathBuf;
use std::process::exit;

fn usage() -> ! {
    eprintln!("usage: sigmaos-ar <status|profiles|sessions|sources|formats|prof-add|prof-del|rec|stop|pause|resume|feed|pcm> [args]");
    exit(2);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() { usage(); }
    let db = SigmaOSAr::default_path();
    let mut m = SigmaOSAr::load(&db).unwrap_or_else(|_| SigmaOSAr::new());
    let cmd = args[0].as_str();
    let rest = &args[1..];
    let arg = |i: usize| -> &str { rest.get(i).map(|s| s.as_str()).unwrap_or_else(|| usage()) };
    // FakeBackend used by CLI until a pipewire backend is wired; sessions
    // still persist and lifecycle is fully exercised.
    let be = FakeBackend::new();

    let result: Result<(), String> = (|| -> Result<(), String> { match cmd {
        "status" => { println!("{}", m.status_line()); Ok(()) }
        "profiles" => {
            for p in m.profiles.values() {
                println!("{:<10} {:<12} {:<10} {}Hz/{}ch {} max={}s{}", p.id, p.name,
                    p.source_kind.label(), p.sample_rate, p.channels, p.format.label(),
                    p.max_duration, p.silence_timeout.map(|s| format!(" sil={}s",s)).unwrap_or_default());
            }
            Ok(())
        }
        "sessions" => {
            for s in m.sessions() {
                println!("{:<4} {:<10} {} -> {} peak={:.2} captured={}s bytes={}", s.id, s.profile,
                    s.state.label(), s.output.display(), s.peak, s.captured_secs, s.bytes_written);
            }
            Ok(())
        }
        "sources" => {
            for k in [SourceKind::Microphone, SourceKind::LineIn, SourceKind::Loopback, SourceKind::App, SourceKind::Mixed] {
                println!("{:<10} {}", k.label(), m.profiles.values().filter(|p| p.source_kind==k).count());
            }
            Ok(())
        }
        "formats" => {
            for f in [SampleFormat::S16, SampleFormat::S24, SampleFormat::S32, SampleFormat::F32] {
                println!("{:<5} {} bytes/sample", f.label(), f.bytes_per_sample());
            }
            Ok(())
        }
        "prof-add" => {
            let sk = SourceKind::parse(arg(2)).ok_or_else(|| format!("bad source kind {}", arg(2)))?;
            let fmt = SampleFormat::parse(arg(6)).ok_or_else(|| format!("bad format {}", arg(6)))?;
            let rate: u32 = arg(4).parse().map_err(|_| "bad rate")?;
            let ch: u16 = arg(5).parse().map_err(|_| "bad channels")?;
            let dur: u64 = rest.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);
            let sil: Option<u64> = rest.get(8).and_then(|s| s.parse().ok());
            m.add_profile(arg(0), arg(1), sk, arg(3), rate, ch, fmt, dur, sil).map_err(|e| e.to_string())?;
            println!("added profile {}", arg(0)); Ok(())
        }
        "prof-del" => { m.remove_profile(arg(0)).map_err(|e| e.to_string())?; println!("removed"); Ok(()) }
        "rec" => {
            let id = m.start(arg(0), &PathBuf::from(arg(1)), &be).map_err(|e| e.to_string())?;
            println!("recording session {}", id); Ok(())
        }
        "stop" => { let id: u64 = arg(0).parse().map_err(|_| "bad id")?; m.stop(id, &be).map_err(|e| e.to_string())?; println!("stopped"); Ok(()) }
        "pause" => { let id: u64 = arg(0).parse().map_err(|_| "bad id")?; m.pause(id).map_err(|e| e.to_string())?; println!("paused"); Ok(()) }
        "resume" => { let id: u64 = arg(0).parse().map_err(|_| "bad id")?; m.resume(id).map_err(|e| e.to_string())?; println!("resumed"); Ok(()) }
        "feed" => {
            let id: u64 = arg(0).parse().map_err(|_| "bad id")?;
            let secs: u64 = arg(1).parse().map_err(|_| "bad secs")?;
            let peak: f32 = arg(2).parse().map_err(|_| "bad peak")?;
            m.feed_levels(id, secs, peak).map_err(|e| e.to_string())?;
            println!("fed {}s peak={:.2}", secs, peak); Ok(())
        }
        "pcm" => {
            let data = std::fs::read(arg(0)).map_err(|e| e.to_string())?;
            let samples: Vec<i16> = data.chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
            let (rms, peak) = SigmaOSAr::pcm_levels(&samples);
            println!("{} samples: rms={:.4} peak={:.4}", samples.len(), rms, peak); Ok(())
        }
        _ => usage(),
    } })();

    match result {
        Ok(()) => { if let Err(e)=m.save(&db) { eprintln!("sigmaos-ar: warn: save: {e}"); } }
        Err(e) => { eprintln!("sigmaos-ar: {e}"); exit(1); }
    }
}
