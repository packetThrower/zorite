//! **Timeline**: every page at the day it was created, in a lane for its
//! topic, with links arcing between them across time. Shows when a topic
//! started, went quiet or came back. Unlinked pages run along the bottom.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{Bounds, Hsla, SharedString, fill, point, px, size};
use rust_i18n::t;

use super::style::{Align, Frame, GraphStyle, Layout, Painter, VAlign, palette};
use super::{Graph, Kind, days_from_civil};
use crate::{dates, theme};

pub struct Timeline;

/// World width of the whole time span.
const SPAN_W: f32 = 1800.0;
/// Lanes share about 60% of the span's width, so the timeline reads closer
/// to the window's shape than a thin strip; never thinner than this.
const MIN_LANE_H: f32 = 90.0;
/// Topics with a lane of their own; smaller ones share "Other".
const MAX_LANES: usize = 7;
/// Vertical slots within a lane for pages created close together.
const SLOTS: [f32; 5] = [0.0, -16.0, 16.0, -32.0, 32.0];

/// The lane of every node: topics by size, then "Other", then the
/// unlinked pages' strip last.
struct Lanes {
    of: Vec<usize>,
    labels: Vec<SharedString>,
    colors: Vec<Hsla>,
    /// World height of one lane.
    h: f32,
}

fn lanes(g: &Graph) -> Lanes {
    let own = g.groups.len().min(MAX_LANES);
    let mut labels: Vec<SharedString> = g.groups[..own]
        .iter()
        .map(|grp| grp.label.clone())
        .collect();
    let mut colors: Vec<Hsla> = (0..own).map(palette).collect();
    let other = g.groups.len() > own;
    if other {
        labels.push(t!("graph.lane_other").into());
        colors.push(theme::text_secondary());
    }
    let strip = labels.len();
    let loose = g.nodes.iter().filter(|n| n.group.is_none()).count();
    labels.push(t!("graph.unlinked", count = loose).into());
    colors.push(theme::text_tertiary());
    let of = g
        .nodes
        .iter()
        .map(|n| match n.group {
            Some(gi) if gi < own => gi,
            Some(_) => own,
            None => strip,
        })
        .collect();
    let h = (SPAN_W * 0.6 / labels.len() as f32).max(MIN_LANE_H);
    Lanes {
        of,
        labels,
        colors,
        h,
    }
}

