//! Chart-style diagrams (pie, gantt, timeline, journey, quadrant): parsers and layouts that
//! produce free-form [`Decor`] primitives instead of nodes and edges.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::decor::{Anchor, Decor, Tone};
use crate::graph::{Diagnostic, Family, Graph, Parsed};
use crate::layout::{Layout, LayoutError, check_cancel};
use crate::lexer::skip_preamble;
use crate::linewise::{Line, diagnostic, lines, split_word, strip_comment};
use crate::metrics::Metrics;
use crate::parser::clean_label;
use crate::shape::Rect;
use crate::text::Measurer;

const MAX_LINES: usize = 20_000;
const MARGIN: f32 = 28.0;

/// Parsed chart data.
#[derive(Clone, Debug)]
pub enum Chart {
    Pie {
        title: Option<String>,
        show_data: bool,
        slices: Vec<(String, f64)>,
    },
    Gantt(Gantt),
    Timeline(Timeline),
    Journey(Journey),
    Quadrant(Quadrant),
}

#[derive(Clone, Debug, Default)]
pub struct Gantt {
    pub title: Option<String>,
    pub sections: Vec<String>,
    pub tasks: Vec<Task>,
}

#[derive(Clone, Debug)]
pub struct Task {
    pub name: String,
    pub section: usize,
    /// Days since 1970-01-01.
    pub start: f64,
    pub end: f64,
    /// 0 normal, 1 active, 2 done, 3 critical.
    pub state: u8,
    pub milestone: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Timeline {
    pub title: Option<String>,
    pub sections: Vec<String>,
    pub periods: Vec<Period>,
}

#[derive(Clone, Debug)]
pub struct Period {
    pub section: Option<usize>,
    pub label: String,
    pub events: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Journey {
    pub title: Option<String>,
    pub sections: Vec<String>,
    pub tasks: Vec<Step>,
    pub actors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub section: Option<usize>,
    pub name: String,
    pub score: u8,
    pub actors: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Quadrant {
    pub title: Option<String>,
    pub x: (String, String),
    pub y: (String, String),
    pub quadrants: [String; 4],
    pub points: Vec<(String, f32, f32)>,
}

fn finish(family: Family, chart: Chart, diagnostics: Vec<Diagnostic>) -> Parsed {
    let graph = Graph {
        chart: Some(Arc::new(chart)),
        ..Graph::default()
    };
    Parsed {
        family,
        graph,
        diagnostics,
    }
}

/// Parses one of the chart families.
pub(crate) fn parse(source: &str, family: Family) -> Parsed {
    let start = skip_preamble(source);
    let all = lines(source, start);
    let mut body: Vec<(Line, &str)> = Vec::new();
    let mut header_rest = "";
    let mut first = true;
    for line in all.iter().take(MAX_LINES) {
        let text = strip_comment(line.text).trim();
        if text.is_empty() {
            continue;
        }
        if first {
            first = false;
            if is_header(text) {
                header_rest = split_word(text).1;
                continue;
            }
        }
        body.push((*line, text));
    }
    let mut diags = Vec::new();
    let chart = match family {
        Family::Pie => parse_pie(header_rest, &body),
        Family::Gantt => Chart::Gantt(parse_gantt(source, &body, &mut diags)),
        Family::Timeline => Chart::Timeline(parse_timeline(&body)),
        Family::Journey => Chart::Journey(parse_journey(&body)),
        _ => Chart::Quadrant(parse_quadrant(&body)),
    };
    finish(family, chart, diags)
}

fn is_header(text: &str) -> bool {
    let w = split_word(text).0;
    matches!(w, "pie" | "gantt" | "timeline" | "journey" | "quadrantChart")
}

// ---- pie --------------------------------------------------------------------

fn parse_pie(header_rest: &str, body: &[(Line, &str)]) -> Chart {
    let mut title = None;
    let mut show_data = false;
    let mut rest = header_rest;
    if let Some(r) = rest.strip_prefix("showData") {
        show_data = true;
        rest = r.trim_start();
    }
    if let Some(t) = rest.strip_prefix("title") {
        title = Some(clean_label(t.trim()));
    }
    let mut slices = Vec::new();
    for (_, text) in body {
        if let Some(t) = text.strip_prefix("title ") {
            title = Some(clean_label(t.trim()));
        } else if *text == "showData" {
            show_data = true;
        } else if let Some((label, value)) = text.rsplit_once(':')
            && let Ok(v) = value.trim().parse::<f64>()
            && v.is_finite()
            && v > 0.0
        {
            slices.push((clean_label(label.trim().trim_matches('"')), v));
        }
    }
    Chart::Pie {
        title,
        show_data,
        slices,
    }
}

// ---- gantt ------------------------------------------------------------------

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parses `text` with a `dateFormat` made of `YYYY`, `YY`, `MM`, `DD`, `HH`, `mm`, `ss`.
fn parse_date(text: &str, fmt: &str) -> Option<f64> {
    let t = text.as_bytes();
    let f = fmt.as_bytes();
    let (mut i, mut j) = (0, 0);
    let (mut y, mut mo, mut d, mut h, mut mi, mut s) = (1970i64, 1i64, 1i64, 0i64, 0i64, 0i64);
    let num = |t: &[u8], at: usize, n: usize| -> Option<i64> {
        let s = t.get(at..at + n)?;
        if !s.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(s).ok()?.parse().ok()
    };
    while j < f.len() {
        let rest = &f[j..];
        if rest.starts_with(b"YYYY") {
            y = num(t, i, 4)?;
            i += 4;
            j += 4;
        } else if rest.starts_with(b"YY") {
            y = 2000 + num(t, i, 2)?;
            i += 2;
            j += 2;
        } else if rest.starts_with(b"MM") {
            mo = num(t, i, 2)?;
            i += 2;
            j += 2;
        } else if rest.starts_with(b"DD") {
            d = num(t, i, 2)?;
            i += 2;
            j += 2;
        } else if rest.starts_with(b"HH") {
            h = num(t, i, 2)?;
            i += 2;
            j += 2;
        } else if rest.starts_with(b"mm") {
            mi = num(t, i, 2)?;
            i += 2;
            j += 2;
        } else if rest.starts_with(b"ss") {
            s = num(t, i, 2)?;
            i += 2;
            j += 2;
        } else {
            if t.get(i) != Some(&f[j]) {
                return None;
            }
            i += 1;
            j += 1;
        }
    }
    if i != t.len() || !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, mo, d) as f64 + (h * 3600 + mi * 60 + s) as f64 / 86_400.0)
}

fn parse_duration(text: &str) -> Option<f64> {
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (n, unit) = text.split_at(split);
    let n: f64 = n.parse().ok()?;
    let per_day = match unit {
        "ms" => 86_400_000.0,
        "s" => 86_400.0,
        "m" => 1440.0,
        "h" => 24.0,
        "d" => 1.0,
        "w" => 1.0 / 7.0,
        "M" => 1.0 / 30.0,
        "y" => 1.0 / 365.0,
        _ => return None,
    };
    Some(n / per_day)
}

fn parse_gantt(source: &str, body: &[(Line, &str)], diags: &mut Vec<Diagnostic>) -> Gantt {
    let mut g = Gantt::default();
    let mut fmt = "YYYY-MM-DD".to_string();
    let mut section = 0usize;
    let mut ids: Vec<(String, f64)> = Vec::new();
    let mut last_end: Option<f64> = None;
    for (line, text) in body {
        let (word, rest) = split_word(text);
        match word {
            "title" => {
                g.title = Some(clean_label(rest));
                continue;
            }
            "dateFormat" => {
                fmt = rest.trim().to_string();
                continue;
            }
            "axisFormat" | "excludes" | "includes" | "todayMarker" | "tickInterval" | "weekday"
            | "weekend" | "accTitle" | "accDescr" | "click" | "topAxis" | "inclusiveEndDates" => {
                continue;
            }
            "section" => {
                g.sections.push(clean_label(rest));
                section = g.sections.len() - 1;
                continue;
            }
            _ => {}
        }
        let Some((name, meta)) = text.split_once(':') else {
            diags.push(diagnostic(source, line, line.span_of(text), "Unrecognised gantt statement"));
            continue;
        };
        if g.sections.is_empty() {
            g.sections.push(String::new());
        }
        let mut state = 0u8;
        let mut milestone = false;
        let mut items: Vec<&str> = Vec::new();
        for t in meta.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            match t {
                "crit" => state = 3,
                "active" if state != 3 => state = 1,
                "done" if state != 3 => state = 2,
                "active" | "done" => {}
                "milestone" => milestone = true,
                _ => items.push(t),
            }
        }
        let prev = last_end.unwrap_or_else(|| {
            g.tasks
                .iter()
                .map(|t| t.start)
                .fold(f64::INFINITY, f64::min)
                .min(1.0e12)
        });
        let prev = if prev.is_finite() { prev } else { 0.0 };
        let resolve_start = |s: &str, ids: &[(String, f64)]| -> Option<f64> {
            if let Some(after) = s.strip_prefix("after ") {
                let mut latest: Option<f64> = None;
                for id in after.split_whitespace() {
                    if let Some((_, e)) = ids.iter().find(|(i, _)| i == id) {
                        latest = Some(latest.map_or(*e, |l: f64| l.max(*e)));
                    }
                }
                return latest;
            }
            parse_date(s, &fmt)
        };
        let (id, start, end) = match items.as_slice() {
            [] => (None, prev, prev + 1.0),
            [one] => {
                if let Some(d) = parse_duration(one) {
                    (None, prev, prev + d)
                } else if let Some(s) = resolve_start(one, &ids) {
                    if one.starts_with("after ") {
                        (None, s, s + 1.0)
                    } else {
                        (None, prev, s)
                    }
                } else {
                    (Some((*one).to_string()), prev, prev + 1.0)
                }
            }
            [a, b] => {
                let s = resolve_start(a, &ids).unwrap_or(prev);
                let e = parse_duration(b).map_or_else(|| parse_date(b, &fmt).unwrap_or(s + 1.0), |d| s + d);
                (None, s, e)
            }
            [id, a, b, ..] => {
                let s = resolve_start(a, &ids).unwrap_or(prev);
                let e = parse_duration(b).map_or_else(|| parse_date(b, &fmt).unwrap_or(s + 1.0), |d| s + d);
                (Some((*id).to_string()), s, e)
            }
        };
        let end = if end < start { start } else { end };
        if let Some(id) = id {
            ids.push((id, end));
        }
        last_end = Some(end);
        g.tasks.push(Task {
            name: clean_label(name.trim()),
            section,
            start,
            end,
            state,
            milestone: milestone || end <= start,
        });
    }
    g
}

