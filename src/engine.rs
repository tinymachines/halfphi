//! Chip-agnostic switch-level solver.
//!
//! The model: nodes are wires, transistors are switches controlled by a node.
//! A node's logic level is not a property of the node but of the *group* of
//! nodes currently shorted together through conducting transistors. Settling
//! means repeatedly rebuilding groups and propagating their resolved level
//! until nothing changes.
//!
//! There is no time, capacitance or drive strength here beyond the ordering in
//! [`Drive`]. That is the same abstraction the original visual6502 used, and it
//! is sufficient to reproduce the 6502 cycle-exactly -- but it is worth knowing
//! what is *not* modelled before trusting it for analogue-level questions.

use std::sync::Arc;

use crate::netlist::{BitSet, Netlist, NodeId, TransId};

/// Matches the reference implementation's loop limiter. A settle that needs
/// more rounds than this is oscillating; the reference silently gave up, we
/// count it in [`Stats::nonconvergent_settles`].
pub const MAX_SETTLE_ROUNDS: usize = 100;

/// How strongly a connected group of nodes is being driven, in ascending
/// precedence. Resolving a group means taking the maximum over its members.
///
/// `ChargedHigh` is the interesting one: it represents a group with no active
/// driver that nonetheless contains a node still holding charge from a previous
/// cycle. This is how dynamic logic (which the 6502 uses heavily) retains state
/// between clock phases. It is a crude stand-in for real charge storage -- there
/// is no decay and no capacitance ratio -- but the 6502's two-phase clock never
/// leaves a node floating long enough for that to matter.
///
/// `PullDown` outranks `PullUp`, decided by measurement (0.1.3): a group
/// holding both is a layout depletion load fighting an external drive or an
/// init-forced level, and the stronger side wins low. The 6502, 6800, Z80 and
/// 2C02 never form such a group (`Stats::contested_groups` is 0 across all
/// their tests, so the order is unobservable there); the 2A03's SO input
/// chain forms three of them at power-on, its reference resolves them low
/// (first match walking out from the driven seed), and with this order its
/// golden replays bit-exact with no exemptions. Before 0.1.3 the order was
/// the other way, an arbitrary choice nothing had ever exercised.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
#[repr(u8)]
pub enum Drive {
    /// Nothing in the group drives or holds a level.
    #[default]
    Floating = 0,
    /// No driver, but some member still holds a high charge.
    ChargedHigh = 1,
    PullUp = 2,
    PullDown = 3,
    Vcc = 4,
    Vss = 5,
}

impl Drive {
    /// The logic level this drive resolves to.
    #[inline]
    pub fn level(self) -> bool {
        matches!(self, Drive::Vcc | Drive::PullUp | Drive::ChargedHigh)
    }
}

/// All mutable electrical state of the chip. Cloning this is a full snapshot
/// (~1.1 KiB for the 6502); the netlist is shared and never copied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChipState {
    /// Logic level of each node.
    pub value: BitSet,
    /// Pullup per node. Mutable, not a netlist constant: driving the data bus
    /// works by flipping the pull on `db0..db7`.
    pub pullup: BitSet,
    /// Pulldown per node.
    pub pulldown: BitSet,
    /// Conducting state of each transistor.
    pub trans_on: BitSet,
}

impl ChipState {
    pub fn new(netlist: &Netlist) -> Self {
        ChipState {
            value: BitSet::new(netlist.node_count()),
            pullup: netlist.pullups().clone(),
            pulldown: BitSet::new(netlist.node_count()),
            trans_on: BitSet::new(netlist.transistor_count()),
        }
    }

    pub fn copy_from(&mut self, other: &ChipState) {
        self.value.copy_from(&other.value);
        self.pullup.copy_from(&other.pullup);
        self.pulldown.copy_from(&other.pulldown);
        self.trans_on.copy_from(&other.trans_on);
    }
}

