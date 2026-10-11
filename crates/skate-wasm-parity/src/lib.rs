//! Native-vs-wasm32 bit parity for the engine crates the browser build runs.
//!
//! Every section drives production code over fixed synthetic inputs and folds each output into a
//! 64-bit FNV-1a digest: f32/f64 bits for sample streams (NaN canonicalised: wasm may change a NaN's
//! sign and payload, nothing else) and `Debug` text for structured state (shortest round-trip, so
//! also bit-exact). A native build and a wasm32 build must give the same digest for every
//! [`Kind::Engine`] and [`Kind::Cost`] section; `tools/web-port/parity.sh` runs both and compares.
//!
//! `f32::mul_add` is exactly rounded everywhere (a CRT `fmaf` or `vfmadd` natively, the software
//! `fmaf` in compiler-builtins on wasm), so it cannot cause a difference; only its speed changes,
//! which the `cost/*` sections time. The std transcendentals can: they call the platform's libm (UCRT
//! on Windows, glibc on Linux, Rust's `libm` port on wasm), which are not correctly rounded and need
//! not agree. The `libm/*` sections sweep each one the engine calls, at its types and ranges; they
//! are measured, not required to match.
//!
//! No EA data: inputs are generated here, and the map is the procedural `maps/format-demo.skate`.
use std::f32::consts::PI;
use std::fmt::Debug;
use std::io::{Read, Write};

use skate_audio::dsp;
use skate_core::math::Vector3;
use skate_core::physics::{
    board::{BodyId, RETAIL_TRUCK_Y_POSITION, RETAIL_WHEEL_RADIUS},
    board_runtime::{BoardMotion, BoardRuntime},
    board_step::BoardStepSettings,
    board_world::{BoardWorld, ContactRetentionSettings, WheelWorldSettings, WorldTriangle},
    collision::{TriangleFeature, WorldContactSettings},
    contact::RetailContactMaterial,
    drive_frames::{AuthoredTransformInputs, RetailAffineTransform, authored_body_transforms, default_truck_transforms},
    drive_parameters::{RetailTruckDriveSettings, retail_truck_drive_dynamics},
    mass::default_skateboard_mass_properties,
    rigid_body::RetailSimulationStep,
};
use skate_core::riding::push::{PushInput, PushLimits, enqueue_push};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Production engine paths: must match bit for bit.
    Engine,
    /// `mul_add` against a separate multiply and add, timed for the cost report; must match too.
    Cost,
    /// One std transcendental through the platform libm, one f32 word per output: measured only.
    LibmF32,
    /// As [`Kind::LibmF32`], with f64 outputs (two words each, low first).
    LibmF64,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Engine => "engine",
            Kind::Cost => "cost",
            Kind::LibmF32 => "libm-f32",
            Kind::LibmF64 => "libm-f64",
        }
    }

    pub fn must_match(self) -> bool {
        matches!(self, Kind::Engine | Kind::Cost)
    }
}

pub struct Section {
    pub name: &'static str,
    pub kind: Kind,
    body: fn(&mut Digest),
}

/// Run order. Names are stable: they key the native/wasm comparison and the value dumps.
pub const SECTIONS: &[Section] = &[
    Section { name: "core/trigonometry", kind: Kind::Engine, body: core_trigonometry },
    Section { name: "core/board-ramp", kind: Kind::Engine, body: core_board_ramp },
    Section { name: "audio/dsp-kernels", kind: Kind::Engine, body: audio_dsp_kernels },
    Section { name: "audio/voice-chain", kind: Kind::Engine, body: audio_voice_chain },
    Section { name: "audio/mixmap-tables", kind: Kind::Engine, body: audio_mixmap_tables },
    Section { name: "data/format-demo", kind: Kind::Engine, body: data_format_demo },
    Section { name: "data/codecs", kind: Kind::Engine, body: data_codecs },
    Section { name: "cost/mul-add", kind: Kind::Cost, body: cost_mul_add },
    Section { name: "cost/mul-then-add", kind: Kind::Cost, body: cost_mul_then_add },
    Section { name: "libm/f32-sin", kind: Kind::LibmF32, body: libm_f32_sin },
    Section { name: "libm/f32-cos", kind: Kind::LibmF32, body: libm_f32_cos },
    Section { name: "libm/f32-sin_cos", kind: Kind::LibmF32, body: libm_f32_sin_cos },
    Section { name: "libm/f32-tan", kind: Kind::LibmF32, body: libm_f32_tan },
    Section { name: "libm/f32-atan2", kind: Kind::LibmF32, body: libm_f32_atan2 },
    Section { name: "libm/f32-acos", kind: Kind::LibmF32, body: libm_f32_acos },
    Section { name: "libm/f64-sin_cos-to-f32", kind: Kind::LibmF32, body: libm_f64_sin_cos_to_f32 },
    Section { name: "libm/f64-powf", kind: Kind::LibmF64, body: libm_f64_powf },
];