/// The time axis: the first day and the world units per day.
fn axis(g: &Graph) -> (i32, f32) {
    let days = g.nodes.iter().filter_map(|n| n.day);
    let (lo, hi) = days.fold((i32::MAX, i32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
    if lo > hi {
        return (today(), SPAN_W / 365.0);
    }
    (lo, SPAN_W / (hi - lo).max(30) as f32)
}

fn today() -> i32 {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    (secs / 86_400) as i32
}

fn year_of(day: i32) -> i32 {
    let mut y = 1970 + (day as f32 / 365.2425).floor() as i32;
    while days_from_civil(y + 1, 1, 1) <= day {
        y += 1;
    }
    while days_from_civil(y, 1, 1) > day {
        y -= 1;
    }
    y
}

impl GraphStyle for Timeline {
    fn id(&self) -> &'static str {
        "timeline"
    }

    fn name(&self) -> SharedString {
        t!("graph.style_timeline").into()
    }

    fn layout(&self, g: &Graph, _focus: Option<usize>) -> Layout {
        let lanes = lanes(g);
        let strip = lanes.labels.len() - 1;
        let (start, per_day) = axis(g);
        let x_of = |i: usize| {
            g.nodes[i]
                .day
                .map_or(-60.0, |d| (d - start) as f32 * per_day)
        };
        let mut pos = vec![[0.0f32; 2]; g.nodes.len()];
        // Within a lane, left to right: take the first slot whose last page
        // is far enough back, so pages close in time stack instead of overlap.
        let mut by_lane: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, &l) in lanes.of.iter().enumerate() {
            by_lane.entry(l).or_default().push(i);
        }
        for (lane, mut members) in by_lane {
            members.sort_by(|&a, &b| x_of(a).total_cmp(&x_of(b)));
            let y = lane as f32 * lanes.h;
            let mut last = [f32::MIN; SLOTS.len()];
            for (k, i) in members.into_iter().enumerate() {
                let x = x_of(i);
                let slot = if lane == strip {
                    k % 3
                } else {
                    (0..SLOTS.len())
                        .find(|&s| x - last[s] > 14.0)
                        .unwrap_or(k % SLOTS.len())
                };
                last[slot] = x;
                let dy = if lane == strip {
                    SLOTS[slot] * 0.4
                } else {
                    SLOTS[slot]
                };
                pos[i] = [x, y + dy];
            }
        }
        Layout::all(pos)
    }

    fn paint(&self, g: &Graph, f: &Frame, p: &mut Painter) {
        let v = p.view;
        let lanes = lanes(g);
        let strip = lanes.labels.len() - 1;
        let (start, per_day) = axis(g);
        let b = p.bounds;
        let lane_y = |l: usize| v.at([0.0, l as f32 * lanes.h]).y;
        // Lane bands, every other one shaded, full width.
        for l in (0..lanes.labels.len()).step_by(2) {
            let h = v.len(lanes.h, 1.0);
            let top = lane_y(l) - h / 2.0;
            p.window.paint_quad(fill(
                Bounds::new(point(b.origin.x, top), size(b.size.width, h)),
                theme::text_primary().opacity(0.025),
            ));
        }
        // Years, and months too once a month is wide enough on screen to
        // label (a short history, or zoomed in). The first label on screen
        // always carries its year.
        let x_at = |day: i32| v.at([(day - start) as f32 * per_day, 0.0]).x;
        let last = g.nodes.iter().filter_map(|n| n.day).max().unwrap_or(start);
        let months = per_day * 30.0 * p.zoom() >= 44.0;
        let mut year_shown = false;
        for y in year_of(start)..=year_of(last.max(today())) + 1 {
            for m in 1..=12 {
                if m > 1 && !months {
                    break;
                }
                let x = x_at(days_from_civil(y, m, 1));
                if x < b.origin.x || x > b.origin.x + b.size.width {
                    continue;
                }
                let strong = m == 1;
                p.window.paint_quad(fill(
                    Bounds::new(point(x, b.origin.y), size(px(1.0), b.size.height)),
                    theme::divider().opacity(if strong { 0.6 } else { 0.25 }),
                ));
                let label: SharedString = if strong {
                    y.to_string().into()
                } else {
                    let name = time::Month::try_from(m as u8)
                        .map(dates::month_name)
                        .unwrap_or_default();
                    if year_shown {
                        name
                    } else {
                        format!("{name} {y}")
                    }
                    .into()
                };
                year_shown = true;
                p.text(
                    &label,
                    point(x + px(5.0), b.origin.y + px(8.0)),
                    11.0,
                    if strong {
                        theme::text_secondary()
                    } else {
                        theme::text_tertiary()
                    },
                    Align::Left,
                    VAlign::Top,
                    strong,
                );
            }
        }
        let now = x_at(today());
        p.stroke(1.0, theme::accent().opacity(0.6), Some([3.0, 3.0]), |pb| {
            pb.move_to(point(now, b.origin.y));
            pb.line_to(point(now, b.origin.y + b.size.height));
            true
        });
        // Links: an arc over the lane within a topic, an S-curve across lanes.
        let quiet = if f.hover.is_some() || f.matches.is_some() {
            0.05
        } else {
            0.32
        };
        let arc = |pb: &mut gpui::PathBuilder, a: usize, bb: usize| {
            let (mut pa, mut pc) = (v.at(f.pos[a]), v.at(f.pos[bb]));
            if pa.x > pc.x {
                std::mem::swap(&mut pa, &mut pc);
            }
            pb.move_to(pa);
            if lanes.of[a] == lanes.of[bb] {
                let lift = (px(10.0) + (pc.x - pa.x) * 0.18).min(v.len(lanes.h * 0.9, 1.0));
                pb.curve_to(pc, point((pa.x + pc.x) / 2.0, pa.y.min(pc.y) - lift));
            } else {
                let my = (pa.y + pc.y) / 2.0;
                let dx = (pc.x - pa.x) * 0.3;
                pb.cubic_bezier_to(pc, point(pa.x + dx, my), point(pc.x - dx, my));
            }
        };
        for l in 0..lanes.labels.len() {
            let color = lanes.colors[l].opacity(quiet);
            p.stroke(1.0, color, None, |pb| {
                let mut any = false;
                for &(a, bb) in &g.edges {
                    if lanes.of[a.min(bb)] == l && !f.edge_lit(a, bb) {
                        arc(pb, a, bb);
                        any = true;
                    }
                }
                any
            });
        }
        p.stroke(1.6, theme::accent(), None, |pb| {
            let mut any = false;
            for &(a, bb) in &g.edges {
                if f.edge_lit(a, bb) {
                    arc(pb, a, bb);
                    any = true;
                }
            }
            any
        });
        // Pages: dots in their lane color; unlinked ones as thin ticks.
        for (i, n) in g.nodes.iter().enumerate() {
            let c = v.at(f.pos[i]);
            let a = if f.dimmed(i) { 0.2 } else { 1.0 };
            if lanes.of[i] == strip {
                let color = if f.lit(i) {
                    theme::accent()
                } else {
                    theme::text_tertiary().opacity(0.5 * a)
                };
                let w = px(if f.lit(i) { 2.0 } else { 1.2 });
                p.window.paint_quad(fill(
                    Bounds::new(point(c.x - w / 2.0, c.y - px(4.0)), size(w, px(8.0))),
                    color,
                ));
                continue;
            }
            let color = if f.lit(i) {
                theme::accent()
            } else {
                lanes.colors[lanes.of[i]]
            };
            p.node(
                c,
                v.len(n.radius * 0.55, 2.5),
                color.opacity(a),
                n.kind == Kind::Board,
            );
        }
        // Lane names, pinned top-right of each band: the panel covers the
        // left edge, and the band's top is where its dots are sparsest. Before
        // the page names, so those keep clear of them.
        for (l, label) in lanes.labels.iter().enumerate() {
            let y = lane_y(l) - v.len(lanes.h, 1.0) / 2.0 + px(4.0);
            if y < b.origin.y || y > b.origin.y + b.size.height {
                continue;
            }
            let x = b.origin.x + b.size.width - px(16.0);
            p.dot(point(x, y + px(8.0)), px(3.5), lanes.colors[l]);
            let (a, va) = (Align::Right, VAlign::Top);
            let color = theme::text_secondary();
            p.label_at(
                label,
                point(x - px(10.0), y),
                12.0,
                color,
                a,
                va,
                true,
                true,
            );
        }
        // Names on hover and search.
        for i in f.label_order(g) {
            if (f.hover.is_some() || f.matches.is_some()) && !f.dimmed(i) {
                let n = &g.nodes[i];
                let c = v.at(f.pos[i]);
                let at = point(c.x, c.y - v.len(n.radius * 0.55, 2.5) - px(4.0));
                p.label(&n.title, at, theme::text_primary(), f.lit(i), f.lit(i));
            }
        }
    }

    fn draggable(&self) -> bool {
        false
    }

    fn hint(&self) -> Option<SharedString> {
        Some(t!("graph.hint_timeline").into())
    }
}