// ---- timeline ----------------------------------------------------------------

fn parse_timeline(body: &[(Line, &str)]) -> Timeline {
    let mut t = Timeline::default();
    let mut section: Option<usize> = None;
    for (_, text) in body {
        let (word, rest) = split_word(text);
        match word {
            "title" => t.title = Some(clean_label(rest)),
            "section" => {
                t.sections.push(clean_label(rest));
                section = Some(t.sections.len() - 1);
            }
            _ => {
                let mut parts = text.split(':').map(str::trim);
                let first = parts.next().unwrap_or("");
                let events: Vec<String> = parts.filter(|p| !p.is_empty()).map(clean_label).collect();
                if first.is_empty() {
                    if let Some(last) = t.periods.last_mut() {
                        last.events.extend(events);
                    }
                } else {
                    t.periods.push(Period {
                        section,
                        label: clean_label(first),
                        events,
                    });
                }
            }
        }
    }
    t
}

// ---- journey -----------------------------------------------------------------

fn parse_journey(body: &[(Line, &str)]) -> Journey {
    let mut j = Journey::default();
    let mut section: Option<usize> = None;
    for (_, text) in body {
        let (word, rest) = split_word(text);
        match word {
            "title" => j.title = Some(clean_label(rest)),
            "section" => {
                j.sections.push(clean_label(rest));
                section = Some(j.sections.len() - 1);
            }
            _ => {
                let mut parts = text.split(':').map(str::trim);
                let name = parts.next().unwrap_or("");
                let score = parts.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(3.0);
                let mut actors = Vec::new();
                for a in parts.next().unwrap_or("").split(',').map(str::trim) {
                    if a.is_empty() {
                        continue;
                    }
                    let idx = match j.actors.iter().position(|x| x == a) {
                        Some(i) => i,
                        None => {
                            j.actors.push(a.to_string());
                            j.actors.len() - 1
                        }
                    };
                    actors.push(idx);
                }
                j.tasks.push(Step {
                    section,
                    name: clean_label(name),
                    score: score.round().clamp(1.0, 5.0) as u8,
                    actors,
                });
            }
        }
    }
    j
}