pub fn section(name: &str) -> Option<&'static Section> {
    SECTIONS.iter().find(|s| s.name == name)
}

/// One section's digest and, with `capture`, every output word in order (for libm comparisons).
pub fn run(section: &Section, capture: bool) -> (u64, Vec<u32>) {
    let mut digest = Digest { hash: 0xCBF2_9CE4_8422_2325, capture: capture.then(Vec::new) };
    (section.body)(&mut digest);
    (digest.hash, digest.capture.unwrap_or_default())
}

pub struct Digest {
    hash: u64,
    capture: Option<Vec<u32>>,
}

impl Digest {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.hash = (self.hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
        }
    }

    fn word(&mut self, word: u32) {
        self.bytes(&word.to_le_bytes());
        if let Some(capture) = &mut self.capture {
            capture.push(word);
        }
    }

    fn f32(&mut self, v: f32) {
        self.word(if v.is_nan() { 0x7FC0_0000 } else { v.to_bits() });
    }

    fn f32s(&mut self, values: &[f32]) {
        for &v in values {
            self.f32(v);
        }
    }

    fn f64(&mut self, v: f64) {
        let bits = if v.is_nan() { 0x7FF8_0000_0000_0000 } else { v.to_bits() };
        self.word(bits as u32);
        self.word((bits >> 32) as u32);
    }

    fn debug(&mut self, value: &impl Debug) {
        self.bytes(format!("{value:?}").as_bytes());
    }
}

/// Numerical Recipes LCG; `range` uses 24 bits, so every step is exact in f32.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * ((self.next() >> 8) as f32 / (1u32 << 24) as f32)
    }
}

/// The retail XNA polynomials (`mul_add` chains) over eight turns either side of zero.
fn core_trigonometry(d: &mut Digest) {
    use skate_core::trigonometry::{acos, asin, cos, sin, sin_cos};
    let mut rng = Lcg(1);
    for _ in 0..1 << 19 {
        let angle = rng.range(-4.0 * PI, 4.0 * PI);
        let (s, c) = sin_cos(angle);
        d.f32s(&[s, c, sin(angle), cos(angle)]);
        let v = rng.range(-1.0, 1.0);
        d.f32s(&[acos(v), asin(v)]);
    }
}

/// A flat run-up, a 6 m ramp rising 1.2 m and a plateau, 200 m wide, facing up.
fn ramp() -> Vec<WorldTriangle> {
    let material = RetailContactMaterial { static_friction: 0.8, dynamic_friction: 0.6, restitution: 0.2 };
    let profile = [(-100.0, 0.0), (4.0, 0.0), (10.0, 1.2), (100.0, 1.2)];
    let mut triangles = Vec::new();
    for pair in profile.windows(2) {
        let ((z0, y0), (z1, y1)) = (pair[0], pair[1]);
        let at = |x: f32, z: f32, y: f32| Vector3::new(x, y, z);
        for vertices in [
            [at(-100.0, z0, y0), at(-100.0, z1, y1), at(100.0, z0, y0)],
            [at(100.0, z0, y0), at(-100.0, z1, y1), at(100.0, z1, y1)],
        ] {
            let flags = TriangleFeature::ONE_SIDED | 0xE0;
            triangles.push(WorldTriangle::from_vertices(vertices, material, 42, flags, [1.0; 3], 0.0).expect("ramp triangle"));
        }
    }
    triangles
}

/// The stock board assembly on [`ramp`] for 20 s of 60 Hz ticks, pushed partway up and rolling
/// back: world wheel contacts, pushes, steering, the 25-iteration solver and integration, every
/// contact and body state hashed every tick.
fn core_board_ramp(d: &mut Digest) {
    board_ramp(d);
}

/// What [`core_board_ramp`] did, so a test can tell a real run from a degenerate one.
#[derive(Debug)]
struct RampRun {
    ticks_in_contact: u32,
    highest_deck: f32,
    final_deck: Vector3,
}

