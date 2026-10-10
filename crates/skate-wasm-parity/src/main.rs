//! Native side of the wasm parity probe. Prints `name kind digest best_ms` per section (best of
//! three), after a `#` line naming the audio FMA path in use. `--values DIR` also writes each libm
//! section's output words to `DIR/<name>.bin` (little-endian u32, `/` → `-`) for
//! `tools/web-port/parity.mjs` to count differences against.
use std::path::PathBuf;
use std::time::Instant;

use skate_wasm_parity::{SECTIONS, run};

fn main() -> Result<(), String> {
    let mut values = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--values" => values = Some(PathBuf::from(args.next().ok_or("--values needs a directory")?)),
            other => return Err(format!("unknown argument {other:?}; usage: skate-wasm-parity [--values DIR]")),
        }
    }
    let fma = skate_audio_fma::init();
    println!("# native {} audio-fma={}", std::env::consts::ARCH, fma.path.name());
    for section in SECTIONS {
        let mut best = f64::INFINITY;
        let mut digest = 0;
        for _ in 0..3 {
            let started = Instant::now();
            digest = run(section, false).0;
            best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        }
        println!("{} {} {digest:016x} {best:.3}", section.name, section.kind.name());
        if let Some(dir) = values.as_ref().filter(|_| !section.kind.must_match()) {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = dir.join(format!("{}.bin", section.name.replace('/', "-")));
            let bytes: Vec<u8> = run(section, true).1.iter().flat_map(|w| w.to_le_bytes()).collect();
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}