// ---- quadrant ----------------------------------------------------------------

fn parse_quadrant(body: &[(Line, &str)]) -> Quadrant {
    let mut q = Quadrant::default();
    for (_, text) in body {
        let (word, rest) = split_word(text);
        let axis = |rest: &str| -> (String, String) {
            match rest.split_once("-->") {
                Some((a, b)) => (clean_label(a.trim()), clean_label(b.trim())),
                None => (clean_label(rest.trim()), String::new()),
            }
        };
        match word {
            "title" => q.title = Some(clean_label(rest)),
            "x-axis" => q.x = axis(rest),
            "y-axis" => q.y = axis(rest),
            "quadrant-1" => q.quadrants[0] = clean_label(rest),
            "quadrant-2" => q.quadrants[1] = clean_label(rest),
            "quadrant-3" => q.quadrants[2] = clean_label(rest),
            "quadrant-4" => q.quadrants[3] = clean_label(rest),
            _ => {
                if let Some((name, pos)) = text.split_once(':')
                    && let Some(open) = pos.find('[')
                    && let Some(close) = pos.find(']')
                    && open < close
                {
                    let mut it = pos[open + 1..close].split(',').map(|v| v.trim().parse::<f32>());
                    if let (Some(Ok(x)), Some(Ok(y))) = (it.next(), it.next()) {
                        q.points.push((
                            clean_label(name.trim().trim_matches('"')),
                            x.clamp(0.0, 1.0),
                            y.clamp(0.0, 1.0),
                        ));
                    }
                }
            }
        }
    }
    q
}