/// Instrumentation. Free to collect and useful for both profiling and for
/// spotting model trouble (see `contested_groups`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub settles: u64,
    pub rounds: u64,
    pub node_recalcs: u64,
    pub group_members: u64,
    /// Settles that hit [`MAX_SETTLE_ROUNDS`] without converging.
    pub nonconvergent_settles: u64,
    /// Groups containing both a pullup and a pulldown. The resolution is
    /// well-defined (pulldown wins, 0.1.3) but such a group is a genuine
    /// electrical contention, so a nonzero count is worth explaining.
    pub contested_groups: u64,
    /// Groups joined to both rails where a marked member made the rails
    /// cancel (see `Netlist::set_rail_conflict_holds`). Zero on every chip
    /// whose netlist marks nothing.
    pub rail_conflict_holds: u64,
    /// Undriven groups where the area vote (`ChargeRule::AreaVote`) said
    /// low although a member held charge: exactly the groups on which the
    /// two charge rules disagree. Zero under `AnyHigh`.
    pub area_vote_lows: u64,
}

/// A record of every group search the solver performed, for analysis.
///
/// Behind the `probe` feature, and off by default even then, because the
/// engine's whole performance story is that a steady-state settle allocates
/// nothing and this allocates. A recalc IS a search: `build_group` walks the
/// conducting-transistor graph out from one seed, so recording the seed, the
/// members it reached and whether anything moved is a complete account of what
/// the solver looked at and what looking cost.
#[cfg(feature = "probe")]
#[derive(Default)]
pub struct Probe {
    /// Recording is opt-in even in a probe build: a caller usually wants to
    /// skip the reset sequence and start at a known place.
    pub on: bool,
    /// The node each recalc was seeded from.
    pub seed: Vec<NodeId>,
    /// Whether that recalc changed any member's stored level. False is the
    /// interesting case: the group was rebuilt to learn nothing.
    pub changed: Vec<bool>,
    /// Offset into `members` where each recalc's group begins.
    pub start: Vec<u32>,
    /// Every group's members, concatenated.
    pub members: Vec<NodeId>,
    /// Caller-supplied labels: (label, the recalc count when it was set).
    pub marks: Vec<(u64, u32)>,
}

#[cfg(feature = "probe")]
impl Probe {
    /// The members of recalc `i`.
    pub fn group(&self, i: usize) -> &[NodeId] {
        let a = self.start[i] as usize;
        let b = self.start.get(i + 1).map_or(self.members.len(), |&x| x as usize);
        &self.members[a..b]
    }
    pub fn len(&self) -> usize {
        self.seed.len()
    }
    pub fn is_empty(&self) -> bool {
        self.seed.is_empty()
    }
    /// Note where the caller is in its own units, half-cycles usually.
    pub fn mark(&mut self, label: u64) {
        self.marks.push((label, self.seed.len() as u32));
    }
}

/// The solver: a netlist plus its mutable state plus reusable scratch buffers.
pub struct Engine {
    netlist: Arc<Netlist>,
    state: ChipState,

    // Scratch, reused across settles so steady-state simulation does no allocation.
    current: Vec<NodeId>,
    next: Vec<NodeId>,
    queued: BitSet,
    /// Fixed-size scratch buffer, node_count + 1 long and never resized, with
    /// `group_len` saying how much of it is live. A `Vec` that grows by
    /// `push` cannot be filled branchlessly; see `build_group`.
    group: Vec<NodeId>,
    group_len: usize,
    in_group: BitSet,

    stats: Stats,
    #[cfg(feature = "probe")]
    probe: Probe,
}

impl Engine {
    pub fn new(netlist: Arc<Netlist>) -> Self {
        let state = ChipState::new(&netlist);
        let nodes = netlist.node_count();
        Engine {
            current: Vec::with_capacity(nodes),
            next: Vec::with_capacity(nodes),
            queued: BitSet::new(nodes),
            // Pre-filled rather than empty, and the length is carried
            // separately: `build_group` writes each candidate at `group_len`
            // whether or not it keeps it, so the slot must already exist.
            // node_count + 1 because the write happens before the decision.
            group: vec![0; nodes + 1],
            group_len: 0,
            in_group: BitSet::new(nodes),
            netlist,
            state,
            stats: Stats::default(),
            #[cfg(feature = "probe")]
            probe: Probe::default(),
        }
    }

    pub fn netlist(&self) -> &Netlist {
        &self.netlist
    }

    pub fn netlist_arc(&self) -> &Arc<Netlist> {
        &self.netlist
    }