fn board_ramp(d: &mut Digest) -> RampRun {
    let gravity = Vector3::new(0.0, -9.8, 0.0);
    let simulation = RetailSimulationStep::fixed_60_hz(30, 0.001, gravity);
    let settings = BoardStepSettings {
        simulation,
        iterations: 25,
        base_truck_transforms: default_truck_transforms(),
        truck_dynamics: retail_truck_drive_dynamics(RetailTruckDriveSettings::STOCK),
        force_point_y_offset: 0.0,
    };
    let wheels = WheelWorldSettings {
        radius: RETAIL_WHEEL_RADIUS,
        material: RetailContactMaterial { static_friction: 0.2, dynamic_friction: 0.1, restitution: 0.8 },
        query: WorldContactSettings {
            volume_padding: 0.02,
            maximum_separating_distance: 0.05,
            edge_cos_bend_normal_threshold: -1.0,
            convexity_epsilon: 0.0,
            is_object: false,
        },
        retention: ContactRetentionSettings { capacity: 100, duplicate_distance_squared: 0.000_001, deferred_reduction: false },
    };
    let spawn = RetailAffineTransform {
        translation: Vector3::new(0.0, RETAIL_WHEEL_RADIUS - RETAIL_TRUCK_Y_POSITION, -8.0),
        ..RetailAffineTransform::IDENTITY
    };
    let mut board = BoardRuntime::new(
        default_skateboard_mass_properties(),
        authored_body_transforms(AuthoredTransformInputs::STOCK),
        spawn,
        simulation,
        BoardMotion::Active,
    );
    let inverse_masses = board.bodies().map(|b| b.inertia.inverse_mass);
    let mut world = BoardWorld::new(ramp());
    let mut run = RampRun { ticks_in_contact: 0, highest_deck: f32::MIN, final_deck: Vector3::ZERO };
    for tick in 0..1200u32 {
        board.clear_forces();
        let v = board.bodies()[BodyId::Deck.index()].rates.linear_velocity;
        let speed = v.x.mul_add(v.x, v.z * v.z).sqrt();
        if tick % 120 < 30 {
            let push = PushInput {
                flags_2468: 0x0200_0000,
                flags_2472: 0,
                target_speed: 6.0,
                current_speed: speed,
                absolute_body_speed: speed,
                scale: f32::NAN,
                delta_seconds: simulation.time_step,
                direction: Vector3::new(0.0, 0.0, 1.0),
            };
            let limits = PushLimits { maximum_pushable_speed: 10.0, low_speed_change: 0.1, high_speed_change: 0.05 };
            enqueue_push(push, limits, &inverse_masses, board.forces_mut());
        }
        let steer = 0.05 * skate_core::trigonometry::sin(tick as f32 * 0.03);
        let contacts = world.query(&board, wheels);
        d.debug(&contacts);
        run.ticks_in_contact += u32::from(!contacts.is_empty());
        board.advance(contacts, [steer, steer], settings);
        d.debug(board.bodies());
        run.final_deck = board.bodies()[BodyId::Deck.index()].rates.position;
        run.highest_deck = run.highest_deck.max(run.final_deck.y);
    }
    d.debug(&board.part_transforms());
    run
}

/// The per-sample `mul_add` loops alone: resampler, biquad kernel and the FSS polynomials, with
/// coefficients given as values (no libm anywhere).
fn audio_dsp_kernels(d: &mut Digest) {
    use dsp::biquad::{Coefficients, kernel};
    let mut rng = Lcg(7);
    let source: Vec<f32> = (0..1 << 16).map(|_| rng.range(-0.5, 0.5)).collect();
    let filters = [
        Coefficients { a1: -1.8, a2: 0.81, b0: 0.0025, b1: 0.005, b2: 0.0025 },
        Coefficients { a1: -1.9, a2: 0.95, b0: 0.025, b1: 0.0, b2: -0.025 },
    ];
    let mut history = [[0.0f32; 4]; 2];
    let mut resampler = dsp::resample::Resampler::default();
    let mut block = [0.0f32; skate_audio::BLOCK];
    for n in 0..4000u32 {
        resampler.set_ratio(0.25 + (n % 64) as f32 * 0.05);
        resampler.render(&mut block, |i| source[i as usize & 0xFFFF]);
        resampler.advance(block.len());
        d.f32s(&block);
        for (k, h) in filters.iter().zip(&mut history) {
            kernel(k, h, &mut block);
            d.f32s(&block);
        }
    }
    for _ in 0..1 << 18 {
        let x = rng.range(-4.0 * PI, 4.0 * PI);
        d.f32s(&[dsp::fss::sin(x), dsp::fss::cos(x)]);
    }
}