// ---- layout ------------------------------------------------------------------

fn text(at: [f32; 2], s: &str, size: f32, anchor: Anchor, bold: bool, tone: Tone) -> Decor {
    Decor::Text {
        at,
        text: s.to_string(),
        size,
        anchor,
        bold,
        tone,
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32, radius: f32, fill: Tone, stroke: Option<Tone>) -> Decor {
    Decor::Rect {
        rect: Rect { x, y, w, h },
        radius,
        fill,
        stroke,
    }
}

fn line(pts: Vec<[f32; 2]>, width: f32, dashed: bool, tone: Tone) -> Decor {
    Decor::Line {
        pts,
        width,
        dashed,
        tone,
    }
}

/// Moves the primitives so the drawing starts at the margin and wraps them in a layout.
fn assemble(mut decor: Vec<Decor>) -> Layout {
    let mut bb: Option<Rect> = None;
    for d in &decor {
        let b = d.bounds();
        bb = Some(bb.map_or(b, |a| a.union(b)));
    }
    let bb = bb.unwrap_or_default();
    let (dx, dy) = (MARGIN - bb.x, MARGIN - bb.y);
    for d in &mut decor {
        d.translate(dx, dy);
    }
    Layout {
        nodes: Vec::new(),
        node_text: Vec::new(),
        records: Vec::new(),
        clusters: Vec::new(),
        edges: Vec::new(),
        edge_text: Vec::new(),
        bounds: Rect {
            x: 0.0,
            y: 0.0,
            w: bb.w + 2.0 * MARGIN,
            h: bb.h + 2.0 * MARGIN,
        },
        decor,
        slots: Vec::new(),
    }
}

/// Lays out a chart.
pub(crate) fn layout(
    chart: &Chart,
    measurer: &Measurer,
    m: &Metrics,
    cancel: &AtomicBool,
) -> Result<Layout, LayoutError> {
    check_cancel(cancel)?;
    let decor = match chart {
        Chart::Pie {
            title,
            show_data,
            slices,
        } => pie(title.as_deref(), *show_data, slices, measurer),
        Chart::Gantt(g) => gantt(g, measurer),
        Chart::Timeline(t) => timeline(t, measurer),
        Chart::Journey(j) => journey(j, measurer),
        Chart::Quadrant(q) => quadrant(q, measurer),
    };
    let _ = m;
    Ok(assemble(decor))
}

fn pie(title: Option<&str>, show_data: bool, slices: &[(String, f64)], me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    let total: f64 = slices.iter().map(|s| s.1).sum();
    let r = 140.0f32;
    let (cx, cy) = (r + 10.0, r + 40.0);
    if let Some(t) = title {
        d.push(text([cx, 8.0], t, 18.0, Anchor::Middle, true, Tone::Ink));
    }
    if total <= 0.0 {
        return d;
    }
    let mut order: Vec<usize> = (0..slices.len()).collect();
    order.sort_by(|&a, &b| slices[b].1.total_cmp(&slices[a].1));
    let mut a = -std::f32::consts::FRAC_PI_2;
    let mut legend_y = cy - r + 4.0;
    let lx = cx + r + 40.0;
    for (rank, &i) in order.iter().enumerate() {
        let (label, v) = &slices[i];
        let frac = (*v / total) as f32;
        let sweep = frac * std::f32::consts::TAU;
        let tone = Tone::Swatch(rank as u8);
        if frac > 0.9999 {
            d.push(Decor::Circle {
                c: [cx, cy],
                r,
                fill: tone,
                stroke: Some(Tone::Canvas),
            });
        } else {
            d.push(Decor::Wedge {
                c: [cx, cy],
                r,
                a0: a,
                a1: a + sweep,
                tone,
            });
        }
        if frac >= 0.03 {
            let mid = a + sweep / 2.0;
            let rr = r * 0.68;
            d.push(text(
                [cx + rr * mid.cos(), cy + rr * mid.sin()],
                &format!("{}%", (frac * 100.0).round()),
                13.0,
                Anchor::Middle,
                true,
                Tone::Ink,
            ));
        }
        a += sweep;
        d.push(rect(lx, legend_y - 7.0, 14.0, 14.0, 3.0, tone, Some(tone)));
        let text_label = if show_data {
            format!("{label} [{}]", fmt_num(*v))
        } else {
            label.clone()
        };
        let _ = me;
        d.push(text([lx + 22.0, legend_y], &text_label, 13.0, Anchor::Start, false, Tone::Ink));
        legend_y += 24.0;
    }
    d
}

fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.2}")
    }
}

