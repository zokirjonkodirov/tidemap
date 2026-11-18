//! Width snapping and shelf packing of independent components.

/// Rounds `w` up to a multiple of `step` (unchanged for a non-positive step).
pub(crate) fn snap_width(w: f32, step: f32) -> f32 {
    if step <= 0.0 || !w.is_finite() {
        return w;
    }
    ((w / step) - 1e-4).ceil().max(1.0) * step
}

/// Shelf-packs rectangles of `sizes` (width, height); returns the top-left offset of each.
/// Components are sorted by height (tallest first); the shelf width is the candidate
/// that brings the overall aspect closest to `target_aspect`.
pub(crate) fn pack(sizes: &[[f32; 2]], gap: f32, target_aspect: f32) -> Vec<[f32; 2]> {
    let n = sizes.len();
    if n <= 1 {
        return vec![[0.0, 0.0]; n];
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| sizes[b][1].total_cmp(&sizes[a][1]).then(a.cmp(&b)));

    // Candidate shelf widths: prefix sums of the sorted widths.
    let mut prefix = Vec::with_capacity(n);
    let mut acc = 0.0f32;
    for (k, &i) in idx.iter().enumerate() {
        acc += sizes[i][0] + if k > 0 { gap } else { 0.0 };
        prefix.push(acc);
    }
    let mut cands: Vec<f32> = if n <= 64 {
        prefix.clone()
    } else {
        (1..=64).map(|c| prefix[(c * n).div_ceil(64) - 1]).collect()
    };
    cands.dedup();

    let simulate = |width: f32, out: &mut Vec<[f32; 2]>| -> (f32, f32) {
        let (mut x, mut y, mut row_h, mut tw) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for &i in &idx {
            let [w, h] = sizes[i];
            if x > 0.0 && x + w > width + 1e-3 {
                y += row_h + gap;
                x = 0.0;
                row_h = 0.0;
            }
            out[i] = [x, y];
            x += w + gap;
            tw = tw.max(x - gap);
            row_h = row_h.max(h);
        }
        (tw, y + row_h)
    };

    let mut best_pos = vec![[0.0f32, 0.0f32]; n];
    let mut best_score = (f32::INFINITY, f32::INFINITY);
    let mut cur = vec![[0.0f32, 0.0f32]; n];
    for &wd in &cands {
        let (tw, th) = simulate(wd, &mut cur);
        let aspect = tw.max(1.0) / th.max(1.0);
        let score = ((aspect / target_aspect.max(0.1)).ln().abs(), tw * th);
        if score.0 < best_score.0 - 1e-4
            || (score.0 <= best_score.0 + 1e-4 && score.1 < best_score.1)
        {
            best_score = score;
            best_pos.clone_from(&cur);
        }
    }
    best_pos
}
