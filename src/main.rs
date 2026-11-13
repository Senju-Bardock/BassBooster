use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use std::collections::VecDeque;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

// ---------- Tuning: normal mode ----------
const MAX_DB: f32 = 12.0; // boost applied at value 100
const HIGHPASS_HZ: f32 = 30.0; // cuts sub-bass rumble that eats headroom
const PUNCH_HZ: f32 = 80.0; // main bass band (kick / bass fundamentals)
const PUNCH_Q: f32 = 0.8; // higher = narrower band
const BODY_HZ: f32 = 140.0; // upper bass band (gives definition to bass lines)
const BODY_Q: f32 = 1.0;
const BODY_WEIGHT: f32 = 0.5; // body band gain relative to the main band
const HEADROOM: f32 = 0.5; // how much of the boost is compensated by lowering the overall level

// ---------- Tuning: thump mode ----------
const THUMP_HIGHPASS_HZ: f32 = 22.0; // lets more sub-bass through
const SUB_HZ: f32 = 50.0; // sub-bass band
const SUB_Q: f32 = 0.9;
const SUB_WEIGHT: f32 = 0.8; // sub gain relative to the main band
const LOW_SPLIT_HZ: f32 = 120.0; // everything below this goes through the low-end compressor
const COMP_THRESHOLD: f32 = 0.2; // linear level where compression starts
const COMP_RATIO: f32 = 3.0;
const COMP_ATTACK_MS: f32 = 10.0; // slow enough to let the initial hit through
const COMP_RELEASE_MS: f32 = 250.0; // higher = longer thump tail
const THUMP_MAKEUP_DB: f32 = 4.0; // level given back to the low end (lifts the tail)
const HARM_DRIVE: f32 = 4.0; // how hard the bass is saturated to create harmonics
const HARM_HP_HZ: f32 = 90.0; // keeps only the harmonics, drops the saturated fundamental
const HARM_MIX: f32 = 0.35; // amount of generated harmonics
const THUMP_EXTRA_HEADROOM_DB: f32 = 3.0; // extra level reduction at 100 to avoid clipping

#[derive(Clone, Copy)]
struct Coefs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

fn normalize(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Coefs {
    Coefs {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
    }
}

// 2nd order Butterworth high-pass (RBJ Audio EQ Cookbook)
fn high_pass(sr: f32, hz: f32) -> Coefs {
    let w0 = 2.0 * std::f32::consts::PI * hz / sr;
    let (sin, cos) = (w0.sin(), w0.cos());
    let alpha = sin / (2.0 * 0.7071);
    normalize(
        (1.0 + cos) / 2.0,
        -(1.0 + cos),
        (1.0 + cos) / 2.0,
        1.0 + alpha,
        -2.0 * cos,
        1.0 - alpha,
    )
}

// 2nd order Butterworth low-pass (RBJ Audio EQ Cookbook)
fn low_pass(sr: f32, hz: f32) -> Coefs {
    let w0 = 2.0 * std::f32::consts::PI * hz / sr;
    let (sin, cos) = (w0.sin(), w0.cos());
    let alpha = sin / (2.0 * 0.7071);
    normalize(
        (1.0 - cos) / 2.0,
        1.0 - cos,
        (1.0 - cos) / 2.0,
        1.0 + alpha,
        -2.0 * cos,
        1.0 - alpha,
    )
}

// Peaking EQ (RBJ Audio EQ Cookbook)
fn peaking(sr: f32, hz: f32, q: f32, db: f32) -> Coefs {
    let a = 10f32.powf(db / 40.0);
    let w0 = 2.0 * std::f32::consts::PI * hz / sr;
    let (sin, cos) = (w0.sin(), w0.cos());
    let alpha = sin / (2.0 * q);
    normalize(
        1.0 + alpha * a,
        -2.0 * cos,
        1.0 - alpha * a,
        1.0 + alpha / a,
        -2.0 * cos,
        1.0 - alpha / a,
    )
}

#[derive(Clone, Copy)]
struct Params {
    chain: [Coefs; 4], // high-pass, punch, body, sub
    lp: Coefs,         // low band splitter (thump mode)
    hph: Coefs,        // harmonics high-pass (thump mode)
    pre_gain: f32,
    thump: bool,
    makeup: f32,
    harm_mix: f32,
    att: f32,
    rel: f32,
}

fn build_params(sr: f32, amount: f32, thump: bool) -> Params {
    let k = amount / 100.0;
    let db = k * MAX_DB;
    let sub_db = if thump { db * SUB_WEIGHT } else { 0.0 };
    let hp_hz = if thump { THUMP_HIGHPASS_HZ } else { HIGHPASS_HZ };
    let extra = if thump { k * THUMP_EXTRA_HEADROOM_DB } else { 0.0 };
    // Lower the overall level so the boosted bass doesn't clip
    let pre_gain = 10f32.powf(-((db + sub_db) * HEADROOM + extra) / 20.0);

    Params {
        chain: [
            high_pass(sr, hp_hz),
            peaking(sr, PUNCH_HZ, PUNCH_Q, db),
            peaking(sr, BODY_HZ, BODY_Q, db * BODY_WEIGHT),
            peaking(sr, SUB_HZ, SUB_Q, sub_db), // 0 dB = does nothing
        ],
        lp: low_pass(sr, LOW_SPLIT_HZ),
        hph: high_pass(sr, HARM_HP_HZ),
        pre_gain,
        thump,
        makeup: 10f32.powf(THUMP_MAKEUP_DB * k / 20.0),
        harm_mix: HARM_MIX * k,
        att: (-1.0 / (sr * COMP_ATTACK_MS / 1000.0)).exp(),
        rel: (-1.0 / (sr * COMP_RELEASE_MS / 1000.0)).exp(),
    }
}

#[derive(Default, Clone, Copy)]
struct Biquad {
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn process(&mut self, x: f32, c: &Coefs) -> f32 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }
}