fn tone_of(state: u8) -> Tone {
    match state {
        1 => Tone::Swatch(1),
        2 => Tone::Swatch(2),
        3 => Tone::Swatch(5),
        _ => Tone::Swatch(0),
    }
}

fn gantt(g: &Gantt, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    if g.tasks.is_empty() {
        return d;
    }
    let min = g.tasks.iter().map(|t| t.start).fold(f64::INFINITY, f64::min);
    let max = g.tasks.iter().map(|t| t.end).fold(f64::NEG_INFINITY, f64::max);
    let span = (max - min).max(1.0);
    let gutter = g
        .sections
        .iter()
        .map(|s| me.width(s, 13.0))
        .fold(0.0, f32::max)
        .max(40.0)
        + 24.0;
    let scale = (760.0 / span as f32).clamp(0.4, 90.0);
    let width = span as f32 * scale;
    let x_of = |day: f64| gutter + (day - min) as f32 * scale;
    let row = 32.0f32;
    let top = if g.title.is_some() { 40.0 } else { 0.0 } + 34.0;
    let rows = g.tasks.len() as f32;
    if let Some(t) = &g.title {
        d.push(text([gutter + width / 2.0, 8.0], t, 18.0, Anchor::Middle, true, Tone::Ink));
    }
    // Section bands.
    let mut y = top;
    for (si, s) in g.sections.iter().enumerate() {
        let n = g.tasks.iter().filter(|t| t.section == si).count() as f32;
        if n == 0.0 {
            continue;
        }
        if si % 2 == 1 {
            d.push(rect(0.0, y, gutter + width + 20.0, n * row, 0.0, Tone::Grid, None));
        }
        d.push(text([8.0, y + n * row / 2.0], s, 13.0, Anchor::Start, true, Tone::Ink));
        y += n * row;
    }
    // Axis ticks.
    let mut ticks: Vec<(f64, String)> = Vec::new();
    let steps = [1.0, 2.0, 7.0, 14.0, 30.0, 91.0, 182.0, 365.0, 730.0, 1825.0];
    let step = steps.iter().copied().find(|s| span / s <= 11.0).unwrap_or(3650.0);
    if step < 28.0 {
        let mut t = (min / step).ceil() * step;
        while t <= max + 1e-9 {
            let (y, m, dd) = civil_from_days(t.floor() as i64);
            let label = if span <= 3.0 && t.fract() != 0.0 {
                format!("{:02}:{:02}", (t.fract() * 24.0) as i64, 0)
            } else {
                format!("{y}-{m:02}-{dd:02}")
            };
            ticks.push((t, label));
            t += step;
        }
    } else {
        let months = (step / 30.4).round() as i64;
        let (mut y, mut m, _) = civil_from_days(min.floor() as i64);
        let first = days_from_civil(y, m, 1) as f64;
        if first < min {
            m += months;
            while m > 12 {
                m -= 12;
                y += 1;
            }
        }
        for _ in 0..400 {
            let t = days_from_civil(y, m, 1) as f64;
            if t > max {
                break;
            }
            let label = if months >= 12 { format!("{y}") } else { format!("{y}-{m:02}") };
            ticks.push((t, label));
            m += months;
            while m > 12 {
                m -= 12;
                y += 1;
            }
        }
    }
    let bottom = top + rows * row;
    d.push(line(vec![[gutter, top], [gutter + width, top]], 1.0, false, Tone::Muted));
    for (t, label) in &ticks {
        let x = x_of(*t);
        d.push(line(vec![[x, top], [x, bottom]], 1.0, true, Tone::Grid));
        d.push(text([x, top - 14.0], label, 12.0, Anchor::Middle, false, Tone::Muted));
    }
    // Bars.
    for (i, t) in g.tasks.iter().enumerate() {
        let cy = top + (i as f32 + 0.5) * row;
        let (x0, x1) = (x_of(t.start), x_of(t.end));
        let tone = tone_of(t.state);
        if t.milestone {
            d.push(Decor::Circle {
                c: [x0, cy],
                r: 8.0,
                fill: Tone::Swatch(3),
                stroke: Some(Tone::Swatch(3)),
            });
            d.push(text([x0 + 14.0, cy], &t.name, 12.5, Anchor::Start, false, Tone::Ink));
            continue;
        }
        let w = (x1 - x0).max(4.0);
        d.push(rect(x0, cy - 10.0, w, 20.0, 5.0, tone, Some(tone)));
        let tw = me.width(&t.name, 12.5);
        if tw + 12.0 <= w {
            d.push(text([x0 + w / 2.0, cy], &t.name, 12.5, Anchor::Middle, false, Tone::Ink));
        } else {
            d.push(text([x0 + w + 8.0, cy], &t.name, 12.5, Anchor::Start, false, Tone::Ink));
        }
    }
    d
}

