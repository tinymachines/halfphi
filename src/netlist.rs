//! The netlist: which nodes exist, which transistors join them, what they are
//! called. Immutable, and holding no simulation state.
//!
//! Adjacency is CSR (compressed sparse row): one flat index array plus per-node
//! offsets, rather than an array-of-arrays. For a chip the size of a 6502 the
//! whole structure is ~90 KiB and stays in L2 during a run; the pointer chase
//! per node was the dominant cost in the original JavaScript.
//!
//! Nothing here names a chip. The rails arrive as node numbers in the decoded
//! blob rather than as the strings `vss` and `vcc`, which matters more than it
//! looks: the 6800's ground rail is called `gnd`.

use std::collections::HashMap;

/// Index of a node (a set of electrically-joined polygons on the die).
pub type NodeId = u16;
/// Index of a transistor.
pub type TransId = u16;

/// A terminal connection: the transistor, and the node on its *other* side.
///
/// Precomputing `other` removes a compare-and-branch from the innermost loop of
/// group traversal, which runs tens of millions of times per simulated second.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Terminal {
    pub transistor: TransId,
    pub other: NodeId,
}

/// Fixed-capacity bit vector.
///
/// `clear_only(indices)` exists because the hot loops set a handful of bits out
/// of ~1700 and then need them cleared; zeroing the whole word array each time
/// costs more than the work being done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitSet {
    words: Box<[u64]>,
    len: usize,
}

impl BitSet {
    pub fn new(len: usize) -> Self {
        BitSet { words: vec![0u64; len.div_ceil(64)].into_boxed_slice(), len }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, i: usize) -> bool {
        debug_assert!(i < self.len);
        (self.words[i >> 6] >> (i & 63)) & 1 != 0
    }

    #[inline]
    pub fn set(&mut self, i: usize) {
        debug_assert!(i < self.len);
        self.words[i >> 6] |= 1 << (i & 63);
    }

    #[inline]
    pub fn clear(&mut self, i: usize) {
        debug_assert!(i < self.len);
        self.words[i >> 6] &= !(1 << (i & 63));
    }

    #[inline]
    pub fn put(&mut self, i: usize, v: bool) {
        if v {
            self.set(i)
        } else {
            self.clear(i)
        }
    }

    /// Set the bit only if `cond`, returning whether THIS call set it.
    ///
    /// Branchless on purpose, and the purpose is measured: the caller is
    /// `build_group`'s terminal loop, where `cond` is "this transistor is
    /// conducting" and the bit is "I already have this node". Neither has a
    /// pattern a branch predictor can learn, because which switches conduct is
    /// the thing being simulated. `0u64.wrapping_sub(x as u64)` is the usual
    /// all-ones-or-zero mask; one load and one store to the same word, where
    /// a `get` then a `set` would touch it twice.
    #[inline]
    pub fn insert_if(&mut self, i: usize, cond: bool) -> bool {
        debug_assert!(i < self.len);
        let (w, b) = (i >> 6, 1u64 << (i & 63));
        let word = self.words[w];
        let fresh = (word & b == 0) & cond;
        self.words[w] = word | (b & 0u64.wrapping_sub(fresh as u64));
        fresh
    }

    /// Set a bit, returning whether it was already set.
    #[inline]
    pub fn test_and_set(&mut self, i: usize) -> bool {
        debug_assert!(i < self.len);
        let (w, m) = (i >> 6, 1u64 << (i & 63));
        let was = self.words[w] & m != 0;
        self.words[w] |= m;
        was
    }

    pub fn clear_all(&mut self) {
        self.words.fill(0);
    }

    /// Clear just these indices -- O(n) in the list, not in the set.
    #[inline]
    pub fn clear_only(&mut self, indices: &[NodeId]) {
        for &i in indices {
            self.clear(i as usize);
        }
    }

    pub fn count_ones(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }

    pub fn as_words(&self) -> &[u64] {
        &self.words
    }

    pub fn copy_from(&mut self, other: &BitSet) {
        debug_assert_eq!(self.len, other.len);
        self.words.copy_from_slice(&other.words);
    }
}

/// A chip's topology: nodes, transistors, and the two adjacency lists the
/// solver walks.
///
/// This crate embeds no die data and names no chip. The sentence that used to
/// sit here said "the 6502 netlist, decoded from the embedded blob", which was
/// true of `v6502-netlist` (where this type was born, and which really does
/// `include_bytes!` a blob) and is the opposite of what makes THIS crate MIT.
/// A netlist arrives through `Netlist::decode` or the parser in `source`.
#[derive(Clone, Debug)]
pub struct Netlist {
    node_count: usize,
    vss: NodeId,
    vcc: NodeId,

    exists: BitSet,
    pullup: BitSet,

    trans_gate: Box<[NodeId]>,
    trans_c1: Box<[NodeId]>,
    trans_c2: Box<[NodeId]>,

    // CSR: node -> transistors whose gate this node drives
    gate_offsets: Box<[u32]>,
    gate_index: Box<[TransId]>,

    // CSR: node -> terminals (transistor + far-side node)
    term_offsets: Box<[u32]>,
    term_index: Box<[Terminal]>,

    names: HashMap<Box<str>, NodeId>,
    node_names: Box<[Option<Box<str>>]>,

    /// Nodes whose groups, when joined to BOTH rails at once, resolve by
    /// their pulls and held charge instead of by either rail: the
    /// rail-conflict hold. Empty by default; a chip crate that knows its
    /// reference simulator applies such a rule (the 2C02's OAM data lines
    /// are the case that forced this) sets it after decoding. The engine
    /// consults it only on the both-rails path, which is rare by
    /// construction.
    rail_conflict_holds: BitSet,

    /// Per-node areas in the reference's own units (twice the shoelace
    /// area, unhalved, summed over a node's polygons), for the hold's
    /// charge vote. Empty unless a chip crate supplies them; required
    /// before any hold is marked, because a vote without weights is a
    /// different rule.
    node_areas: Box<[f64]>,
}

#[derive(Debug)]
pub enum DecodeError {
    BadMagic,
    Truncated,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::BadMagic => write!(f, "netlist blob has wrong magic"),
            DecodeError::Truncated => write!(f, "netlist blob is truncated"),
        }
    }
}

impl std::error::Error for DecodeError {}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let s = self.b.get(self.i..self.i + n).ok_or(DecodeError::Truncated)?;
        self.i += n;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, DecodeError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, DecodeError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn bits(&mut self, len: usize) -> Result<BitSet, DecodeError> {
        let bytes = self.take(len.div_ceil(8))?;
        let mut set = BitSet::new(len);
        for (bi, &byte) in bytes.iter().enumerate() {
            for k in 0..8 {
                let idx = bi * 8 + k;
                if idx < len && byte >> k & 1 != 0 {
                    set.set(idx);
                }
            }
        }
        Ok(set)
    }
}

