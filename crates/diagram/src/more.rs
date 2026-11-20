//! More chart-style diagrams: sankey, xychart, packet, kanban, radar and treemap.

use crate::charts::{line, rect, text};
use crate::decor::{Anchor, Decor, Tone};
use crate::linewise::Line;
use crate::parser::clean_label;
use crate::shape::Rect;
use crate::text::Measurer;

const TAU: f32 = std::f32::consts::TAU;
const HALF_PI: f32 = std::f32::consts::FRAC_PI_2;

type Body<'a> = [(Line<'a>, &'a str)];

fn unquote(s: &str) -> String {
    clean_label(s.trim().trim_matches('"'))
}

/// Splits CSV-ish text at commas outside double quotes.
fn csv(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn indent_of(line: &Line) -> usize {
    line.text
        .chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

// ---- sankey ---------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Sankey {
    pub names: Vec<String>,
    pub links: Vec<(usize, usize, f64)>,
}

pub(crate) fn parse_sankey(body: &Body) -> Sankey {
    let mut s = Sankey::default();
    let id = |names: &mut Vec<String>, n: &str| -> usize {
        match names.iter().position(|x| x == n) {
            Some(i) => i,
            None => {
                names.push(n.to_string());
                names.len() - 1
            }
        }
    };
    for (_, text) in body {
        let parts = csv(text);
        if parts.len() < 3 {
            continue;
        }
        let Ok(v) = parts[2].parse::<f64>() else {
            continue;
        };
        if !(v.is_finite() && v > 0.0) {
            continue;
        }
        let a = id(&mut s.names, &unquote(&parts[0]));
        let b = id(&mut s.names, &unquote(&parts[1]));
        if a != b {
            s.links.push((a, b, v));
        }
    }
    s
}

pub(crate) fn layout_sankey(s: &Sankey, me: &Measurer) -> Vec<Decor> {
    let n = s.names.len();
    let mut d = Vec::new();
    if n == 0 {
        return d;
    }
    let mut inn = vec![0.0f64; n];
    let mut out = vec![0.0f64; n];
    for &(a, b, v) in &s.links {
        out[a] += v;
        inn[b] += v;
    }
    let value: Vec<f64> = (0..n).map(|i| inn[i].max(out[i])).collect();
    let mut depth = vec![0usize; n];
    for _ in 0..n {
        let mut changed = false;
        for &(a, b, _) in &s.links {
            if depth[b] < depth[a] + 1 && depth[a] < n {
                depth[b] = depth[a] + 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let cols = depth.iter().copied().max().unwrap_or(0) + 1;
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); cols];
    for i in 0..n {
        columns[depth[i]].push(i);
    }
    let gap = 16.0f32;
    let col_sum = |c: &Vec<usize>| c.iter().map(|&i| value[i]).sum::<f64>();
    let max_sum = columns.iter().map(col_sum).fold(0.0, f64::max).max(1e-9);
    let max_count = columns.iter().map(Vec::len).max().unwrap_or(1);
    let height = 360.0f32;
    let scale = ((height - gap * max_count as f32).max(120.0) as f64 / max_sum) as f32;
    let col_x = |c: usize| c as f32 * 240.0;
    let place = |columns: &Vec<Vec<usize>>| -> Vec<f32> {
        let mut y = vec![0.0f32; n];
        for c in columns {
            let total: f32 = c.iter().map(|&i| value[i] as f32 * scale).sum::<f32>()
                + gap * c.len().saturating_sub(1) as f32;
            let mut cur = (height - total).max(0.0) / 2.0;
            for &i in c {
                y[i] = cur;
                cur += value[i] as f32 * scale + gap;
            }
        }
        y
    };
    let mut ys = place(&columns);
    for sweep in 0..4 {
        let forward = sweep % 2 == 0;
        let order: Vec<usize> = if forward { (1..cols).collect() } else { (0..cols.saturating_sub(1)).rev().collect() };
        for c in order {
            let mut keyed: Vec<(f32, usize)> = columns[c]
                .iter()
                .map(|&i| {
                    let mut sum = 0.0f64;
                    let mut w = 0.0f64;
                    for &(a, b, v) in &s.links {
                        let other = if forward && b == i { Some(a) } else if !forward && a == i { Some(b) } else { None };
                        if let Some(o) = other {
                            sum += f64::from(ys[o] + value[o] as f32 * scale / 2.0) * v;
                            w += v;
                        }
                    }
                    (if w > 0.0 { (sum / w) as f32 } else { ys[i] }, i)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            columns[c] = keyed.into_iter().map(|k| k.1).collect();
        }
        ys = place(&columns);
    }
    let w = 14.0f32;
    let mut out_off = vec![0.0f32; n];
    let mut in_off = vec![0.0f32; n];
    let mut links: Vec<usize> = (0..s.links.len()).collect();
    links.sort_by(|&a, &b| ys[s.links[a].1].total_cmp(&ys[s.links[b].1]));
    let mut src_y = vec![0.0f32; s.links.len()];
    for &l in &links {
        let (a, _, v) = s.links[l];
        src_y[l] = ys[a] + out_off[a];
        out_off[a] += v as f32 * scale;
    }
    let mut by_src: Vec<usize> = (0..s.links.len()).collect();
    by_src.sort_by(|&a, &b| src_y[a].total_cmp(&src_y[b]));
    let mut dst_y = vec![0.0f32; s.links.len()];
    for &l in &by_src {
        let (_, b, v) = s.links[l];
        dst_y[l] = ys[b] + in_off[b];
        in_off[b] += v as f32 * scale;
    }
    for (l, &(a, b, v)) in s.links.iter().enumerate() {
        let t = v as f32 * scale;
        let (x0, x1) = (col_x(depth[a]) + w, col_x(depth[b]));
        let mid = (x0 + x1) / 2.0;
        let bez = |ya: f32, yb: f32, rev: bool| -> Vec<[f32; 2]> {
            let mut pts = Vec::new();
            for k in 0..=20 {
                let u = k as f32 / 20.0;
                let v = 1.0 - u;
                let x = v * v * v * x0 + 3.0 * v * v * u * mid + 3.0 * v * u * u * mid + u * u * u * x1;
                let y = v * v * v * ya + 3.0 * v * v * u * ya + 3.0 * v * u * u * yb + u * u * u * yb;
                pts.push([x, y]);
            }
            if rev {
                pts.reverse();
            }
            pts
        };
        let mut pts = bez(src_y[l], dst_y[l], false);
        pts.extend(bez(src_y[l] + t, dst_y[l] + t, true));
        d.push(Decor::Poly {
            pts,
            fill: Tone::Swatch(a as u8),
            stroke: None,
            alpha: 140,
        });
    }
    for i in 0..n {
        let h = (value[i] as f32 * scale).max(2.0);
        let tone = Tone::Swatch(i as u8);
        d.push(rect(col_x(depth[i]), ys[i], w, h, 2.0, tone, Some(Tone::Ink)));
        let last = depth[i] == cols - 1 && cols > 1;
        let (x, anchor) = if last {
            (col_x(depth[i]) - 8.0, Anchor::End)
        } else {
            (col_x(depth[i]) + w + 8.0, Anchor::Start)
        };
        let label = format!("{} {}", s.names[i], fmt(value[i]));
        let _ = me;
        d.push(text([x, ys[i] + h / 2.0], &label, 12.5, anchor, false, Tone::Ink));
    }
    d
}

fn fmt(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

// ---- xychart --------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Xy {
    pub title: Option<String>,
    pub x_title: String,
    pub categories: Vec<String>,
    pub x_range: Option<(f64, f64)>,
    pub y_title: String,
    pub y_range: Option<(f64, f64)>,
    /// `(is_bar, values)`
    pub series: Vec<(bool, Vec<f64>)>,
}

fn list(text: &str) -> Vec<String> {
    let open = text.find('[');
    let close = text.rfind(']');
    match (open, close) {
        (Some(o), Some(c)) if o < c => csv(&text[o + 1..c]).into_iter().map(|s| unquote(&s)).collect(),
        _ => Vec::new(),
    }
}

fn range(text: &str) -> Option<(f64, f64)> {
    let (a, b) = text.split_once("-->")?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

pub(crate) fn parse_xy(header_rest: &str, body: &Body) -> Xy {
    let mut x = Xy::default();
    let _ = header_rest;
    for (_, text) in body {
        let (word, rest) = crate::linewise::split_word(text);
        match word {
            "title" => x.title = Some(unquote(rest)),
            "x-axis" => {
                if rest.contains('[') {
                    x.categories = list(rest);
                    if let Some(q) = rest.strip_prefix('"') {
                        x.x_title = q.split('"').next().unwrap_or("").to_string();
                    }
                } else if let Some(r) = range(rest.rsplit('"').next().unwrap_or(rest)) {
                    x.x_range = Some(r);
                    if let Some(q) = rest.strip_prefix('"') {
                        x.x_title = q.split('"').next().unwrap_or("").to_string();
                    }
                } else {
                    x.x_title = unquote(rest);
                }
            }
            "y-axis" => {
                x.y_range = range(rest.rsplit('"').next().unwrap_or(rest));
                if let Some(q) = rest.strip_prefix('"') {
                    x.y_title = q.split('"').next().unwrap_or("").to_string();
                }
            }
            "bar" | "line" => {
                let vals: Vec<f64> = list(rest).iter().filter_map(|v| v.parse().ok()).collect();
                if !vals.is_empty() {
                    x.series.push((word == "bar", vals));
                }
            }
            _ => {}
        }
    }
    x
}

fn nice_step(range: f64, target: f64) -> f64 {
    let raw = (range / target).max(1e-12);
    let mag = 10f64.powf(raw.log10().floor());
    let f = raw / mag;
    let nice = if f <= 1.0 { 1.0 } else if f <= 2.0 { 2.0 } else if f <= 5.0 { 5.0 } else { 10.0 };
    nice * mag
}

pub(crate) fn layout_xy(x: &Xy, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    if x.series.is_empty() {
        return d;
    }
    let n = x
        .series
        .iter()
        .map(|s| s.1.len())
        .max()
        .unwrap_or(0)
        .max(x.categories.len());
    let (mut lo, mut hi) = x.y_range.unwrap_or_else(|| {
        let max = x.series.iter().flat_map(|s| s.1.iter().copied()).fold(f64::MIN, f64::max);
        let min = x.series.iter().flat_map(|s| s.1.iter().copied()).fold(f64::MAX, f64::min);
        (min.min(0.0), max.max(0.0))
    });
    if hi <= lo {
        hi = lo + 1.0;
    }
    let step = nice_step(hi - lo, 5.0);
    if x.y_range.is_none() {
        lo = (lo / step).floor() * step;
        hi = (hi / step).ceil() * step;
    }
    let (pw, ph) = (580.0f32, 320.0f32);
    let left = 56.0f32;
    let top = if x.title.is_some() { 44.0 } else { 12.0 };
    let y_of = |v: f64| top + ph - ((v - lo) / (hi - lo)) as f32 * ph;
    if let Some(t) = &x.title {
        d.push(text([left + pw / 2.0, 14.0], t, 18.0, Anchor::Middle, true, Tone::Ink));
    }
    let mut t = (lo / step).ceil() * step;
    while t <= hi + step * 1e-6 {
        let y = y_of(t);
        d.push(line(vec![[left, y], [left + pw, y]], 1.0, true, Tone::Grid));
        d.push(text([left - 8.0, y], &fmt(t), 12.0, Anchor::End, false, Tone::Muted));
        t += step;
    }
    d.push(line(vec![[left, top], [left, top + ph]], 1.0, false, Tone::Muted));
    d.push(line(vec![[left, top + ph], [left + pw, top + ph]], 1.0, false, Tone::Muted));
    let slot = pw / n.max(1) as f32;
    let bars = x.series.iter().filter(|s| s.0).count().max(1);
    let mut bar_i = 0;
    let mut line_i = 0;
    for (si, (is_bar, vals)) in x.series.iter().enumerate() {
        if *is_bar {
            let bw = slot * 0.7 / bars as f32;
            for (i, v) in vals.iter().enumerate() {
                let cx = left + (i as f32 + 0.5) * slot;
                let x0 = cx - slot * 0.35 + bar_i as f32 * bw;
                let (y0, y1) = (y_of(*v), y_of(0.0f64.clamp(lo, hi)));
                let tone = Tone::Swatch(si as u8);
                d.push(rect(x0, y0.min(y1), bw, (y1 - y0).abs().max(1.0), 2.0, tone, Some(tone)));
            }
            bar_i += 1;
        } else {
            let pts: Vec<[f32; 2]> = vals
                .iter()
                .enumerate()
                .map(|(i, v)| [left + (i as f32 + 0.5) * slot, y_of(*v)])
                .collect();
            let tone = Tone::Swatch((si as u8) + 3 + line_i);
            line_i += 1;
            d.push(line(pts.clone(), 2.5, false, tone));
            for p in pts {
                d.push(Decor::Circle { c: p, r: 3.5, fill: tone, stroke: Some(tone) });
            }
        }
    }
    for i in 0..n {
        let label = if let Some(c) = x.categories.get(i) {
            c.clone()
        } else if let Some((a, b)) = x.x_range {
            fmt(a + (b - a) * i as f64 / (n.max(2) - 1) as f64)
        } else {
            (i + 1).to_string()
        };
        let _ = me;
        d.push(text([left + (i as f32 + 0.5) * slot, top + ph + 16.0], &label, 12.0, Anchor::Middle, false, Tone::Muted));
    }
    if !x.x_title.is_empty() {
        d.push(text([left + pw / 2.0, top + ph + 38.0], &x.x_title, 12.5, Anchor::Middle, true, Tone::Ink));
    }
    if !x.y_title.is_empty() {
        d.push(text([left - 40.0, top - 14.0], &x.y_title, 12.5, Anchor::Start, true, Tone::Ink));
    }
    d
}

// ---- packet ---------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Packet {
    pub title: Option<String>,
    pub fields: Vec<(u32, u32, String)>,
}

pub(crate) fn parse_packet(body: &Body) -> Packet {
    let mut p = Packet::default();
    let mut next = 0u32;
    for (_, text) in body {
        if let Some(t) = text.strip_prefix("title ") {
            p.title = Some(unquote(t));
            continue;
        }
        let Some((range, label)) = text.split_once(':') else {
            continue;
        };
        let range = range.trim();
        let (a, b) = if let Some(len) = range.strip_prefix('+') {
            let Ok(n) = len.trim().parse::<u32>() else { continue };
            (next, next + n.max(1) - 1)
        } else if let Some((a, b)) = range.split_once('-') {
            let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) else { continue };
            (a, b.max(a))
        } else if let Ok(a) = range.parse::<u32>() {
            (a, a)
        } else {
            continue;
        };
        if b > 65_535 {
            continue;
        }
        next = b + 1;
        p.fields.push((a, b, unquote(label)));
    }
    p
}

pub(crate) fn layout_packet(p: &Packet, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    let bit_w = 19.0f32;
    let row_h = 46.0f32;
    let top = if p.title.is_some() { 48.0 } else { 20.0 };
    if let Some(t) = &p.title {
        d.push(text([16.0 * bit_w, 8.0], t, 18.0, Anchor::Middle, true, Tone::Ink));
    }
    for (fi, (a, b, label)) in p.fields.iter().enumerate() {
        let tone = Tone::Swatch(fi as u8 % 4);
        let mut bit = *a;
        while bit <= *b {
            let row = bit / 32;
            let end = (*b).min(row * 32 + 31);
            let x = (bit % 32) as f32 * bit_w;
            let w = (end - bit + 1) as f32 * bit_w;
            let y = top + row as f32 * row_h;
            d.push(rect(x, y, w, row_h - 10.0, 0.0, tone, Some(Tone::Muted)));
            let mut shown = label.clone();
            while me.width(&shown, 12.5) > w - 6.0 && shown.chars().count() > 2 {
                shown.pop();
                shown.pop();
                shown.push('\u{2026}');
            }
            d.push(text([x + w / 2.0, y + (row_h - 10.0) / 2.0], &shown, 12.5, Anchor::Middle, false, Tone::Ink));
            d.push(text([x + 2.0, y - 7.0], &bit.to_string(), 10.0, Anchor::Start, false, Tone::Muted));
            if end > bit {
                d.push(text([x + w - 2.0, y - 7.0], &end.to_string(), 10.0, Anchor::End, false, Tone::Muted));
            }
            bit = end + 1;
        }
    }
    d
}

// ---- kanban ---------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct Kanban {
    pub columns: Vec<(String, Vec<String>)>,
}

fn kanban_label(text: &str) -> String {
    let text = text.split("@{").next().unwrap_or(text).trim();
    match (text.find('['), text.rfind(']')) {
        (Some(o), Some(c)) if o < c => unquote(&text[o + 1..c]),
        _ => unquote(text),
    }
}

pub(crate) fn parse_kanban(body: &Body) -> Kanban {
    let mut k = Kanban::default();
    let base = body.iter().map(|(l, _)| indent_of(l)).min().unwrap_or(0);
    for (line, text) in body {
        let label = kanban_label(text);
        if indent_of(line) <= base || k.columns.is_empty() {
            k.columns.push((label, Vec::new()));
        } else if let Some(col) = k.columns.last_mut() {
            col.1.push(label);
        }
    }
    k
}

pub(crate) fn layout_kanban(k: &Kanban, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    let col_w = 230.0f32;
    let gap = 16.0f32;
    let head = 36.0f32;
    let mut heights = Vec::new();
    let mut cards: Vec<Vec<Decor>> = Vec::new();
    for (ci, (_, items)) in k.columns.iter().enumerate() {
        let x = ci as f32 * (col_w + gap);
        let mut y = head + 12.0;
        let mut list = Vec::new();
        for it in items {
            let tb = me.wrap(it, 13.0, 17.0, col_w - 36.0);
            let h = tb.height + 18.0;
            list.push(rect(x + 8.0, y, col_w - 16.0, h, 6.0, Tone::Canvas, Some(Tone::Muted)));
            for (li, l) in tb.lines.iter().enumerate() {
                list.push(text([x + 18.0, y + 9.0 + 8.5 + li as f32 * 17.0], l, 13.0, Anchor::Start, false, Tone::Ink));
            }
            y += h + 10.0;
        }
        heights.push(y + 2.0);
        cards.push(list);
    }
    let tall = heights.iter().copied().fold(head + 20.0, f32::max);
    for (ci, (title, _)) in k.columns.iter().enumerate() {
        let x = ci as f32 * (col_w + gap);
        d.push(rect(x, 0.0, col_w, tall, 8.0, Tone::Grid, None));
        let tone = Tone::Swatch(ci as u8);
        d.push(rect(x, 0.0, col_w, head, 8.0, tone, Some(tone)));
        d.push(text([x + 14.0, head / 2.0], title, 13.5, Anchor::Start, true, Tone::Ink));
    }
    for list in cards {
        d.extend(list);
    }
    d
}

// ---- radar ----------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Radar {
    pub title: Option<String>,
    pub axes: Vec<String>,
    pub curves: Vec<(String, Vec<f64>)>,
    pub min: f64,
    pub max: Option<f64>,
    pub ticks: usize,
    pub circle: bool,
}

pub(crate) fn parse_radar(body: &Body) -> Radar {
    let mut r = Radar {
        title: None,
        axes: Vec::new(),
        curves: Vec::new(),
        min: 0.0,
        max: None,
        ticks: 5,
        circle: true,
    };
    let label_of = |item: &str| -> String {
        match (item.find('['), item.rfind(']')) {
            (Some(o), Some(c)) if o < c => unquote(&item[o + 1..c]),
            _ => item.trim().to_string(),
        }
    };
    for (_, text) in body {
        let (word, rest) = crate::linewise::split_word(text);
        match word {
            "title" => r.title = Some(unquote(rest)),
            "axis" => {
                for item in csv(rest) {
                    r.axes.push(label_of(&item));
                }
            }
            "curve" => {
                let mut rest = rest;
                while let Some(open) = rest.find('{') {
                    let Some(close) = rest[open..].find('}') else { break };
                    let head = rest[..open].trim().trim_start_matches(',').trim();
                    let vals: Vec<f64> = rest[open + 1..open + close]
                        .split(',')
                        .filter_map(|v| v.rsplit(':').next().and_then(|v| v.trim().parse().ok()))
                        .collect();
                    r.curves.push((label_of(head), vals));
                    rest = &rest[open + close + 1..];
                }
            }
            "max" => r.max = rest.trim().parse().ok(),
            "min" => r.min = rest.trim().parse().unwrap_or(0.0),
            "ticks" => r.ticks = rest.trim().parse::<usize>().unwrap_or(5).clamp(1, 20),
            "graticule" => r.circle = !rest.trim().starts_with("polygon"),
            _ => {}
        }
    }
    r
}

pub(crate) fn layout_radar(r: &Radar, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    let n = r.axes.len();
    if n < 3 {
        return d;
    }
    let max = r
        .max
        .unwrap_or_else(|| r.curves.iter().flat_map(|c| c.1.iter().copied()).fold(r.min + 1.0, f64::max));
    let rad = 170.0f32;
    let c = [rad + 90.0, rad + 70.0];
    if let Some(t) = &r.title {
        d.push(text([c[0], 12.0], t, 18.0, Anchor::Middle, true, Tone::Ink));
    }
    let ang = |i: usize| -HALF_PI + i as f32 * TAU / n as f32;
    let at = |i: usize, f: f32| [c[0] + rad * f * ang(i).cos(), c[1] + rad * f * ang(i).sin()];
    for k in 1..=r.ticks {
        let f = k as f32 / r.ticks as f32;
        let ring: Vec<[f32; 2]> = if r.circle {
            (0..=72).map(|s| {
                let a = s as f32 * TAU / 72.0;
                [c[0] + rad * f * a.cos(), c[1] + rad * f * a.sin()]
            }).collect()
        } else {
            (0..=n).map(|i| at(i % n, f)).collect()
        };
        d.push(line(ring, 1.0, true, Tone::Muted));
    }
    for i in 0..n {
        d.push(line(vec![c, at(i, 1.0)], 1.0, false, Tone::Muted));
        let p = at(i, 1.12);
        let cos = ang(i).cos();
        let anchor = if cos > 0.3 { Anchor::Start } else if cos < -0.3 { Anchor::End } else { Anchor::Middle };
        d.push(text(p, &r.axes[i], 13.0, anchor, false, Tone::Ink));
    }
    let _ = me;
    for (ci, (_, vals)) in r.curves.iter().enumerate() {
        let tone = Tone::Swatch(ci as u8 + 3);
        let pts: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let v = vals.get(i).copied().unwrap_or(r.min);
                at(i, (((v - r.min) / (max - r.min).max(1e-9)) as f32).clamp(0.0, 1.0))
            })
            .collect();
        d.push(Decor::Poly { pts: pts.clone(), fill: tone, stroke: Some(tone), alpha: 110 });
        let mut closed = pts;
        closed.push(closed[0]);
        d.push(line(closed, 2.0, false, tone));
    }
    let lx = c[0] + rad + 90.0;
    for (ci, (name, _)) in r.curves.iter().enumerate() {
        let y = 40.0 + ci as f32 * 24.0;
        let tone = Tone::Swatch(ci as u8 + 3);
        d.push(rect(lx, y - 7.0, 14.0, 14.0, 3.0, tone, Some(tone)));
        d.push(text([lx + 22.0, y], name, 13.0, Anchor::Start, false, Tone::Ink));
    }
    d
}