fn timeline(t: &Timeline, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    if t.periods.is_empty() {
        return d;
    }
    let col_w = t
        .periods
        .iter()
        .flat_map(|p| std::iter::once(&p.label).chain(p.events.iter()))
        .map(|s| me.width(s, 13.0).min(190.0))
        .fold(80.0, f32::max)
        + 28.0;
    let gap = 18.0;
    let n = t.periods.len();
    let total = n as f32 * (col_w + gap) - gap;
    let mut y = 0.0;
    if let Some(title) = &t.title {
        d.push(text([total / 2.0, 8.0], title, 18.0, Anchor::Middle, true, Tone::Ink));
        y = 40.0;
    }
    let tone_for = |p: &Period, i: usize| Tone::Swatch(p.section.unwrap_or(i) as u8);
    if !t.sections.is_empty() {
        let mut i = 0;
        while i < n {
            let s = t.periods[i].section;
            let mut j = i;
            while j < n && t.periods[j].section == s {
                j += 1;
            }
            if let Some(si) = s {
                let x = i as f32 * (col_w + gap);
                let w = (j - i) as f32 * (col_w + gap) - gap;
                d.push(rect(x, y, w, 30.0, 6.0, Tone::Swatch(si as u8), Some(Tone::Swatch(si as u8))));
                d.push(text([x + w / 2.0, y + 15.0], &t.sections[si], 13.0, Anchor::Middle, true, Tone::Ink));
            }
            i = j;
        }
        y += 44.0;
    }
    let head_h = 38.0;
    let axis_y = y + head_h + 14.0;
    d.push(line(vec![[0.0, axis_y], [total, axis_y]], 2.0, false, Tone::Muted));
    for (i, p) in t.periods.iter().enumerate() {
        let x = i as f32 * (col_w + gap);
        let tone = tone_for(p, i);
        d.push(rect(x, y, col_w, head_h, 6.0, tone, Some(tone)));
        d.push(text([x + col_w / 2.0, y + head_h / 2.0], &p.label, 13.0, Anchor::Middle, true, Tone::Ink));
        d.push(line(vec![[x + col_w / 2.0, y + head_h], [x + col_w / 2.0, axis_y]], 1.5, false, Tone::Muted));
        let mut ey = axis_y + 16.0;
        for e in &p.events {
            let tb = me.wrap(e, 13.0, 17.0, col_w - 24.0);
            let h = tb.height + 14.0;
            d.push(line(vec![[x + col_w / 2.0, ey - 16.0], [x + col_w / 2.0, ey]], 1.0, true, Tone::Muted));
            d.push(rect(x, ey, col_w, h, 6.0, tone, Some(tone)));
            for (k, l) in tb.lines.iter().enumerate() {
                d.push(text(
                    [x + col_w / 2.0, ey + 7.0 + 8.5 + k as f32 * 17.0],
                    l,
                    13.0,
                    Anchor::Middle,
                    false,
                    Tone::Ink,
                ));
            }
            ey += h + 12.0;
        }
    }
    d
}