impl Netlist {
    pub fn decode(blob: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader { b: blob, i: 0 };
        if r.take(8)? != b"HALFPHI1" {
            return Err(DecodeError::BadMagic);
        }
        let node_count = r.u32()? as usize;
        let trans_count = r.u32()? as usize;
        let name_count = r.u32()? as usize;
        let vss = r.u32()? as NodeId;
        let vcc = r.u32()? as NodeId;

        let exists = r.bits(node_count)?;
        let pullup = r.bits(node_count)?;

        let mut trans_gate = Vec::with_capacity(trans_count);
        let mut trans_c1 = Vec::with_capacity(trans_count);
        let mut trans_c2 = Vec::with_capacity(trans_count);
        for _ in 0..trans_count {
            trans_gate.push(r.u16()?);
            trans_c1.push(r.u16()?);
            trans_c2.push(r.u16()?);
        }

        let mut names = HashMap::with_capacity(name_count);
        let mut node_names: Vec<Option<Box<str>>> = vec![None; node_count];
        for _ in 0..name_count {
            let len = r.u16()? as usize;
            let name = String::from_utf8_lossy(r.take(len)?).into_owned();
            let node = r.i32()?;
            // `p5: -1` and friends: the status register has no bit 5, so the
            // reference records a sentinel. Keep the name out of the map rather
            // than inventing a node for it.
            if node < 0 || node as usize >= node_count {
                continue;
            }
            let node = node as NodeId;
            node_names[node as usize].get_or_insert_with(|| name.clone().into_boxed_str());
            names.insert(name.into_boxed_str(), node);
        }

        // --- Build CSR adjacency by counting, prefix-summing, then filling. ---
        //
        // vss/vcc are deliberately given no terminal entries: group traversal
        // stops at a power rail (it records the rail and does not cross it), so
        // their adjacency would be thousands of entries that are never read.
        let mut gate_counts = vec![0u32; node_count + 1];
        let mut term_counts = vec![0u32; node_count + 1];
        for t in 0..trans_count {
            gate_counts[trans_gate[t] as usize + 1] += 1;
            for n in [trans_c1[t], trans_c2[t]] {
                if n != vss && n != vcc {
                    term_counts[n as usize + 1] += 1;
                }
            }
        }
        for i in 0..node_count {
            gate_counts[i + 1] += gate_counts[i];
            term_counts[i + 1] += term_counts[i];
        }
        let gate_offsets = gate_counts;
        let term_offsets = term_counts;

        let mut gate_index = vec![0 as TransId; gate_offsets[node_count] as usize];
        let mut term_index =
            vec![Terminal { transistor: 0, other: 0 }; term_offsets[node_count] as usize];
        let mut gate_fill = gate_offsets.clone();
        let mut term_fill = term_offsets.clone();
        for t in 0..trans_count {
            let g = trans_gate[t] as usize;
            gate_index[gate_fill[g] as usize] = t as TransId;
            gate_fill[g] += 1;

            let (c1, c2) = (trans_c1[t], trans_c2[t]);
            // A transistor with c1 == c2 is pushed twice by the reference; keep
            // that, since it affects nothing but must not silently differ.
            for (near, far) in [(c1, c2), (c2, c1)] {
                if near != vss && near != vcc {
                    let ni = near as usize;
                    term_index[term_fill[ni] as usize] =
                        Terminal { transistor: t as TransId, other: far };
                    term_fill[ni] += 1;
                }
            }
        }

        Ok(Netlist {
            node_count,
            vss,
            vcc,
            exists,
            pullup,
            trans_gate: trans_gate.into_boxed_slice(),
            trans_c1: trans_c1.into_boxed_slice(),
            trans_c2: trans_c2.into_boxed_slice(),
            gate_offsets: gate_offsets.into_boxed_slice(),
            gate_index: gate_index.into_boxed_slice(),
            term_offsets: term_offsets.into_boxed_slice(),
            term_index: term_index.into_boxed_slice(),
            names,
            node_names: node_names.into_boxed_slice(),
            rail_conflict_holds: BitSet::new(node_count),
            node_areas: Box::new([]),
        })
    }

    #[inline]
    pub fn node_count(&self) -> usize {
        self.node_count
    }
    #[inline]
    pub fn transistor_count(&self) -> usize {
        self.trans_gate.len()
    }
    #[inline]
    pub fn vss(&self) -> NodeId {
        self.vss
    }
    #[inline]
    pub fn vcc(&self) -> NodeId {
        self.vcc
    }
    #[inline]
    pub fn is_rail(&self, n: NodeId) -> bool {
        n == self.vss || n == self.vcc
    }