/// A voice through the production modules with moving parameters: the resampler, high/low-pass,
/// shelf and peaking filters (whose coefficient design calls libm in f64), delay, de-click gain,
/// frequency shifter and reverb, ten seconds at 48 kHz.
fn audio_voice_chain(d: &mut Digest) {
    use dsp::biquad::{Iir2, Kind as Pass};
    const RATE: f32 = 48_000.0;
    let mut rng = Lcg(11);
    let source: Vec<f32> = (0..1 << 16).map(|_| rng.range(-0.5, 0.5)).collect();
    let mut resampler = dsp::resample::Resampler::default();
    let mut low = Iir2::new(Pass::LowPass);
    let mut high = Iir2::new(Pass::HighPass);
    let mut shelf = dsp::shelf::HighShelfIir2::new(4_000.0, 0.5);
    let mut peak = dsp::peaking::PeakingIir2::default();
    (peak.freq, peak.gain, peak.q) = (900.0, 2.0, 1.5);
    let mut delay = dsp::delay::Delay::new(0.5, 2);
    (delay.delay, delay.feedback) = (0.21, 0.4);
    let mut gain = dsp::gain::Gain::default();
    let mut shift = dsp::fss::FrequencyShift::default();
    shift.shift_hz = 37.0;
    let mut reverb = dsp::reverb::ReverbModel1::default();
    (reverb.time, reverb.size, reverb.brightness) = (1.8, 0.6, 0.7);
    for n in 0..2000u32 {
        resampler.set_ratio(0.5 + (n % 50) as f32 * 0.03);
        low.cutoff = 200.0 + (n % 400) as f32 * 50.0;
        high.cutoff = 30.0 + (n % 97) as f32 * 3.0;
        gain.target = 0.5 + (n % 7) as f32 * 0.1;
        let mut left = [0.0f32; skate_audio::BLOCK];
        resampler.render(&mut left, |i| source[i as usize & 0xFFFF]);
        resampler.advance(left.len());
        let mut right = left;
        {
            let mut channels = [&mut left[..], &mut right[..]];
            low.process(&mut channels, RATE);
            high.process(&mut channels, RATE);
            shelf.process(&mut channels, RATE);
            peak.process(&mut channels, RATE);
            delay.process(&mut channels);
            gain.process(&mut channels);
        }
        shift.process(&mut left, RATE);
        reverb.process(&mut right);
        d.f32s(&left);
        d.f32s(&right);
    }
}

/// The MixMap tables are generated with f64 `powf` / `cos`, then rounded to integers or f32.
fn audio_mixmap_tables(d: &mut Digest) {
    d.debug(&skate_audio::mixmap::tables::Tables::generate());
}

fn data_format_demo(d: &mut Digest) {
    let map = skate_data::skate_map::SkateMap::parse(include_bytes!("../../../maps/format-demo.skate"))
        .expect("maps/format-demo.skate");
    d.debug(&map);
}

/// The C zstd (compiled for wasm through zstd-sys's shim) and flate2 round trips, 4 MiB.
fn data_codecs(d: &mut Digest) {
    let mut rng = Lcg(3);
    let raw: Vec<u8> = (0..4u32 << 20).map(|i| if i % 7 < 3 { (i >> 10) as u8 } else { (rng.next() >> 24) as u8 & 0x0F }).collect();
    let zstd = zstd::stream::encode_all(&raw[..], 3).expect("zstd encode");
    let unzstd = zstd::stream::decode_all(&zstd[..]).expect("zstd decode");
    assert!(unzstd == raw, "zstd round trip");
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&raw).expect("deflate encode");
    let deflate = encoder.finish().expect("deflate encode");
    let mut inflate = Vec::new();
    flate2::read::ZlibDecoder::new(&deflate[..]).read_to_end(&mut inflate).expect("deflate decode");
    assert!(inflate == raw, "deflate round trip");
    d.bytes(&zstd);
    d.bytes(&deflate);
    d.bytes(&unzstd);
}

/// Eight independent chains of 2^21 steps each (16.8 M operations), so throughput is measured.
fn chains(d: &mut Digest, op: impl Fn(f32, f32, f32) -> f32) {
    let mut acc: [f32; 8] = std::array::from_fn(|k| 0.5 + k as f32 * 0.0625);
    let add: [f32; 8] = std::array::from_fn(|k| 1e-3 * (k + 1) as f32);
    for _ in 0..1 << 21 {
        for k in 0..8 {
            acc[k] = op(acc[k], 0.999_9, add[k]);
        }
    }
    d.f32s(&acc);
}

fn cost_mul_add(d: &mut Digest) {
    chains(d, f32::mul_add);
}

/// For timing only: the engine never replaces `mul_add` with this (it rounds twice).
fn cost_mul_then_add(d: &mut Digest) {
    chains(d, |a, b, c| a * b + c);
}

