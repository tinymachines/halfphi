//! What is in this die?
//!
//!     cargo run --example inspect -- extern/visual6502
//!     cargo run --example inspect -- extern/visual6502/chip-z80
//!     cargo run --example inspect -- extern/visual6502/chip-6800 --ground gnd
//!
//! Reads the three files, decodes the netlist, and reports what came back. This
//! is the smallest useful thing the library does and a good first check that a
//! chip you have obtained is readable at all.

use std::collections::BTreeMap;
use std::path::PathBuf;

use halfphi::{parse, ChipSource, Netlist, Rails};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().ok_or("usage: inspect <chip-dir> [--ground NAME]")?);
    let mut ground = "vss".to_string();
    let mut supply = "vcc".to_string();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--ground" => ground = args.next().ok_or("--ground needs a name")?,
            "--supply" => supply = args.next().ok_or("--supply needs a name")?,
            other => return Err(format!("unknown flag {other}").into()),
        }
    }

    let read = |f: &str| std::fs::read_to_string(dir.join(f));
    let parsed = parse(&ChipSource {
        segdefs: &read("segdefs.js")?,
        transdefs: &read("transdefs.js")?,
        nodenames: &read("nodenames.js")?,
        rails: Rails { ground: &ground, supply: &supply },
    })?;
    let nl = Netlist::decode(&parsed.blob)?;

    println!("{}", dir.display());
    println!("  {:>7} nodes", nl.node_count());
    println!("  {:>7} transistors", nl.transistor_count());
    println!("  {:>7} names", parsed.name_count);
    println!("  {:>7} polygons", parsed.polygons.len());
    println!("  rails: {ground} = node {}, {supply} = node {}", nl.vss(), nl.vcc());

    // Mask layers are data, not a constant: not every die in the collection uses
    // the same set, so anything drawing one has to ask rather than assume.
    let mut per_layer: BTreeMap<u8, usize> = BTreeMap::new();
    for p in &parsed.polygons {
        *per_layer.entry(p.layer).or_default() += 1;
    }
    println!("  layers present: {:?}", per_layer.keys().collect::<Vec<_>>());
    for (layer, n) in &per_layer {
        println!("    layer {layer}: {n} polygons");
    }

    if parsed.gated_by_supply > 0 {
        // Permanently on in silicon, permanently off in this model, because
        // group evaluation never crosses a rail. Worth shouting about.
        println!("  WARNING: {} transistors are gated by the supply rail", parsed.gated_by_supply);
    }

    // A pullup with nothing pulling down is a node that can only ever be high,
    // which is usually a rail tap and occasionally a sign the parse went wrong.
    let pullups = (0..nl.node_count()).filter(|&n| nl.pullups().get(n)).count();
    println!("  {pullups} nodes have a pullup");

    let mut named: Vec<_> = nl.names().map(|(s, _)| s).collect();
    named.sort_unstable();
    println!(
        "  first names on the die: {}",
        named.iter().take(12).cloned().collect::<Vec<_>>().join(" ")
    );
    Ok(())
}
