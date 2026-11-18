//! Cycle removal: Eades–Lin–Smyth greedy feedback arc set.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use super::LGraph;

struct State {
    outs: Vec<Vec<(u32, i64)>>,
    ins: Vec<Vec<(u32, i64)>>,
    out_w: Vec<i64>,
    in_w: Vec<i64>,
    removed: Vec<bool>,
    queued: Vec<bool>,
    sinks: VecDeque<u32>,
    sources: VecDeque<u32>,
    heap: BinaryHeap<(i64, Reverse<u32>)>,
    remaining: usize,
}

impl State {
    fn remove(&mut self, v: u32) {
        let vi = v as usize;
        self.removed[vi] = true;
        self.remaining -= 1;
        for k in 0..self.outs[vi].len() {
            let (w, wt) = self.outs[vi][k];
            let wi = w as usize;
            if self.removed[wi] {
                continue;
            }
            self.in_w[wi] -= wt;
            if self.in_w[wi] == 0 {
                if !self.queued[wi] {
                    self.queued[wi] = true;
                    self.sources.push_back(w);
                }
            } else {
                self.heap.push((self.out_w[wi] - self.in_w[wi], Reverse(w)));
            }
        }
        for k in 0..self.ins[vi].len() {
            let (u, wt) = self.ins[vi][k];
            let ui = u as usize;
            if self.removed[ui] {
                continue;
            }
            self.out_w[ui] -= wt;
            if self.out_w[ui] == 0 {
                if !self.queued[ui] {
                    self.queued[ui] = true;
                    self.sinks.push_back(u);
                }
            } else {
                self.heap.push((self.out_w[ui] - self.in_w[ui], Reverse(u)));
            }
        }
    }
}

/// Reverses a greedy feedback arc set so the graph becomes acyclic.
/// Deterministic: ties are broken by node index.
pub(crate) fn make_acyclic(lg: &mut LGraph) {
    let n = lg.nodes.len();
    let mut st = State {
        outs: vec![Vec::new(); n],
        ins: vec![Vec::new(); n],
        out_w: vec![0; n],
        in_w: vec![0; n],
        removed: vec![false; n],
        queued: vec![false; n],
        sinks: VecDeque::new(),
        sources: VecDeque::new(),
        heap: BinaryHeap::new(),
        remaining: n,
    };
    for e in &lg.edges {
        if e.from == e.to {
            continue;
        }
        let w = i64::from(e.weight.max(1));
        st.outs[e.from as usize].push((e.to, w));
        st.ins[e.to as usize].push((e.from, w));
        st.out_w[e.from as usize] += w;
        st.in_w[e.to as usize] += w;
    }
    for v in 0..n {
        if st.out_w[v] == 0 {
            st.queued[v] = true;
            st.sinks.push_back(v as u32);
        } else if st.in_w[v] == 0 {
            st.queued[v] = true;
            st.sources.push_back(v as u32);
        } else {
            st.heap.push((st.out_w[v] - st.in_w[v], Reverse(v as u32)));
        }
    }
    let mut s1: Vec<u32> = Vec::with_capacity(n);
    let mut s2: Vec<u32> = Vec::new();
    while st.remaining > 0 {
        let before = st.remaining;
        while let Some(v) = st.sinks.pop_front() {
            if !st.removed[v as usize] {
                st.remove(v);
                s2.push(v);
            }
        }
        while let Some(v) = st.sources.pop_front() {
            if !st.removed[v as usize] {
                st.remove(v);
                s1.push(v);
            }
        }
        if !st.sinks.is_empty() {
            continue;
        }
        while let Some((d, Reverse(v))) = st.heap.pop() {
            let vi = v as usize;
            if st.removed[vi] || d != st.out_w[vi] - st.in_w[vi] {
                continue;
            }
            st.remove(v);
            s1.push(v);
            break;
        }
        if st.remaining == before {
            // Defensive: guarantees progress.
            if let Some(v) = (0..n).find(|&v| !st.removed[v]) {
                st.remove(v as u32);
                s1.push(v as u32);
            }
        }
    }
    let mut pos = vec![0u32; n];
    for (i, &v) in s1.iter().chain(s2.iter().rev()).enumerate() {
        pos[v as usize] = i as u32;
    }
    for e in &mut lg.edges {
        if e.from != e.to && pos[e.from as usize] > pos[e.to as usize] {
            std::mem::swap(&mut e.from, &mut e.to);
            e.reversed = !e.reversed;
        }
    }
}