const LIBM_SAMPLES: u32 = 1 << 18;

/// Pan, splice envelopes, camera: `f32::sin` / `cos` over two turns either side of zero.
fn libm_f32_sin(d: &mut Digest) {
    let mut rng = Lcg(21);
    for _ in 0..LIBM_SAMPLES {
        d.f32(rng.range(-4.0 * PI, 4.0 * PI).sin());
    }
}

fn libm_f32_cos(d: &mut Digest) {
    let mut rng = Lcg(21);
    for _ in 0..LIBM_SAMPLES {
        d.f32(rng.range(-4.0 * PI, 4.0 * PI).cos());
    }
}

/// skate-core's manual camera roll and skate-audio's player heading.
fn libm_f32_sin_cos(d: &mut Digest) {
    let mut rng = Lcg(22);
    for _ in 0..LIBM_SAMPLES {
        let (s, c) = rng.range(-4.0 * PI, 4.0 * PI).sin_cos();
        d.f32s(&[s, c]);
    }
}

/// skate-core's offboard air selector launch angles.
fn libm_f32_tan(d: &mut Digest) {
    let mut rng = Lcg(23);
    for _ in 0..LIBM_SAMPLES {
        d.f32(rng.range(-1.5, 1.5).tan());
    }
}

/// skate-core's `BoardAdjustAngle` (stick x over −y) and skate-audio's panner / object azimuth.
fn libm_f32_atan2(d: &mut Digest) {
    let mut rng = Lcg(24);
    for _ in 0..LIBM_SAMPLES {
        let (x, y) = (rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
        d.f32(x.atan2(-y));
    }
}

/// skate-audio's zone angle.
fn libm_f32_acos(d: &mut Digest) {
    let mut rng = Lcg(25);
    for _ in 0..LIBM_SAMPLES {
        d.f32(rng.range(-1.0, 1.0).acos());
    }
}

/// The audio filter design: ω in the biquad's floor…ceiling, `sin` / `cos` in f64, rounded once.
fn libm_f64_sin_cos_to_f32(d: &mut Digest) {
    let mut rng = Lcg(26);
    for _ in 0..LIBM_SAMPLES {
        let w = f64::from(rng.range(dsp::biquad::FLOOR, dsp::biquad::CEIL));
        d.f32s(&[w.sin() as f32, w.cos() as f32]);
    }
}

/// skate-core's flat-tetrahedron threshold, `size^1.5` in f64, over squared edge lengths from
/// 2^−40 to 2^14 (built from bits, so no other libm call feeds it).
fn libm_f64_powf(d: &mut Digest) {
    let mut rng = Lcg(27);
    for _ in 0..LIBM_SAMPLES {
        let exponent = u64::from(1023 - 40 + rng.next() % 54);
        let mantissa = u64::from(rng.next()) << 20;
        d.f64(f64::from_bits(exponent << 52 | mantissa).powf(1.5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_board_rolls_on_its_wheels_up_the_ramp_and_back() {
        let run = board_ramp(&mut Digest { hash: 0, capture: None });
        // Rests with the deck about 0.11 m up; pushed partway up the ramp, it rolls back down.
        assert!(run.ticks_in_contact > 1100, "{run:?}");
        assert!(run.highest_deck > 0.5, "{run:?}");
        assert!((0.0..1.2).contains(&run.final_deck.y) && run.final_deck.z > 0.0, "{run:?}");
    }

    #[test]
    fn sections_are_repeatable_and_uniquely_named() {
        for (i, section) in SECTIONS.iter().enumerate() {
            assert!(SECTIONS[..i].iter().all(|s| s.name != section.name), "{}", section.name);
        }
        let section = section("audio/dsp-kernels").unwrap();
        assert_eq!(run(section, false).0, run(section, false).0);
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use wasm_bindgen::prelude::wasm_bindgen;

    /// Every section as a `name kind` line, in run order.
    #[wasm_bindgen]
    pub fn sections() -> String {
        super::SECTIONS.iter().map(|s| format!("{} {}\n", s.name, s.kind.name())).collect()
    }

    /// One section's digest as 16 hex digits; empty for an unknown name.
    #[wasm_bindgen]
    pub fn run(name: &str) -> String {
        super::section(name).map_or_else(String::new, |s| format!("{:016x}", super::run(s, false).0))
    }

    /// One section's output words, to count libm differences against the native dump.
    #[wasm_bindgen]
    pub fn values(name: &str) -> Vec<u32> {
        super::section(name).map_or_else(Vec::new, |s| super::run(s, true).1)
    }
}
