//! Switch-level simulation and analysis of a photographed die.
//!
//! # What this is
//!
//! A chip traced from photographs is a set of polygons, a set of transistors,
//! and some names. Nothing in it says what the chip *does*. This library takes
//! that description and gives you two things: a simulation in which behaviour is
//! an emergent property of switches opening and closing, and a set of analyses
//! that recover structure from the switch network rather than being told it.
//!
//! The name is the unit. A chip driven by a two-phase clock does work on both
//! edges, so the half-cycle -- half of phi -- is the smallest step that means
//! anything. Counting whole cycles loses half the story.
//!
//! # What this is not
//!
//! It is not a 6502 emulator, or an emulator of anything. There is no
//! instruction table here and no behavioural model. If you want to know what
//! `$69` does you have to run it and look.
//!
//! It also carries **no die data**. That is a licence boundary as much as a
//! design one: the visual6502 die data is CC BY-NC-SA 3.0, and NonCommercial
//! and ShareAlike propagate to anything that ships it. This crate is MIT and
//! stays MIT by holding none of it -- you supply the bytes. See NOTICE.md.
//!
//! # The shape of it
//!
//! ```text
//!   source::parse(&ChipSource)  ->  a netlist blob + geometry
//!   Netlist::decode(&blob)      ->  topology: nodes, transistors, names
//!   Engine::new(netlist)        ->  state: settle, read, drive
//! ```
//!
//! Each step is separable on purpose. The topology is immutable, shared and
//! cache-hot; the state is per-instance and mutable; and a caller that only
//! wants to *analyse* a chip never has to instantiate an engine at all.
//!
//! # Which chips
//!
//! Any die published in the visual6502 three-file form. The collection includes
//! the 6502, the 6800 and the Z80, and they differ in ways that a library about
//! one chip would have quietly baked in: the 6800 calls its ground rail `gnd`
//! rather than `vss`, and neither the 6800 nor the Z80 uses the 6502's full set
//! of mask layers. Rails are therefore a parameter, and layers are data.
//!
//! # What a chip layer has to add
//!
//! The library stops at the point where a switch network becomes a *processor*.
//! What a clock edge is, which nodes are pins, what counts as a register, and
//! how a bus handshake works are all facts about a particular chip, and they
//! belong in a crate that is honestly about that chip.

#![forbid(unsafe_code)]

pub mod engine;
pub mod netlist;
pub mod source;

pub use engine::{ChipState, Drive, Engine, Stats, MAX_SETTLE_ROUNDS};
pub use netlist::{BitSet, DecodeError, Netlist, NodeId, Terminal, TransId};
pub use source::{parse, ChipSource, Parsed, Polygon, Rails};