fn journey(j: &Journey, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    if j.tasks.is_empty() {
        return d;
    }
    let card_w = j
        .tasks
        .iter()
        .map(|t| me.width(&t.name, 13.0).min(150.0))
        .fold(90.0, f32::max)
        + 24.0;
    let gap = 18.0;
    let legend_w = j.actors.iter().map(|a| me.width(a, 13.0)).fold(0.0, f32::max) + 40.0;
    let x0 = if j.actors.is_empty() { 0.0 } else { legend_w + 24.0 };
    let n = j.tasks.len();
    let total = x0 + n as f32 * (card_w + gap) - gap;
    let mut y = 0.0;
    if let Some(title) = &j.title {
        d.push(text([total / 2.0, 8.0], title, 18.0, Anchor::Middle, true, Tone::Ink));
        y = 40.0;
    }
    if !j.sections.is_empty() {
        let mut i = 0;
        while i < n {
            let s = j.tasks[i].section;
            let mut k = i;
            while k < n && j.tasks[k].section == s {
                k += 1;
            }
            if let Some(si) = s {
                let x = x0 + i as f32 * (card_w + gap);
                let w = (k - i) as f32 * (card_w + gap) - gap;
                d.push(rect(x, y, w, 30.0, 6.0, Tone::Swatch(si as u8), Some(Tone::Swatch(si as u8))));
                d.push(text([x + w / 2.0, y + 15.0], &j.sections[si], 13.0, Anchor::Middle, true, Tone::Ink));
            }
            i = k;
        }
        y += 42.0;
    }
    let card_h = 50.0;
    let field_top = y + card_h + 30.0;
    let level = 38.0;
    let mut path = Vec::new();
    for (i, t) in j.tasks.iter().enumerate() {
        let x = x0 + i as f32 * (card_w + gap);
        let tone = Tone::Swatch(t.section.unwrap_or(0) as u8);
        d.push(rect(x, y, card_w, card_h, 6.0, tone, Some(tone)));
        let tb = me.wrap(&t.name, 13.0, 16.0, card_w - 16.0);
        for (k, l) in tb.lines.iter().enumerate() {
            let cy = y + card_h / 2.0 - (tb.lines.len() as f32 - 1.0) * 8.0 + k as f32 * 16.0;
            d.push(text([x + card_w / 2.0, cy], l, 13.0, Anchor::Middle, false, Tone::Ink));
        }
        let fx = x + card_w / 2.0;
        let fy = field_top + (5 - t.score) as f32 * level;
        path.push([fx, fy]);
        d.push(line(vec![[fx, y + card_h], [fx, fy]], 1.0, true, Tone::Grid));
    }
    if path.len() > 1 {
        d.push(line(path.clone(), 2.0, false, Tone::Muted));
    }
    for (t, p) in j.tasks.iter().zip(&path) {
        let tone = Tone::Swatch([5u8, 1, 3, 2, 2][(t.score - 1) as usize]);
        d.push(Decor::Circle {
            c: *p,
            r: 15.0,
            fill: tone,
            stroke: Some(tone),
        });
        for dx in [-5.0, 5.0] {
            d.push(Decor::Circle {
                c: [p[0] + dx, p[1] - 4.0],
                r: 1.7,
                fill: Tone::Ink,
                stroke: None,
            });
        }
        let mouth: Vec<[f32; 2]> = match t.score {
            4 | 5 => (0..=6)
                .map(|k| {
                    let u = k as f32 / 6.0;
                    [p[0] - 6.0 + 12.0 * u, p[1] + 4.0 + 3.5 * (1.0 - (2.0 * u - 1.0).powi(2))]
                })
                .collect(),
            3 => vec![[p[0] - 6.0, p[1] + 6.0], [p[0] + 6.0, p[1] + 6.0]],
            _ => (0..=6)
                .map(|k| {
                    let u = k as f32 / 6.0;
                    [p[0] - 6.0 + 12.0 * u, p[1] + 9.0 - 3.5 * (1.0 - (2.0 * u - 1.0).powi(2))]
                })
                .collect(),
        };
        d.push(line(mouth, 1.5, false, Tone::Ink));
        for (k, a) in t.actors.iter().enumerate() {
            d.push(Decor::Circle {
                c: [p[0] - (t.actors.len() as f32 - 1.0) * 6.0 + k as f32 * 12.0, p[1] + 26.0],
                r: 4.5,
                fill: Tone::Swatch(*a as u8 + 3),
                stroke: Some(Tone::Swatch(*a as u8 + 3)),
            });
        }
    }
    for (i, a) in j.actors.iter().enumerate() {
        let ly = field_top + i as f32 * 24.0;
        d.push(Decor::Circle {
            c: [6.0, ly],
            r: 5.0,
            fill: Tone::Swatch(i as u8 + 3),
            stroke: Some(Tone::Swatch(i as u8 + 3)),
        });
        d.push(text([18.0, ly], a, 13.0, Anchor::Start, false, Tone::Ink));
    }
    d
}

