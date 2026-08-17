//! Power a die on and let it settle.
//!
//!     cargo run --release --example settle -- extern/visual6502
//!
//! No clock layer, no pins, no notion of an instruction: just the switch network
//! finding a fixed point. If a chip converges here, the solver can carry it; if
//! it does not, that is a result worth having before building anything on top.

use std::path::PathBuf;
use std::sync::Arc;

use halfphi::{parse, ChipSource, Engine, Netlist, Rails};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(
        args.next()
            .ok_or("usage: settle <chip-dir> [--ground NAME]")?,
    );
    let mut ground = "vss".to_string();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--ground" => ground = args.next().ok_or("--ground needs a name")?,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }

    let read = |f: &str| std::fs::read_to_string(dir.join(f));
    let parsed = parse(&ChipSource {
        segdefs: &read("segdefs.js")?,
        transdefs: &read("transdefs.js")?,
        nodenames: &read("nodenames.js")?,
        rails: Rails {
            ground: &ground,
            supply: "vcc",
        },
    })?;
    let nl = Netlist::decode(&parsed.blob)?;

    let mut eng = Engine::new(Arc::new(nl));
    eng.force_power_on_state();
    eng.restore_layout_pulls();
    eng.settle_all();

    let s = eng.stats();
    println!("{}", dir.display());
    println!("  settles run:            {}", s.settles);
    println!("  node recalculations:    {}", s.node_recalcs);
    println!("  nonconvergent settles:  {}", s.nonconvergent_settles);
    println!("  contested groups:       {}", s.contested_groups);
    if s.nonconvergent_settles > 0 {
        // Not necessarily a bug in the chip or the solver: this performs no
        // chip-specific initialisation, and some dies need one.
        println!("\n  This die did not reach a fixed point from a cold power-on.");
    }

    let high = (0..eng.netlist().node_count())
        .filter(|&n| eng.netlist().exists(n as u16) && eng.is_high(n as u16))
        .count();
    println!("  {high} nodes settled high");
    Ok(())
}