    /// Supply per-node areas (the reference's units: twice the shoelace
    /// area summed per node). Call after decoding, before marking holds.
    pub fn set_node_areas(&mut self, areas: Box<[f64]>) {
        assert_eq!(areas.len(), self.node_count, "one area per node id");
        self.node_areas = areas;
    }

    #[inline]
    pub fn node_area(&self, n: NodeId) -> f64 {
        self.node_areas[n as usize]
    }

    /// Mark nodes for the rail-conflict hold (see the field's note). Call
    /// after decoding, before the netlist is shared, and after
    /// `set_node_areas`: the hold's charge vote is area-weighted, and a
    /// vote without weights would be a different rule wearing this one's
    /// name.
    pub fn set_rail_conflict_holds(&mut self, nodes: &[NodeId]) {
        assert!(
            self.node_areas.len() == self.node_count,
            "set_node_areas must come first: the hold votes by area"
        );
        for &n in nodes {
            assert!(!self.is_rail(n), "a rail cannot carry the hold");
            self.rail_conflict_holds.set(n as usize);
        }
    }

    /// Whether any of these group members carries the rail-conflict hold.
    #[inline]
    pub fn any_rail_conflict_hold(&self, members: &[NodeId]) -> bool {
        members.iter().any(|&m| self.rail_conflict_holds.get(m as usize))
    }

    /// Whether the die data defines this node at all. The node array is sparse:
    /// a few indices below the maximum are unused, and the reference renders
    /// them as `x` in its state string.
    #[inline]
    pub fn exists(&self, n: NodeId) -> bool {
        self.exists.get(n as usize)
    }

    /// Initial pullup flags, as extracted from the layout.
    pub fn pullups(&self) -> &BitSet {
        &self.pullup
    }

    #[inline]
    pub fn gates_of(&self, n: NodeId) -> &[TransId] {
        let (a, b) = (self.gate_offsets[n as usize], self.gate_offsets[n as usize + 1]);
        &self.gate_index[a as usize..b as usize]
    }

    #[inline]
    pub fn terminals_of(&self, n: NodeId) -> &[Terminal] {
        let (a, b) = (self.term_offsets[n as usize], self.term_offsets[n as usize + 1]);
        &self.term_index[a as usize..b as usize]
    }

    #[inline]
    pub fn transistor_gate(&self, t: TransId) -> NodeId {
        self.trans_gate[t as usize]
    }
    #[inline]
    pub fn transistor_c1(&self, t: TransId) -> NodeId {
        self.trans_c1[t as usize]
    }
    #[inline]
    pub fn transistor_c2(&self, t: TransId) -> NodeId {
        self.trans_c2[t as usize]
    }

    /// Resolve a signal name (`"clk0"`, `"ab3"`, `"#pclp0"`) to a node.
    pub fn node(&self, name: &str) -> Option<NodeId> {
        self.names.get(name).copied()
    }

    /// The canonical name for a node, if it has one.
    pub fn name_of(&self, n: NodeId) -> Option<&str> {
        self.node_names.get(n as usize)?.as_deref()
    }

    pub fn names(&self) -> impl Iterator<Item = (&str, NodeId)> {
        self.names.iter().map(|(k, &v)| (&**k, v))
    }

    /// Resolve a bus named `prefix0..prefixN-1`. Returns `None` if any bit is
    /// missing, so a typo fails loudly instead of silently reading a short bus.
    pub fn bus<const N: usize>(&self, prefix: &str) -> Option<[NodeId; N]> {
        let mut out = [0; N];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.node(&format!("{prefix}{i}"))?;
        }
        Some(out)
    }

    /// Width of the bus named `prefix`, discovered the way the reference does:
    /// by counting `prefix0`, `prefix1`, ... until one is missing.
    pub fn bus_width(&self, prefix: &str) -> usize {
        (0..).take_while(|i| self.names.contains_key(format!("{prefix}{i}").as_str())).count()
    }
}
