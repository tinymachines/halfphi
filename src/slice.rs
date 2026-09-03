//! The solver again, as a kernel: 64 independent machines in one instruction
//! stream, one per bit of a `u64`.
//!
//! `engine::Engine` is queue-driven and materialises groups. That is the right
//! shape for ONE machine and the wrong shape for many: the queue is data
//! dependent, so two machines cannot share a control path, and its inner loop
//! branches on "is this transistor conducting", which is the thing being
//! simulated and therefore unpredictable.
//!
//! This is the same fixed point computed the other way. Instead of collecting
//! a group and taking the maximum drive over it, every node repeatedly takes
//! the maximum drive of its conducting neighbours until nothing moves. The two
//! agree because a group's level IS the maximum over its members, and
//! relaxation reaches the same maximum in as many rounds as the group is wide
//! (measured mean group: 2.03 nodes, so: few).
//!
//! Two properties make it bit-sliceable, and they are the whole point:
//!
//! - **The drive lattice is encoded as a thermometer, so `max` is `|`.**
//!   `Floating < ChargedHigh < PullUp < PullDown < Vcc < Vss` becomes five
//!   planes, plane `k` meaning "at least level k+1". The maximum of two drives
//!   is then the bitwise OR of their planes, which is the same instruction for
//!   all 64 machines at once.
//! - **Nothing branches on machine state.** "Is this transistor conducting"
//!   becomes a mask (`& on`), not an `if`. Every machine executes every step;
//!   the ones for which it is a no-op OR in zero.
//!
//! It is deliberately kernel-shaped -- flat arrays, a fixed sweep over all
//! nodes and all transistors, no queue, no early exit per machine -- because
//! that is what ports to a GPU compute shader without being redesigned. A
//! `u64` lane here is a `u32` lane there, and the sweep is the dispatch.
//!
//! Cost, per machine, is higher than the scalar engine's and the parallelism
//! pays for it many times over: this does work proportional to the whole die
//! every round, where the queue touches ~900 nodes, but it does it for 64
//! machines in the same instructions.

use crate::netlist::{Netlist, NodeId, TransId};

/// Machines per word. One bit of every `u64` in [`SliceState`] belongs to one
/// machine, and lane `k` never influences lane `j`.
pub const LANES: usize = 64;

/// How many drive levels above `Floating` there are, and therefore how many
/// thermometer planes it takes to encode one. See the module note.
const PLANES: usize = 5;

const P_CHARGED: usize = 0;
const P_PULLUP: usize = 1;
const P_PULLDOWN: usize = 2;
const P_VCC: usize = 3;
const P_VSS: usize = 4;

/// How a transistor's two ends may exchange drive.
///
/// A rail is recorded as drive but never crossed: vss alone is an end of 2493
/// transistors, and walking through it would merge most of the die into one
/// group. `engine::build_group` enforces that by giving rails no adjacency at
/// all; here it is a per-transistor direction, computed once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Flow {
    /// Ordinary: drive moves both ways when the transistor conducts.
    Both,
    /// `c1` is a rail: it drives `c2` and nothing flows back.
    C1ToC2,
    /// `c2` is a rail: it drives `c1` and nothing flows back.
    C2ToC1,
    /// Both ends are rails, or the transistor can never conduct. Skipped.
    None,
}

/// The netlist, arranged for the sweep. Built once and shared.
pub struct SliceNetlist {
    node_count: usize,
    trans_count: usize,
    gate: Vec<NodeId>,
    c1: Vec<NodeId>,
    c2: Vec<NodeId>,
    flow: Vec<Flow>,
    /// Static per-node facts, hoisted out of the inner loop.
    is_vss: Vec<bool>,
    is_vcc: Vec<bool>,
    is_rail: Vec<bool>,
    exists: Vec<bool>,
}