    pub fn state(&self) -> &ChipState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut ChipState {
        &mut self.state
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    #[cfg(feature = "probe")]
    pub fn probe(&self) -> &Probe {
        &self.probe
    }

    #[cfg(feature = "probe")]
    pub fn probe_mut(&mut self) -> &mut Probe {
        &mut self.probe
    }

    pub fn reset_stats(&mut self) {
        self.stats = Stats::default();
    }

    #[inline]
    pub fn is_high(&self, n: NodeId) -> bool {
        self.state.value.get(n as usize)
    }

    /// Read a little-endian bus: `nodes[0]` is bit 0.
    #[inline]
    pub fn read_bus(&self, nodes: &[NodeId]) -> u32 {
        let mut v = 0u32;
        for (i, &n) in nodes.iter().enumerate() {
            if self.is_high(n) {
                v |= 1 << i;
            }
        }
        v
    }

    /// Drive a node high (pullup on, pulldown off) and settle.
    pub fn drive_high(&mut self, n: NodeId) {
        self.state.pullup.set(n as usize);
        self.state.pulldown.clear(n as usize);
        self.settle(&[n]);
    }

    /// Drive a node low (pulldown on, pullup off) and settle.
    pub fn drive_low(&mut self, n: NodeId) {
        self.state.pullup.clear(n as usize);
        self.state.pulldown.set(n as usize);
        self.settle(&[n]);
    }

    /// Set the pull on a node without settling. Use when driving several nodes
    /// as one event (a bus write), then settle once with all of them as seeds --
    /// settling per bit would let the chip see a half-updated bus.
    pub fn set_pull(&mut self, n: NodeId, high: bool) {
        self.state.pullup.put(n as usize, high);
        self.state.pulldown.put(n as usize, !high);
    }

    /// Restore every pull to the layout defaults. Not part of a warm reset --
    /// see `Cpu::power_cycle`.
    pub fn restore_layout_pulls(&mut self) {
        self.state.pullup.copy_from(self.netlist.pullups());
        self.state.pulldown.clear_all();
    }

    /// Force the whole chip to the power-on condition: every node low except
    /// vcc, every transistor off. Does not settle.
    pub fn force_power_on_state(&mut self) {
        self.state.value.clear_all();
        self.state.value.set(self.netlist.vcc() as usize);
        self.state.trans_on.clear_all();
    }

    /// Recalculate every node that exists. Used once after power-on, when no
    /// incremental seed set is meaningful.
    pub fn settle_all(&mut self) {
        let seeds: Vec<NodeId> = (0..self.netlist.node_count() as NodeId)
            .filter(|&n| self.netlist.exists(n) && !self.netlist.is_rail(n))
            .collect();
        self.settle(&seeds);
    }

    /// Propagate from `seeds` until the network reaches a fixed point.
    pub fn settle(&mut self, seeds: &[NodeId]) {
        let nl = Arc::clone(&self.netlist);
        self.stats.settles += 1;

        self.current.clear();
        self.current.extend_from_slice(seeds);

        for _ in 0..MAX_SETTLE_ROUNDS {
            if self.current.is_empty() {
                return;
            }
            self.stats.rounds += 1;
            self.next.clear();

            let mut i = 0;
            while i < self.current.len() {
                let n = self.current[i];
                i += 1;
                self.recalc_node(&nl, n);
            }

            // Un-queue before the swap: these nodes are about to become the
            // current list and must be re-queueable during the next round.
            self.queued.clear_only(&self.next);
            std::mem::swap(&mut self.current, &mut self.next);
        }

        self.stats.nonconvergent_settles += 1;
        self.current.clear();
    }

    fn recalc_node(&mut self, nl: &Netlist, n: NodeId) {
        // Rails are fixed points by definition; the reference skips them here
        // and so must we, or vss/vcc would fight the group that contains them.
        if nl.is_rail(n) {
            return;
        }
        self.stats.node_recalcs += 1;

        let level = self.build_group(nl, n).level();
        #[cfg(feature = "probe")]
        let mut touched = false;

        for gi in 0..self.group_len {
            let m = self.group[gi];
            // A rail is in the group for its drive, not to be driven. Without
            // this, a group joined to both rails resolves low and vcc's own
            // stored level goes low with it -- unobservable to the solver,
            // which reads a rail's identity rather than its state, but visible
            // to anything that reads levels out (the halfshot export showed
            // node 657 dipping on most opcode fetches). The reference has the
            // same write and the same blindness; its stateString prints the
            // rails as `g`/`v` without looking, which is why the golden test
            // never saw it either.
            if nl.is_rail(m) {
                continue;
            }
            if self.state.value.get(m as usize) == level {
                continue;
            }
            #[cfg(feature = "probe")]
            {
                touched = true;
            }
            self.state.value.put(m as usize, level);
            for &t in nl.gates_of(m) {
                if level {
                    self.transistor_on(nl, t);
                } else {
                    self.transistor_off(nl, t);
                }
            }
        }

        #[cfg(feature = "probe")]
        if self.probe.on {
            self.probe.seed.push(n);
            self.probe.changed.push(touched);
            self.probe.start.push(self.probe.members.len() as u32);
            self.probe.members.extend_from_slice(&self.group[..self.group_len]);
        }

        self.in_group.clear_only(&self.group[..self.group_len]);
    }

    /// Collect everything electrically joined to `n` through conducting
    /// transistors, accumulating the group's drive as we go.
    ///
    /// Traversal is iterative, using `group` as both the result and the work
    /// queue. Rails are recorded but not crossed -- vss and vcc connect to
    /// hundreds of transistors each, and walking through them would merge most
    /// of the chip into one group.
    fn build_group(&mut self, nl: &Netlist, n: NodeId) -> Drive {
        // Destructured so the buffer is provably a different object from the
        // bitsets it is filled from.
        let Engine { state, group, in_group, stats, group_len, .. } = self;
        group[0] = n;
        let mut len: usize = 1;
        in_group.set(n as usize);

        let (vss, vcc) = (nl.vss(), nl.vcc());
        let mut drive = Drive::Floating;
        let (mut saw_up, mut saw_down) = (false, false);
        let (mut saw_vss, mut saw_vcc) = (false, false);
        // The charge rule is a property of the netlist, so this branch is
        // perfectly predicted; the two sums cost two adds per member and
        // are read only when nothing drives the group.
        let area_vote = nl.charge_rule() == crate::netlist::ChargeRule::AreaVote;
        let (mut hi_area, mut lo_area) = (0.0f64, 0.0f64);

        let mut i = 0;
        while i < len {
            let m = group[i];
            i += 1;

            if m == vss {
                saw_vss = true;
                continue;
            }
            if m == vcc {
                saw_vcc = true;
                continue;
            }

            if state.pullup.get(m as usize) {
                drive = drive.max(Drive::PullUp);
                saw_up = true;
            }
            if state.pulldown.get(m as usize) {
                drive = drive.max(Drive::PullDown);
                saw_down = true;
            }
            if state.value.get(m as usize) {
                drive = drive.max(Drive::ChargedHigh);
            }
            if area_vote {
                if state.value.get(m as usize) {
                    hi_area += nl.node_area(m);
                } else {
                    lo_area += nl.node_area(m);
                }
            }

            // Branchless. The two conditions here -- "is this transistor
            // conducting" and "do I already have this node" -- are
            // unpredictable by construction, because which switches conduct is
            // the thing being simulated. The candidate is written whether or
            // not it is kept and the length advances by a bool, so neither
            // decision becomes a branch. The store is still bounds-checked,
            // but that branch is perfectly predicted: the buffer is
            // node_count + 1 long and `in_group` admits each node once, so
            // `len` never reaches the end.
            for term in nl.terminals_of(m) {
                let on = state.trans_on.get(term.transistor as usize);
                let fresh = in_group.insert_if(term.other as usize, on);
                group[len] = term.other;
                len += fresh as usize;
            }
        }
        *group_len = len;

        stats.group_members += len as u64;
        if saw_up && saw_down {
            stats.contested_groups += 1;
        }
        // Under the area vote an undriven group is high only if the charged
        // members out-weigh the uncharged ones (the 2C02 reference's rule);
        // `AnyHigh` above already made it high on any charged member. Counted
        // where the two would have disagreed.
        if area_vote && drive == Drive::ChargedHigh && hi_area <= lo_area {
            drive = Drive::Floating;
            stats.area_vote_lows += 1;
        }
        // Rails are folded here rather than in the loop so that the
        // rail-conflict hold has somewhere to stand: a group joined to BOTH
        // rails is a fight no clean resolution order describes, and where the
        // netlist has marked a member (see `Netlist::set_rail_conflict_holds`;
        // the 2C02's OAM data lines are the case that forced this, mirroring
        // its reference simulator's own special case), the rails cancel and
        // the fallback is the reference's own rule: pulls first, then the
        // AREA-WEIGHTED charge vote (the instrumented reference showed
        // eleven charged members losing to two large ones, so counting
        // heads is a different rule). The re-scan is confined to this rare
        // path; the hot loop above is untouched. Everywhere else the rails
        // fold in exactly as before, Vss outranking Vcc.
        if saw_vss && saw_vcc && nl.any_rail_conflict_hold(&group[..len]) {
            stats.rail_conflict_holds += 1;
            let mut held = Drive::Floating;
            let (mut hi_area, mut lo_area) = (0.0f64, 0.0f64);
            for &m in group[..len].iter() {
                if m == vss || m == vcc {
                    continue;
                }
                if state.pullup.get(m as usize) {
                    held = held.max(Drive::PullUp);
                }
                if state.pulldown.get(m as usize) {
                    held = held.max(Drive::PullDown);
                }
                if state.value.get(m as usize) {
                    hi_area += nl.node_area(m);
                } else {
                    lo_area += nl.node_area(m);
                }
            }
            if held == Drive::Floating && hi_area > lo_area {
                held = Drive::ChargedHigh;
            }
            drive = held;
        } else {
            if saw_vcc {
                drive = drive.max(Drive::Vcc);
            }
            if saw_vss {
                drive = drive.max(Drive::Vss);
            }
        }
        drive
    }

    #[inline]
    fn transistor_on(&mut self, nl: &Netlist, t: TransId) {
        if self.state.trans_on.test_and_set(t as usize) {
            return;
        }
        // Only c1 is queued. Closing the switch merges c1 and c2 into one
        // group, so recalculating from either end reaches both.
        self.queue(nl, nl.transistor_c1(t));
    }

    #[inline]
    fn transistor_off(&mut self, nl: &Netlist, t: TransId) {
        if !self.state.trans_on.get(t as usize) {
            return;
        }
        self.state.trans_on.clear(t as usize);
        // Opening the switch splits one group into two, so both ends need
        // re-evaluating independently. This asymmetry with `transistor_on` is
        // load-bearing, not an oversight.
        self.queue(nl, nl.transistor_c1(t));
        self.queue(nl, nl.transistor_c2(t));
    }

    #[inline]
    fn queue(&mut self, nl: &Netlist, n: NodeId) {
        if nl.is_rail(n) {
            return;
        }
        if !self.queued.test_and_set(n as usize) {
            self.next.push(n);
        }
    }

    /// Every node currently shorted to `n` through conducting transistors.
    ///
    /// This is the same traversal `settle` uses, exposed read-only for the UI:
    /// it answers "what is this wire actually connected to *right now*", which
    /// changes as transistors switch. Allocates, so it is for interaction, not
    /// the hot path.
    pub fn group_of(&self, n: NodeId) -> Vec<NodeId> {
        let nl = &self.netlist;
        if n as usize >= nl.node_count() || !nl.exists(n) {
            return Vec::new();
        }
        let mut group = vec![n];
        let mut seen = BitSet::new(nl.node_count());
        seen.set(n as usize);
        let mut i = 0;
        while i < group.len() {
            let m = group[i];
            i += 1;
            if nl.is_rail(m) {
                continue;
            }
            for term in nl.terminals_of(m) {
                if !self.state.trans_on.get(term.transistor as usize) {
                    continue;
                }
                if !seen.test_and_set(term.other as usize) {
                    group.push(term.other);
                }
            }
        }
        group
    }

    /// The reference's node-state encoding, for differential testing:
    /// `x` undefined, `g` ground, `v` vcc, `h` high, `l` low -- one char per
    /// node index, including gaps in the node array.
    pub fn state_string(&self) -> String {
        let nl = &self.netlist;
        (0..nl.node_count() as NodeId)
            .map(|n| {
                if !nl.exists(n) {
                    'x'
                } else if n == nl.vss() {
                    'g'
                } else if n == nl.vcc() {
                    'v'
                } else if self.is_high(n) {
                    'h'
                } else {
                    'l'
                }
            })
            .collect()
    }
}
