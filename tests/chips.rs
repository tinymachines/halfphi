//! Three chips, one API.
//!
//! This is the test that decides whether the library is about switch networks
//! or about the 6502 with extra steps. The visual6502 collection publishes the
//! 6800 and the Z80 in the same three-file form, and nothing in `halfphi` was
//! written with either of them in mind -- so if they load, settle and analyse
//! through exactly the same calls, the API is canonical in the only sense that
//! can be checked.
//!
//! It SKIPS when `extern/visual6502` is absent, the same way the golden trace
//! test does: the die data is a submodule and is not this repository's to
//! redistribute. Set `HALFPHI_REQUIRE_CHIPS=1` to make its absence a failure
//! instead, which is what CI should do.

use std::path::{Path, PathBuf};

use halfphi::{parse, ChipSource, Engine, Netlist, Rails};

/// Where the die traces are, if they are anywhere.
///
/// Two candidates, because this file is shared verbatim between the standalone
/// `halfphi` repository (where the submodule sits beside the crate) and the
/// `tinymachines/6502` monorepo (where the crate is two levels down). Searching
/// rather than hardcoding is what lets the two copies stay byte-identical, which
/// is the only thing stopping them drifting apart.
fn refdir() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    ["extern/visual6502", "../../extern/visual6502"]
        .iter()
        .map(|p| root.join(p))
        .find(|d| d.join("segdefs.js").exists())
}

fn require() -> bool {
    std::env::var("HALFPHI_REQUIRE_CHIPS").is_ok()
}

struct Chip {
    name: &'static str,
    dir: &'static str,
    rails: Rails<'static>,
}

/// The three dies, and the one place they visibly disagree.
const CHIPS: &[Chip] = &[
    Chip { name: "6502", dir: ".", rails: Rails { ground: "vss", supply: "vcc" } },
    // Not `vss`. This is the whole reason rails are a parameter, and a library
    // that hardcoded the 6502's spelling would fail here with "no vss in
    // nodenames" while looking perfectly general.
    Chip { name: "6800", dir: "chip-6800", rails: Rails { ground: "gnd", supply: "vcc" } },
    Chip { name: "z80", dir: "chip-z80", rails: Rails { ground: "vss", supply: "vcc" } },
];

fn load(base: &Path, chip: &Chip) -> (Netlist, halfphi::Parsed) {
    let d = base.join(chip.dir);
    let read = |f: &str| std::fs::read_to_string(d.join(f)).expect(f);
    let parsed = parse(&ChipSource {
        segdefs: &read("segdefs.js"),
        transdefs: &read("transdefs.js"),
        nodenames: &read("nodenames.js"),
        rails: chip.rails,
    })
    .unwrap_or_else(|e| panic!("{}: {e}", chip.name));
    let nl = Netlist::decode(&parsed.blob).expect("decode");
    (nl, parsed)
}

#[test]
fn every_published_die_loads_through_the_same_calls() {
    let Some(base) = refdir() else {
        assert!(!require(), "extern/visual6502 missing and HALFPHI_REQUIRE_CHIPS is set");
        eprintln!("SKIP: extern/visual6502 not present");
        return;
    };

    for chip in CHIPS {
        let (nl, p) = load(&base, chip);
        assert!(nl.transistor_count() > 1000, "{}: too few transistors", chip.name);
        assert!(nl.node_count() > 500, "{}: too few nodes", chip.name);
        assert_ne!(nl.vss(), nl.vcc(), "{}: rails collapsed onto one node", chip.name);
        assert!(nl.is_rail(nl.vss()) && nl.is_rail(nl.vcc()), "{}: rails not rails", chip.name);
        // Geometry comes back too, and every polygon must belong to a node that
        // the netlist agrees exists -- the cheapest check that the two halves of
        // the parse describe the same die.
        for poly in p.polygons.iter().take(2000) {
            assert!(nl.exists(poly.node), "{}: polygon on a node that does not exist", chip.name);
        }
        eprintln!(
            "{:5} {:5} nodes {:5} transistors {:5} names {:6} polygons  rails {}/{}",
            chip.name,
            nl.node_count(),
            nl.transistor_count(),
            p.name_count,
            p.polygons.len(),
            chip.rails.ground,
            chip.rails.supply,
        );
    }
}

/// The 6502's figures, which this repository knows independently.
///
/// The other two are unverified against any outside source, so they are only
/// checked for shape. This one is checked for value: it is what stops a parser
/// change from silently altering every chip at once.
#[test]
fn the_6502_still_parses_to_the_numbers_we_know() {
    let Some(base) = refdir() else {
        assert!(!require());
        eprintln!("SKIP: extern/visual6502 not present");
        return;
    };
    let (nl, p) = load(&base, &CHIPS[0]);
    assert_eq!(nl.node_count(), 1725);
    assert_eq!(nl.transistor_count(), 3510);
    assert_eq!(p.polygons.len(), 8233);
    assert_eq!(nl.node("vss"), Some(nl.vss()));
    // 17 transistors are gated by ground and so are permanently off, which is
    // physically correct. None are gated by the supply, which would be
    // permanently on in silicon and off in this model.
    assert_eq!(p.gated_by_supply, 0);
}

/// A netlist is not a simulation until something settles.
///
/// Running the engine on a chip nobody wrote a chip-layer for is the point: it
/// needs no notion of a clock, a pin or an instruction. What comes back is a
/// measurement rather than a pass mark, so the per-chip results are written
/// down: a change in any of them is a change in the solver and should be seen.
///
/// All three converge from a cold power-on, and the Z80 did not always. Until
/// the solver stopped writing a group's level back into a rail it had reached,
/// vcc's stored level bounced with whichever group touched it, and each bounce
/// switched the 32 Z80 transistors that are gated by vcc (the 6502 has none),
/// which never settled within the hundred rounds the reference also caps at.
/// That was recorded here as "did NOT converge" and attributed to the missing
/// per-chip `support.js`; the attribution was wrong, and this table is what
/// caught the change of mind. The engine still runs a die twice the size of the
/// one it was developed against with no chip-specific setup, and the
/// non-convergence counter is still what would say so if that stopped.
#[test]
fn the_engine_runs_a_chip_it_knows_nothing_about() {
    let Some(base) = refdir() else {
        assert!(!require());
        eprintln!("SKIP: extern/visual6502 not present");
        return;
    };
    // Converges from a cold power-on, with no chip-specific setup?
    let expected = [("6502", true), ("6800", true), ("z80", true)];

    for (chip, converges) in expected {
        let c = CHIPS.iter().find(|x| x.name == chip).unwrap();
        let (nl, _) = load(&base, c);
        let mut eng = Engine::new(std::sync::Arc::new(nl));
        eng.force_power_on_state();
        eng.restore_layout_pulls();
        eng.settle_all();
        let cold = eng.stats().nonconvergent_settles;
        assert_eq!(
            cold == 0,
            converges,
            "{chip}: power-on convergence changed (nonconvergent settles: {cold})"
        );
        // The PullDown-over-PullUp order (0.1.3) is unobservable on these
        // three chips because they never form a contested group. The 2A03
        // is the chip that does, and its golden is the order's oracle; if
        // this count ever goes nonzero here, that golden is the first
        // thing to re-run.
        assert_eq!(
            eng.stats().contested_groups,
            0,
            "{chip}: a contested group appeared; the drive-order choice is now observable here"
        );
        eprintln!(
            "{:5} power-on: {:14} {} settles run",
            chip,
            if cold == 0 { "converged" } else { "did NOT converge" },
            eng.stats().settles
        );
    }
}