impl SliceNetlist {
    pub fn new(nl: &Netlist) -> Self {
        let (n, t) = (nl.node_count(), nl.transistor_count());
        let (vss, vcc) = (nl.vss(), nl.vcc());
        let mut s = SliceNetlist {
            node_count: n,
            trans_count: t,
            gate: Vec::with_capacity(t),
            c1: Vec::with_capacity(t),
            c2: Vec::with_capacity(t),
            flow: Vec::with_capacity(t),
            is_vss: (0..n).map(|i| i as NodeId == vss).collect(),
            is_vcc: (0..n).map(|i| i as NodeId == vcc).collect(),
            is_rail: (0..n).map(|i| nl.is_rail(i as NodeId)).collect(),
            exists: (0..n).map(|i| nl.exists(i as NodeId)).collect(),
        };
        for ti in 0..t as TransId {
            let (g, a, b) = (nl.transistor_gate(ti), nl.transistor_c1(ti), nl.transistor_c2(ti));
            let (ra, rb) = (nl.is_rail(a), nl.is_rail(b));
            // A transistor gated by vss can never conduct: vss is low by
            // identity, so its gate never rises. 17 of them on the 6502, and
            // dropping them here is not an optimisation but the same
            // permanently-off fact the scalar model records.
            let dead = g == vss;
            s.flow.push(match (ra, rb, dead) {
                (_, _, true) => Flow::None,
                (true, true, _) => Flow::None,
                (true, false, _) => Flow::C1ToC2,
                (false, true, _) => Flow::C2ToC1,
                (false, false, _) => Flow::Both,
            });
            s.gate.push(g);
            s.c1.push(a);
            s.c2.push(b);
        }
        s
    }
}

/// 64 machines' worth of mutable electrical state, bit-sliced.
///
/// Every array is indexed by node (or transistor) and holds one `u64` whose
/// bit `k` is that machine's value. Cloning is a snapshot of all 64.
#[derive(Clone)]
pub struct SliceState {
    pub value: Vec<u64>,
    pub pullup: Vec<u64>,
    pub pulldown: Vec<u64>,
    pub trans_on: Vec<u64>,
    /// Scratch: the thermometer planes, rebuilt every round.
    planes: [Vec<u64>; PLANES],
    next: Vec<u64>,
}

impl SliceState {
    pub fn new(snl: &SliceNetlist) -> Self {
        let (n, t) = (snl.node_count, snl.trans_count);
        SliceState {
            value: vec![0; n],
            pullup: vec![0; n],
            pulldown: vec![0; n],
            trans_on: vec![0; t],
            planes: std::array::from_fn(|_| vec![0; n]),
            next: vec![0; n],
        }
    }

    /// Put machine `lane` into the state of a scalar [`crate::ChipState`].
    ///
    /// This is how the oracle is set up: run the real engine, copy it into a
    /// lane, and the two must then agree half-cycle for half-cycle.
    pub fn load_lane(&mut self, lane: usize, st: &crate::ChipState) {
        let bit = 1u64 << lane;
        for i in 0..self.value.len() {
            put(&mut self.value, i, bit, st.value.get(i));
            put(&mut self.pullup, i, bit, st.pullup.get(i));
            put(&mut self.pulldown, i, bit, st.pulldown.get(i));
        }
        for i in 0..self.trans_on.len() {
            put(&mut self.trans_on, i, bit, st.trans_on.get(i));
        }
    }

    #[inline]
    pub fn is_high(&self, lane: usize, n: NodeId) -> bool {
        self.value[n as usize] >> lane & 1 != 0
    }

    /// Set a node's pull in one lane, the bit-sliced `drive_high`/`drive_low`.
    pub fn set_pull(&mut self, lane: usize, n: NodeId, high: bool) {
        let bit = 1u64 << lane;
        put(&mut self.pullup, n as usize, bit, high);
        put(&mut self.pulldown, n as usize, bit, !high);
    }

    /// Set a node's pull in EVERY lane at once, which is the point of the
    /// thing: 64 machines take the same clock edge in one instruction.
    pub fn set_pull_all(&mut self, n: NodeId, high: bool) {
        self.pullup[n as usize] = if high { !0 } else { 0 };
        self.pulldown[n as usize] = if high { 0 } else { !0 };
    }

    /// Drive all 64 lanes' copy of a node from a per-lane mask.
    pub fn set_pull_mask(&mut self, n: NodeId, high_mask: u64) {
        self.pullup[n as usize] = high_mask;
        self.pulldown[n as usize] = !high_mask;
    }

    /// Set a node's pull in the lanes named by `lanes`, leaving the rest
    /// alone. This is how a bus read is serviced: the CPU only drives the data
    /// bus in the lanes whose machine is reading, and the others keep whatever
    /// the chip itself is putting there.
    pub fn set_pull_where(&mut self, n: NodeId, lanes: u64, high: u64) {
        let i = n as usize;
        self.pullup[i] = (self.pullup[i] & !lanes) | (high & lanes);
        self.pulldown[i] = (self.pulldown[i] & !lanes) | (!high & lanes);
    }

    /// Read one lane's value off a bus of nodes, LSB first.
    pub fn read_bus(&self, lane: usize, bus: &[NodeId]) -> u32 {
        let mut v = 0u32;
        for (i, &n) in bus.iter().enumerate() {
            v |= (self.is_high(lane, n) as u32) << i;
        }
        v
    }