#[derive(Default, Clone, Copy)]
struct ChannelState {
    chain: [Biquad; 4],
    lp: Biquad,
    hph: Biquad,
}

// Gentle limiter, only acts on very high peaks
fn soft_clip(x: f32) -> f32 {
    let t = 0.95;
    let a = x.abs();
    if a <= t {
        x
    } else {
        x.signum() * (t + (1.0 - t) * ((a - t) / (1.0 - t)).tanh())
    }
}

fn find_device(devs: impl Iterator<Item = cpal::Device>, name: &str) -> Option<cpal::Device> {
    let name = name.to_lowercase();
    devs.into_iter().find(|d| {
        d.name()
            .map(|n| n.to_lowercase().contains(&name))
            .unwrap_or(false)
    })
}

fn print_help() {
    println!("Bass Booster");
    println!();
    println!("Usage:");
    println!("  bassboost [AMOUNT] [--thump] [--input NAME] [--output NAME]");
    println!("  bassboost --list");
    println!();
    println!("  AMOUNT    bass boost from 0 to 100 (default: 50)");
    println!("  --thump   start with thump mode enabled (deeper, longer bass hits)");
    println!("  --input   input device name (or part of it)");
    println!("  --output  output device name (or part of it)");
    println!("  --list    list available audio devices");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let host = cpal::default_host();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }

    if args.iter().any(|a| a == "--list") {
        println!("Input devices:");
        for d in host.input_devices()? {
            println!("  {}", d.name()?);
        }
        println!("Output devices:");
        for d in host.output_devices()? {
            println!("  {}", d.name()?);
        }
        return Ok(());
    }

    let mut amount: u32 = 50;
    let mut thump = false;
    let (mut in_name, mut out_name) = (None, None);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                in_name = args.get(i + 1).cloned();
                i += 1;
            }
            "--output" => {
                out_name = args.get(i + 1).cloned();
                i += 1;
            }
            "--thump" => thump = true,
            v => {
                if let Ok(n) = v.parse::<u32>() {
                    amount = n.min(100);
                }
            }
        }
        i += 1;
    }

    let in_dev = match in_name {
        Some(n) => find_device(host.input_devices()?, &n).ok_or("input device not found")?,
        None => host
            .default_input_device()
            .ok_or("no default input device")?,
    };
    let out_dev = match out_name {
        Some(n) => find_device(host.output_devices()?, &n).ok_or("output device not found")?,
        None => host
            .default_output_device()
            .ok_or("no default output device")?,
    };

    let out_cfg = out_dev.default_output_config()?;
    if out_cfg.sample_format() != SampleFormat::F32 {
        return Err("output device does not support f32 samples".into());
    }
    let cfg: StreamConfig = out_cfg.config();
    let ch = cfg.channels as usize;
    let sr = cfg.sample_rate.0 as f32;

    println!("Input:  {}", in_dev.name()?);
    println!("Output: {}", out_dev.name()?);
    println!("Format: {} Hz, {} channels", cfg.sample_rate.0, cfg.channels);
    println!("Bass boost: {amount}");
    println!("Thump mode: {}", if thump { "ON" } else { "OFF" });
    println!("Type a value from 0 to 100 and press Enter to change the bass.");
    println!("Type t and press Enter to toggle thump mode, or q to quit.");

    let max_samples = (sr as usize * ch) / 10; // ~100 ms max buffer
    let buf: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
    let amount_a = Arc::new(AtomicU32::new(amount));
    let thump_a = Arc::new(AtomicBool::new(thump));

    let buf_in = buf.clone();
    let in_stream = in_dev.build_input_stream(
        &cfg,
        move |data: &[f32], _| {
            let mut q = buf_in.lock().unwrap();
            q.extend(data.iter().copied());
            while q.len() > max_samples {
                q.pop_front();
            }
        },
        |e| eprintln!("input error: {e}"),
        None,
    )?;

    let buf_out = buf.clone();
    let amt = amount_a.clone();
    let thp = thump_a.clone();
    let mut st = vec![ChannelState::default(); ch];
    let mut tmp = vec![0.0f32; ch];
    let mut low = vec![0.0f32; ch];
    let mut env = 0.0f32;
    let mut last_a = u32::MAX;
    let mut last_t = !thump;
    let mut p = build_params(sr, 0.0, false);
    let out_stream = out_dev.build_output_stream(
        &cfg,
        move |data: &mut [f32], _| {
            let a = amt.load(Relaxed);
            let t = thp.load(Relaxed);
            if a != last_a || t != last_t {
                p = build_params(sr, a as f32, t);
                last_a = a;
                last_t = t;
            }
            let mut q = buf_out.lock().unwrap();
            for frame in data.chunks_mut(ch) {
                if q.len() < ch || frame.len() < ch {
                    frame.fill(0.0);
                    continue;
                }

                // 1) Headroom + EQ on every channel
                for c in 0..ch {
                    let mut y = q.pop_front().unwrap_or(0.0) * p.pre_gain;
                    for k in 0..4 {
                        y = st[c].chain[k].process(y, &p.chain[k]);
                    }
                    tmp[c] = y;
                }

                // 2) Thump: compress the low band (linked across channels) and add harmonics
                if p.thump {
                    let mut peak = 0.0f32;
                    for c in 0..ch {
                        low[c] = st[c].lp.process(tmp[c], &p.lp);
                        peak = peak.max(low[c].abs());
                    }
                    let coef = if peak > env { p.att } else { p.rel };
                    env = peak + coef * (env - peak);
                    let mut gain = if env > COMP_THRESHOLD {
                        (COMP_THRESHOLD + (env - COMP_THRESHOLD) / COMP_RATIO) / env
                    } else {
                        1.0
                    };
                    gain *= p.makeup;
                    for c in 0..ch {
                        let rest = tmp[c] - low[c];
                        let harm = st[c].hph.process((HARM_DRIVE * low[c]).tanh(), &p.hph)
                            * p.harm_mix;
                        tmp[c] = rest + low[c] * gain + harm;
                    }
                }

                // 3) Output
                for c in 0..ch {
                    frame[c] = soft_clip(tmp[c]);
                }
            }
        },
        |e| eprintln!("output error: {e}"),
        None,
    )?;

    in_stream.play()?;
    out_stream.play()?;

    for line in std::io::stdin().lock().lines() {
        let line = line?;
        let line = line.trim();
        if line == "q" {
            break;
        }
        if line == "t" {
            let now = !thump_a.load(Relaxed);
            thump_a.store(now, Relaxed);
            println!("Thump mode: {}", if now { "ON" } else { "OFF" });
            continue;
        }
        match line.parse::<u32>() {
            Ok(n) if n <= 100 => {
                amount_a.store(n, Relaxed);
                println!("Bass boost: {n}");
            }
            _ => println!("Please enter a number between 0 and 100, t or q"),
        }
    }
    Ok(())
}