fn quadrant(q: &Quadrant, me: &Measurer) -> Vec<Decor> {
    let mut d = Vec::new();
    let size = 420.0f32;
    let left = me.width(&q.y.0, 12.0).max(me.width(&q.y.1, 12.0)) + 16.0;
    let top = if q.title.is_some() { 44.0 } else { 8.0 };
    let (sx, sy) = (left, top);
    if let Some(t) = &q.title {
        d.push(text([sx + size / 2.0, 14.0], t, 18.0, Anchor::Middle, true, Tone::Ink));
    }
    let half = size / 2.0;
    let cells = [(1usize, half, 0.0, 2u8), (0, 0.0, 0.0, 1), (2, 0.0, half, 4), (3, half, half, 0)];
    // Order: q1 top-right, q2 top-left, q3 bottom-left, q4 bottom-right.
    let cells = [
        (0usize, half, 0.0, cells[0].3),
        (1, 0.0, 0.0, cells[1].3),
        (2, 0.0, half, cells[2].3),
        (3, half, half, cells[3].3),
    ];
    for (i, ox, oy, tone) in cells {
        d.push(rect(sx + ox, sy + oy, half, half, 0.0, Tone::Swatch(tone), Some(Tone::Canvas)));
        if !q.quadrants[i].is_empty() {
            d.push(text(
                [sx + ox + half / 2.0, sy + oy + 18.0],
                &q.quadrants[i],
                13.0,
                Anchor::Middle,
                true,
                Tone::Ink,
            ));
        }
    }
    d.push(text([sx, sy + size + 16.0], &q.x.0, 12.0, Anchor::Start, false, Tone::Muted));
    d.push(text([sx + size, sy + size + 16.0], &q.x.1, 12.0, Anchor::End, false, Tone::Muted));
    d.push(text([sx - 8.0, sy + size - 8.0], &q.y.0, 12.0, Anchor::End, false, Tone::Muted));
    d.push(text([sx - 8.0, sy + 8.0], &q.y.1, 12.0, Anchor::End, false, Tone::Muted));
    for (name, x, y) in &q.points {
        let c = [sx + x * size, sy + (1.0 - y) * size];
        d.push(Decor::Circle {
            c,
            r: 6.0,
            fill: Tone::Swatch(4),
            stroke: Some(Tone::Ink),
        });
        d.push(text([c[0] + 11.0, c[1]], name, 12.0, Anchor::Start, false, Tone::Ink));
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart(src: &str, family: Family) -> Chart {
        let p = parse(src, family);
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
        (*p.graph.chart.unwrap()).clone()
    }

    #[test]
    fn pie_parses_title_and_slices() {
        let Chart::Pie { title, show_data, slices } =
            chart("pie showData title Pets\n  \"Dogs\" : 386\n  \"Cats\" : 85.5\n", Family::Pie)
        else {
            panic!()
        };
        assert_eq!(title.as_deref(), Some("Pets"));
        assert!(show_data);
        assert_eq!(slices.len(), 2);
        assert_eq!(slices[1].1, 85.5);
    }

    #[test]
    fn dates_round_trip() {
        let d = parse_date("2024-02-29", "YYYY-MM-DD").unwrap();
        assert_eq!(civil_from_days(d as i64), (2024, 2, 29));
        assert_eq!(parse_date("29/02/24", "DD/MM/YY"), Some(d));
        assert!(parse_date("2024-13-01", "YYYY-MM-DD").is_none());
        assert_eq!(parse_duration("2w"), Some(14.0));
    }

    #[test]
    fn gantt_resolves_after_and_durations() {
        let Chart::Gantt(g) = chart(
            "gantt\n title Plan\n dateFormat YYYY-MM-DD\n section A\n Design :a1, 2024-01-01, 10d\n Build :a2, after a1, 5d\n Ship :crit, 2024-01-20, 2024-01-22\n Done :milestone, m1, 2024-01-22, 0d\n",
            Family::Gantt,
        ) else {
            panic!()
        };
        assert_eq!(g.tasks.len(), 4);
        assert_eq!(g.tasks[1].start, g.tasks[0].end);
        assert_eq!(g.tasks[1].end - g.tasks[1].start, 5.0);
        assert_eq!(g.tasks[2].state, 3);
        assert!(g.tasks[3].milestone);
    }

    #[test]
    fn timeline_journey_quadrant_parse() {
        let Chart::Timeline(t) = chart("timeline\n title H\n section Early\n 2002 : A : B\n      : C\n 2004 : D\n", Family::Timeline) else { panic!() };
        assert_eq!(t.periods.len(), 2);
        assert_eq!(t.periods[0].events, vec!["A", "B", "C"]);
        let Chart::Journey(j) = chart("journey\n title T\n section S\n Tea: 5: Me, Cat\n Work: 1: Me\n", Family::Journey) else { panic!() };
        assert_eq!(j.tasks.len(), 2);
        assert_eq!(j.actors, vec!["Me", "Cat"]);
        let Chart::Quadrant(q) = chart("quadrantChart\n x-axis Low --> High\n y-axis Bad --> Good\n quadrant-1 Top\n A: [0.3, 0.6]\n", Family::Quadrant) else { panic!() };
        assert_eq!(q.x.1, "High");
        assert_eq!(q.points.len(), 1);
    }
}