    /// Relax to a fixed point. Returns the number of rounds taken, which is a
    /// diagnostic and not a per-machine number: a round is a round for all 64.
    pub fn settle(&mut self, snl: &SliceNetlist, max_rounds: usize) -> usize {
        for round in 1..=max_rounds {
            // 0. Switches follow their gates BEFORE anything propagates. The
            //    scalar engine maintains this continuously -- a level change
            //    toggles that node's gated transistors immediately -- so a
            //    round that propagated through last round's switch positions
            //    would be reading a configuration the chip was never in. That
            //    matters here and not in a static circuit: a node that is
            //    briefly joined to a driver keeps the level afterwards, so a
            //    transient this kernel invents does not wash out, it is
            //    remembered as charge.
            let mut sw = 0u64;
            for ti in 0..snl.trans_count {
                let g = self.value[snl.gate[ti] as usize];
                sw |= g ^ self.trans_on[ti];
                self.trans_on[ti] = g;
            }

            // 1. Every node's own contribution to its group's drive. A rail
            //    pins its plane and is never written back in step 3.
            for i in 0..snl.node_count {
                let (pu, pd, v) = (self.pullup[i], self.pulldown[i], self.value[i]);
                let (vss, vcc) = (self.is_vss_mask(snl, i), self.is_vcc_mask(snl, i));
                // Thermometer: plane k is "at least level k+1", so a higher
                // drive lights every plane below it.
                self.planes[P_VSS][i] = vss;
                self.planes[P_VCC][i] = vss | vcc;
                self.planes[P_PULLDOWN][i] = vss | vcc | pd;
                self.planes[P_PULLUP][i] = vss | vcc | pd | pu;
                self.planes[P_CHARGED][i] = vss | vcc | pd | pu | v;
            }

            // 2. Spread drive across conducting transistors until the planes
            //    stop moving. This is the transitive closure the scalar engine
            //    gets by walking a group, done as a relaxation so that all 64
            //    machines take it together.
            let mut spread = 0;
            loop {
                let mut moved = 0u64;
                for ti in 0..snl.trans_count {
                    let on = self.trans_on[ti];
                    if on == 0 {
                        continue; // no machine has this switch closed
                    }
                    let flow = snl.flow[ti];
                    if flow == Flow::None {
                        continue;
                    }
                    let (a, b) = (snl.c1[ti] as usize, snl.c2[ti] as usize);
                    for p in 0..PLANES {
                        let (pa, pb) = (self.planes[p][a], self.planes[p][b]);
                        if flow != Flow::C2ToC1 {
                            let new_b = pb | (pa & on);
                            moved |= new_b ^ pb;
                            self.planes[p][b] = new_b;
                        }
                        if flow != Flow::C1ToC2 {
                            let new_a = pa | (pb & on);
                            moved |= new_a ^ pa;
                            self.planes[p][a] = new_a;
                        }
                    }
                }
                spread += 1;
                if moved == 0 || spread >= max_rounds {
                    break;
                }
            }

            // 3. Resolve. `Drive::level()` is high for Vcc, PullUp and
            //    ChargedHigh, which in thermometer form is "exactly at that
            //    level", i.e. this plane set and the one above clear.
            let mut changed = 0u64;
            for i in 0..snl.node_count {
                if snl.is_rail[i] || !snl.exists[i] {
                    self.next[i] = self.value[i];
                    continue;
                }
                let (c, pu, pd, vcc, vss) = (
                    self.planes[P_CHARGED][i],
                    self.planes[P_PULLUP][i],
                    self.planes[P_PULLDOWN][i],
                    self.planes[P_VCC][i],
                    self.planes[P_VSS][i],
                );
                let high = (c & !pu) | (pu & !pd) | (vcc & !vss);
                changed |= high ^ self.value[i];
                self.next[i] = high;
            }
            self.value.copy_from_slice(&self.next);

            if changed == 0 && sw == 0 {
                return round;
            }
        }
        max_rounds
    }

    #[inline]
    fn is_vss_mask(&self, snl: &SliceNetlist, i: usize) -> u64 {
        if snl.is_vss[i] {
            !0
        } else {
            0
        }
    }

    #[inline]
    fn is_vcc_mask(&self, snl: &SliceNetlist, i: usize) -> u64 {
        if snl.is_vcc[i] {
            !0
        } else {
            0
        }
    }
}

#[inline]
fn put(words: &mut [u64], i: usize, bit: u64, v: bool) {
    if v {
        words[i] |= bit;
    } else {
        words[i] &= !bit;
    }
}