// ---- treemap --------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct TNode {
    pub name: String,
    pub value: Option<f64>,
    pub children: Vec<TNode>,
}

impl TNode {
    fn total(&self) -> f64 {
        if self.children.is_empty() {
            self.value.unwrap_or(0.0).max(0.0)
        } else {
            self.children.iter().map(TNode::total).sum()
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Treemap {
    pub roots: Vec<TNode>,
}

pub(crate) fn parse_treemap(body: &Body) -> Treemap {
    let mut t = Treemap::default();
    let mut stack: Vec<(usize, Vec<usize>)> = Vec::new(); // (indent, path)
    for (line, text) in body {
        if text.starts_with("classDef") || text.starts_with("title") {
            continue;
        }
        let text = text.split(":::").next().unwrap_or(text).trim();
        let (name, value) = match text.rsplit_once(':') {
            Some((n, v)) if v.trim().parse::<f64>().is_ok() => (n, v.trim().parse::<f64>().ok()),
            _ => (text, None),
        };
        let node = TNode {
            name: unquote(name),
            value,
            children: Vec::new(),
        };
        let indent = indent_of(line);
        while stack.last().is_some_and(|(i, _)| *i >= indent) {
            stack.pop();
        }
        let path = match stack.last() {
            Some((_, parent)) => {
                let mut cur = &mut t.roots[parent[0]];
                for &i in &parent[1..] {
                    cur = &mut cur.children[i];
                }
                cur.children.push(node);
                let mut p = parent.clone();
                p.push(cur.children.len() - 1);
                p
            }
            None => {
                t.roots.push(node);
                vec![t.roots.len() - 1]
            }
        };
        stack.push((indent, path));
    }
    t
}

/// Squarified treemap: `values` sorted descending, positive.
fn squarify(values: &[f64], bounds: Rect) -> Vec<Rect> {
    let total: f64 = values.iter().sum();
    let mut out = Vec::with_capacity(values.len());
    if total <= 0.0 || bounds.w <= 0.0 || bounds.h <= 0.0 {
        return vec![Rect::default(); values.len()];
    }
    let area = f64::from(bounds.w) * f64::from(bounds.h);
    let scaled: Vec<f64> = values.iter().map(|v| v / total * area).collect();
    let mut rect = bounds;
    let mut i = 0;
    while i < scaled.len() {
        let side = f64::from(rect.w.min(rect.h)).max(1e-6);
        let worst = |row: &[f64]| -> f64 {
            let s: f64 = row.iter().sum();
            let (mx, mn) = (row.iter().copied().fold(f64::MIN, f64::max), row.iter().copied().fold(f64::MAX, f64::min));
            (side * side * mx / (s * s)).max(s * s / (side * side * mn))
        };
        let mut j = i + 1;
        while j < scaled.len() && worst(&scaled[i..=j]) <= worst(&scaled[i..j]) {
            j += 1;
        }
        let row = &scaled[i..j];
        let s: f64 = row.iter().sum();
        let thick = (s / side) as f32;
        let horizontal = rect.w >= rect.h;
        let mut off = 0.0f32;
        for a in row {
            let len = (a / s) as f32 * side as f32;
            out.push(if horizontal {
                Rect { x: rect.x, y: rect.y + off, w: thick, h: len }
            } else {
                Rect { x: rect.x + off, y: rect.y, w: len, h: thick }
            });
            off += len;
        }
        if horizontal {
            rect.x += thick;
            rect.w -= thick;
        } else {
            rect.y += thick;
            rect.h -= thick;
        }
        i = j;
    }
    out
}

fn treemap_into(nodes: &[TNode], bounds: Rect, top: Option<usize>, me: &Measurer, d: &mut Vec<Decor>) {
    let mut order: Vec<usize> = (0..nodes.len()).filter(|&i| nodes[i].total() > 0.0).collect();
    order.sort_by(|&a, &b| nodes[b].total().total_cmp(&nodes[a].total()));
    let values: Vec<f64> = order.iter().map(|&i| nodes[i].total()).collect();
    let rects = squarify(&values, bounds);
    for (k, &i) in order.iter().enumerate() {
        let n = &nodes[i];
        let r = rects[k];
        let tone = Tone::Swatch(top.unwrap_or(i) as u8);
        if n.children.is_empty() {
            d.push(rect(r.x, r.y, r.w, r.h, 3.0, tone, Some(Tone::Canvas)));
            let label = format!("{} {}", n.name, fmt(n.total()));
            if r.w > me.width(&n.name, 12.0) + 10.0 && r.h > 22.0 {
                let shown = if r.w > me.width(&label, 12.0) + 10.0 { label } else { n.name.clone() };
                d.push(text([r.x + r.w / 2.0, r.y + r.h / 2.0], &shown, 12.0, Anchor::Middle, false, Tone::Ink));
            }
        } else {
            d.push(rect(r.x, r.y, r.w, r.h, 4.0, Tone::Grid, Some(tone)));
            let head = if r.h > 46.0 && r.w > 40.0 { 22.0 } else { 0.0 };
            if head > 0.0 {
                d.push(text([r.x + 8.0, r.y + head / 2.0 + 1.0], &n.name, 12.5, Anchor::Start, true, Tone::Ink));
            }
            let inner = Rect { x: r.x + 4.0, y: r.y + head + 2.0, w: (r.w - 8.0).max(0.0), h: (r.h - head - 6.0).max(0.0) };
            treemap_into(&n.children, inner, Some(top.unwrap_or(i)), me, d);
        }
    }
}

pub(crate) fn layout_treemap(t: &Treemap, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    treemap_into(&t.roots, Rect { x: 0.0, y: 0.0, w: 760.0, h: 460.0 }, None, me, &mut d);
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linewise::Line;

    fn body(src: &str) -> Vec<(Line<'_>, &str)> {
        let mut off = 0;
        let mut out = Vec::new();
        for raw in src.split_inclusive('\n') {
            let text = raw.trim_end_matches(['\n', '\r']);
            if !text.trim().is_empty() {
                out.push((Line { text, offset: off }, text.trim()));
            }
            off += raw.len();
        }
        out
    }

    #[test]
    fn sankey_xy_packet_parse() {
        let b = body("Coal,Power,20\nGas,Power,10.5\nPower,\"Homes, Big\",30\n");
        let s = parse_sankey(&b);
        assert_eq!(s.names.len(), 5 - 1 + 0);
        assert_eq!(s.links.len(), 3);
        let b = body("title Sales\nx-axis [jan, feb, mar]\ny-axis \"Revenue\" 0 --> 100\nbar [10, 20, 30]\nline [5, 15, 25]\n");
        let x = parse_xy("", &b);
        assert_eq!(x.categories, vec!["jan", "feb", "mar"]);
        assert_eq!(x.y_range, Some((0.0, 100.0)));
        assert_eq!(x.series.len(), 2);
        let b = body("0-15: \"Source Port\"\n16-31: \"Dest\"\n+8: \"Flags\"\n");
        let p = parse_packet(&b);
        assert_eq!(p.fields[2], (32, 39, "Flags".to_string()));
    }

    #[test]
    fn kanban_radar_treemap_parse() {
        let b = body("todo[Todo]\n  t1[Write docs]\n  t2[Ship]\ndone[Done]\n  t3[Plan]\n");
        let k = parse_kanban(&b);
        assert_eq!(k.columns.len(), 2);
        assert_eq!(k.columns[0].1, vec!["Write docs", "Ship"]);
        let b = body("title G\naxis m[\"Math\"], s[\"Sci\"], e[\"Eng\"]\ncurve a[\"Alice\"]{85, 90, 80}, b[\"Bob\"]{70, 75, 85}\nmax 100\n");
        let r = parse_radar(&b);
        assert_eq!(r.axes, vec!["Math", "Sci", "Eng"]);
        assert_eq!(r.curves.len(), 2);
        assert_eq!(r.curves[1].1, vec![70.0, 75.0, 85.0]);
        let b = body("\"A\"\n  \"A1\": 10\n  \"A2\": 30\n\"B\": 20\n");
        let t = parse_treemap(&b);
        assert_eq!(t.roots.len(), 2);
        assert_eq!(t.roots[0].total(), 40.0);
    }

    #[test]
    fn squarify_fills_the_area() {
        let r = squarify(&[6.0, 3.0, 2.0, 1.0], Rect { x: 0.0, y: 0.0, w: 120.0, h: 80.0 });
        let area: f32 = r.iter().map(|r| r.w * r.h).sum();
        assert!((area - 9600.0).abs() < 1.0, "{area}");
    }
}
