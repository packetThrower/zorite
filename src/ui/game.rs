//! **Blockdown** — the hidden brick-breaker (type `/play` in any note). You
//! are the caret: the paddle is a cursor bar, the ball a bullet point, and
//! the bricks markdown constructs with matching powers — `**bold**` takes
//! two hits and widens the paddle, `` `code` `` is armored and grants
//! slow-mo, `[[wiki]]` opens portals through the side walls, `#tag` splits
//! the ball, `*italic*` deflects at a skew. Arrow keys (or the mouse) move,
//! Space launches, Esc slips back out. Everything paints in the active
//! theme's tokens, so the game re-skins with the app.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    Bounds, Context, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Pixels, SharedString, Styled,
    TextRun, canvas, div, fill, point, px, size,
};

use crate::app::AppView;
use crate::theme;

/// Logical world size; the canvas letterboxes it to fit.
pub const W: f32 = 800.0;
pub const H: f32 = 560.0;

const PADDLE_Y: f32 = 528.0;
const PADDLE_H: f32 = 12.0;
const PADDLE_W: f32 = 110.0;
const PADDLE_W_WIDE: f32 = 180.0;
const PADDLE_SPEED: f32 = 560.0;
const BALL_R: f32 = 7.0;
const BALL_SPEED: f32 = 400.0;
const MAX_BALLS: usize = 6;

#[derive(Clone, Copy, PartialEq)]
pub enum BrickKind {
    Quote,  // > plain filler
    Tag,    // # multiball
    Italic, // * skew deflection
    Wiki,   // [[ ]] portal walls
    Bold,   // ** two hits, wide paddle
    Code,   // ` armored, slow-mo
}

impl BrickKind {
    fn hp(self) -> u8 {
        match self {
            Self::Code => 3,
            Self::Bold => 2,
            _ => 1,
        }
    }

    fn points(self) -> u32 {
        match self {
            Self::Quote => 10,
            Self::Italic => 15,
            Self::Bold => 20,
            Self::Tag | Self::Wiki => 25,
            Self::Code => 30,
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::Quote => ">",
            Self::Tag => "#",
            Self::Italic => "*",
            Self::Wiki => "[[ ]]",
            Self::Bold => "**",
            Self::Code => "`",
        }
    }
}

pub struct Brick {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    kind: BrickKind,
    hp: u8,
}

#[derive(Clone, Copy)]
pub struct Ball {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Phase {
    /// Ball riding the paddle, waiting for launch.
    Ready,
    Running,
    Paused,
    Over,
    Won,
}

pub struct GameState {
    pub focus: FocusHandle,
    paddle_x: f32,
    balls: Vec<Ball>,
    bricks: Vec<Brick>,
    score: u32,
    lives: u8,
    level: u32,
    /// A finished run's (score, level), waiting for the host to write it to
    /// the high-score page.
    record: Option<(u32, u32)>,
    phase: Phase,
    left_held: bool,
    right_held: bool,
    /// Effect countdowns, seconds.
    wide: f32,
    slow: f32,
    portal: f32,
    /// Deterministic wobble source for italic deflections.
    seed: u32,
    bounds: Rc<Cell<Bounds<Pixels>>>,
}

impl GameState {
    pub fn new(focus: FocusHandle) -> Self {
        // Time-seeded: every run scatters its specials differently.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0xC0FFEE)
            | 1;
        let mut this = Self {
            focus,
            paddle_x: W / 2.0,
            balls: Vec::new(),
            bricks: Vec::new(),
            score: 0,
            lives: 3,
            level: 1,
            record: None,
            phase: Phase::Ready,
            left_held: false,
            right_held: false,
            wide: 0.0,
            slow: 0.0,
            portal: 0.0,
            seed,
            bounds: Rc::default(),
        };
        this.bricks = build_level(&mut || this_rng(&mut this.seed), 1);
        this.reset_ball();
        this
    }

    fn restart(&mut self) {
        let focus = self.focus.clone();
        *self = Self::new(focus);
    }

    /// Advance after a cleared board: faster ball, more specials, bonus life.
    fn next_level(&mut self) {
        self.level += 1;
        self.lives = (self.lives + 1).min(5);
        self.wide = 0.0;
        self.slow = 0.0;
        self.portal = 0.0;
        let mut seed = self.seed;
        self.bricks = build_level(&mut || this_rng(&mut seed), self.level);
        self.seed = seed;
        self.reset_ball();
    }

    /// Ball speed for the current level (+8% per level, capped at 1.6×).
    fn speed(&self) -> f32 {
        BALL_SPEED * (1.0 + 0.08 * (self.level - 1) as f32).min(1.6)
    }

    /// A finished run's score, handed over exactly once.
    pub fn take_record(&mut self) -> Option<(u32, u32)> {
        self.record.take()
    }

    fn paddle_w(&self) -> f32 {
        if self.wide > 0.0 {
            PADDLE_W_WIDE
        } else {
            PADDLE_W
        }
    }

    fn reset_ball(&mut self) {
        self.balls.clear();
        self.balls.push(Ball {
            x: self.paddle_x,
            y: PADDLE_Y - BALL_R - 1.0,
            vx: 0.0,
            vy: 0.0,
        });
        self.phase = Phase::Ready;
    }

    fn launch(&mut self) {
        match self.phase {
            Phase::Ready => {
                // Slightly off-vertical so the first bounce isn't a metronome.
                let speed = self.speed();
                let (vx, vy) = (speed * 0.25, -speed * 0.97);
                if let Some(b) = self.balls.first_mut() {
                    b.vx = vx;
                    b.vy = vy;
                }
                self.phase = Phase::Running;
            }
            Phase::Won => self.next_level(),
            Phase::Over => self.restart(),
            Phase::Paused => self.phase = Phase::Running,
            Phase::Running => {}
        }
    }

    fn toggle_pause(&mut self) {
        self.phase = match self.phase {
            Phase::Running => Phase::Paused,
            Phase::Paused => Phase::Running,
            other => other,
        };
    }

    /// A cheap deterministic "random" in [-1, 1] (no `rand`; xorshift).
    fn wobble(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 2000) as f32 / 1000.0 - 1.0
    }

    /// One fixed timestep. Returns true when anything moved (repaint).
    pub fn step(&mut self, dt: f32) -> bool {
        let paddle_w = self.paddle_w();
        // Paddle from held arrows; the ready ball rides along.
        let dir = (self.right_held as i32 - self.left_held as i32) as f32;
        if dir != 0.0 {
            self.paddle_x =
                (self.paddle_x + dir * PADDLE_SPEED * dt).clamp(paddle_w / 2.0, W - paddle_w / 2.0);
        }
        if self.phase == Phase::Ready
            && let Some(b) = self.balls.first_mut()
        {
            b.x = self.paddle_x;
            return dir != 0.0;
        }
        if self.phase != Phase::Running {
            return false;
        }
        for t in [&mut self.wide, &mut self.slow, &mut self.portal] {
            *t = (*t - dt).max(0.0);
        }
        let speed_scale = if self.slow > 0.0 { 0.55 } else { 1.0 };
        let portal = self.portal > 0.0;
        let dt = dt * speed_scale;

        let mut spawned: Vec<Ball> = Vec::new();
        let mut lost: Vec<usize> = Vec::new();
        for i in 0..self.balls.len() {
            let mut b = self.balls[i];
            b.x += b.vx * dt;
            b.y += b.vy * dt;
            // Walls: portals wrap, otherwise reflect.
            if portal {
                if b.x < -BALL_R {
                    b.x = W + BALL_R;
                } else if b.x > W + BALL_R {
                    b.x = -BALL_R;
                }
            } else if b.x < BALL_R {
                b.x = BALL_R;
                b.vx = b.vx.abs();
            } else if b.x > W - BALL_R {
                b.x = W - BALL_R;
                b.vx = -b.vx.abs();
            }
            if b.y < BALL_R {
                b.y = BALL_R;
                b.vy = b.vy.abs();
            }
            // Paddle.
            let half = self.paddle_w() / 2.0;
            if b.vy > 0.0
                && b.y + BALL_R >= PADDLE_Y
                && b.y + BALL_R <= PADDLE_Y + PADDLE_H + 8.0
                && (b.x - self.paddle_x).abs() <= half + BALL_R
            {
                // Exit angle from where the paddle was struck.
                let speed = self.speed();
                let off = ((b.x - self.paddle_x) / half).clamp(-1.0, 1.0);
                let vx = off * speed * 0.75;
                b.vx = vx;
                b.vy = -(speed * speed - vx * vx).max(1.0).sqrt();
                b.y = PADDLE_Y - BALL_R;
            }
            // Bricks: first overlap wins this step.
            if let Some(bi) = self
                .bricks
                .iter()
                .position(|br| br.hp > 0 && circle_hits_rect(b.x, b.y, BALL_R, br))
            {
                let (kind, cx_, cy_) = {
                    let br = &mut self.bricks[bi];
                    br.hp -= 1;
                    (br.kind, br.x + br.w / 2.0, br.y + br.h / 2.0)
                };
                // Reflect off the shallower axis of penetration.
                let br = &self.bricks[bi];
                let dx = (b.x - cx_).abs() / (br.w / 2.0);
                let dy = (b.y - cy_).abs() / (br.h / 2.0);
                if dx > dy {
                    b.vx = if b.x < cx_ { -b.vx.abs() } else { b.vx.abs() };
                } else {
                    b.vy = if b.y < cy_ { -b.vy.abs() } else { b.vy.abs() };
                }
                if self.bricks[bi].hp == 0 {
                    self.score += kind.points();
                    match kind {
                        BrickKind::Bold => self.wide = 10.0,
                        BrickKind::Code => self.slow = 5.0,
                        BrickKind::Wiki => self.portal = 5.0,
                        BrickKind::Tag => {
                            if self.balls.len() + spawned.len() < MAX_BALLS {
                                spawned.push(Ball {
                                    x: b.x,
                                    y: b.y,
                                    vx: -b.vx,
                                    vy: -b.vy.abs(),
                                });
                            }
                        }
                        BrickKind::Italic => {
                            let skew = self.wobble() * 0.6;
                            let (vx, vy) = (b.vx, b.vy);
                            b.vx = vx * skew.cos() - vy * skew.sin();
                            b.vy = vx * skew.sin() + vy * skew.cos();
                        }
                        BrickKind::Quote => {}
                    }
                }
            }
            // Floor.
            if b.y > H + BALL_R {
                lost.push(i);
            } else {
                self.balls[i] = b;
            }
        }
        for i in lost.into_iter().rev() {
            self.balls.remove(i);
        }
        self.balls.extend(spawned);
        if self.balls.is_empty() {
            if self.lives <= 1 {
                self.lives = 0;
                self.phase = Phase::Over;
                // Hand the run to the host once, for the high-score page.
                self.record = Some((self.score, self.level));
            } else {
                self.lives -= 1;
                self.reset_ball();
            }
        }
        if !self.bricks.iter().any(|b| b.hp > 0) {
            self.phase = Phase::Won;
        }
        true
    }

    fn set_paddle_from_mouse(&mut self, world_x: f32) {
        let half = self.paddle_w() / 2.0;
        self.paddle_x = world_x.clamp(half, W - half);
    }

    /// Window point → world coords, through the letterbox transform.
    fn to_world(&self, p: gpui::Point<Pixels>) -> (f32, f32) {
        let b = self.bounds.get();
        let (bw, bh) = (f32::from(b.size.width), f32::from(b.size.height));
        let scale = (bw / W).min(bh / H).max(0.01);
        let ox = f32::from(b.origin.x) + (bw - W * scale) / 2.0;
        let oy = f32::from(b.origin.y) + (bh - H * scale) / 2.0;
        ((f32::from(p.x) - ox) / scale, (f32::from(p.y) - oy) / scale)
    }
}

/// Step a xorshift word — the game's whole RNG (`rand` stays out of the tree).
fn this_rng(seed: &mut u32) -> u32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    *seed
}

/// One level: an armored code row on top, a bold row under it, then a quote
/// field with scattered specials (powers stay rare enough to read as events —
/// a full row of portals felt like a wall bug, not a power). The scatter and
/// the gap holes come from the run's seed, so every game lays out a little
/// differently; higher levels sprinkle more specials.
fn build_level(rng: &mut impl FnMut() -> u32, level: u32) -> Vec<Brick> {
    const COLS: usize = 10;
    const FIELD_ROWS: std::ops::Range<usize> = 2..6;
    // Scatter specials over distinct field cells.
    let mut specials: Vec<(usize, usize, BrickKind)> = Vec::new();
    let mut place = |rng: &mut dyn FnMut() -> u32, kind: BrickKind, n: usize| {
        let mut placed = 0;
        let mut guard = 0;
        while placed < n && guard < 200 {
            guard += 1;
            let r = FIELD_ROWS.start + (rng() as usize) % FIELD_ROWS.len();
            let c = (rng() as usize) % COLS;
            if !specials.iter().any(|&(sr, sc, _)| sr == r && sc == c) {
                specials.push((r, c, kind));
                placed += 1;
            }
        }
    };
    let extra = (level.saturating_sub(1) as usize).min(3);
    place(rng, BrickKind::Wiki, 2 + extra.min(2));
    place(rng, BrickKind::Tag, 2 + extra / 2);
    place(rng, BrickKind::Italic, 4 + extra);

    let (bw, bh, gap) = (72.0, 24.0, 6.0);
    let x0 = (W - (COLS as f32 * (bw + gap) - gap)) / 2.0;
    let mut out = Vec::new();
    for r in 0..6 {
        for c in 0..COLS {
            let kind = match r {
                0 => BrickKind::Code,
                1 => BrickKind::Bold,
                _ => specials
                    .iter()
                    .find(|&&(sr, sc, _)| sr == r && sc == c)
                    .map(|&(.., k)| k)
                    .unwrap_or(BrickKind::Quote),
            };
            // A few holes in the quote field give each run its own shape.
            if kind == BrickKind::Quote && rng().is_multiple_of(8) {
                continue;
            }
            out.push(Brick {
                x: x0 + c as f32 * (bw + gap),
                y: 64.0 + r as f32 * (bh + gap),
                w: bw,
                h: bh,
                kind,
                hp: kind.hp(),
            });
        }
    }
    out
}

fn circle_hits_rect(cx: f32, cy: f32, r: f32, br: &Brick) -> bool {
    let nx = cx.clamp(br.x, br.x + br.w);
    let ny = cy.clamp(br.y, br.y + br.h);
    let (dx, dy) = (cx - nx, cy - ny);
    dx * dx + dy * dy <= r * r
}

pub fn render(app: &AppView, cx: &mut Context<AppView>) -> gpui::AnyElement {
    let Some(state) = app.game.as_ref() else {
        return div().size_full().into_any_element();
    };

    // Snapshot for the paint closure.
    let bricks: Vec<(f32, f32, f32, f32, BrickKind, u8)> = state
        .bricks
        .iter()
        .filter(|b| b.hp > 0)
        .map(|b| (b.x, b.y, b.w, b.h, b.kind, b.hp))
        .collect();
    let balls = state.balls.clone();
    let (paddle_x, paddle_w) = (state.paddle_x, state.paddle_w());
    let (score, lives, phase, level) = (state.score, state.lives, state.phase, state.level);
    let (slow, portal) = (state.slow > 0.0, state.portal > 0.0);
    let bounds_cell = state.bounds.clone();
    let accent = theme::accent();
    let colors = |k: BrickKind| -> (gpui::Hsla, gpui::Hsla) {
        // (fill, glyph)
        match k {
            BrickKind::Quote => (theme::glass(), theme::text_tertiary()),
            BrickKind::Tag => (theme::accent_tint(), theme::accent()),
            BrickKind::Italic => (theme::hover(), theme::text_secondary()),
            BrickKind::Wiki => (theme::accent(), theme::bg_content()),
            BrickKind::Bold => (theme::text_secondary(), theme::bg_content()),
            BrickKind::Code => (theme::elevated(), theme::text_primary()),
        }
    };
    let brick_data: Vec<_> = bricks
        .iter()
        .map(|&(x, y, w, h, k, hp)| {
            let (fill_c, glyph_c) = colors(k);
            (x, y, w, h, k.glyph(), fill_c, glyph_c, hp)
        })
        .collect();
    let (panel_bg, border, text_dim, text_main) = (
        theme::bg_sidebar(),
        theme::border_subtle(),
        theme::text_tertiary(),
        theme::text_primary(),
    );

    div()
        .id("blockdown")
        .size_full()
        .relative()
        .bg(theme::bg_content())
        .track_focus(&state.focus)
        .on_key_down(
            cx.listener(|this: &mut AppView, ev: &KeyDownEvent, window, cx| {
                let Some(g) = this.game.as_mut() else { return };
                match ev.keystroke.key.as_str() {
                    "left" => g.left_held = true,
                    "right" => g.right_held = true,
                    "space" | "up" => g.launch(),
                    "p" => g.toggle_pause(),
                    "escape" => {
                        this.close_game(window, cx);
                        return;
                    }
                    _ => return,
                }
                cx.notify();
            }),
        )
        .on_key_up(cx.listener(|this: &mut AppView, ev: &KeyUpEvent, _w, cx| {
            let Some(g) = this.game.as_mut() else { return };
            match ev.keystroke.key.as_str() {
                "left" => g.left_held = false,
                "right" => g.right_held = false,
                _ => return,
            }
            cx.notify();
        }))
        .on_mouse_move(
            cx.listener(|this: &mut AppView, ev: &MouseMoveEvent, _w, cx| {
                if let Some(g) = this.game.as_mut() {
                    let (wx, _) = g.to_world(ev.position);
                    g.set_paddle_from_mouse(wx);
                    cx.notify();
                }
            }),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this: &mut AppView, _: &MouseDownEvent, window, cx| {
                if let Some(g) = this.game.as_mut() {
                    window.focus(&g.focus, cx);
                    g.launch();
                    cx.notify();
                }
            }),
        )
        .child(
            canvas(
                move |bounds, _, _| bounds_cell.set(bounds),
                move |bounds, _, window, cx| {
                    let (bw, bh) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                    let scale = (bw / W).min(bh / H).max(0.01);
                    let ox = f32::from(bounds.origin.x) + (bw - W * scale) / 2.0;
                    let oy = f32::from(bounds.origin.y) + (bh - H * scale) / 2.0;
                    let pt = |x: f32, y: f32| point(px(ox + x * scale), px(oy + y * scale));
                    let rect = |x: f32, y: f32, w: f32, h: f32| {
                        Bounds::new(pt(x, y), size(px(w * scale), px(h * scale)))
                    };
                    // The board panel.
                    let mut q = fill(rect(0.0, 0.0, W, H), panel_bg);
                    q.corner_radii = gpui::Corners::all(px(10.0 * scale));
                    q.border_widths = gpui::Edges::all(px(1.0));
                    q.border_color = if portal { accent } else { border };
                    window.paint_quad(q);
                    // Bricks + glyphs.
                    for &(x, y, w, h, glyph, fill_c, glyph_c, hp) in &brick_data {
                        let mut q = fill(rect(x, y, w, h), fill_c);
                        q.corner_radii = gpui::Corners::all(px(4.0 * scale));
                        // Damaged armor shows a border crack.
                        if hp > 1 {
                            q.border_widths = gpui::Edges::all(px(1.5));
                            q.border_color = glyph_c;
                        }
                        window.paint_quad(q);
                        let font_size = px((12.0 * scale).max(6.0));
                        let run = TextRun {
                            len: glyph.len(),
                            font: window.text_style().font(),
                            color: glyph_c,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let shaped = window.text_system().shape_line(
                            SharedString::from(glyph),
                            font_size,
                            &[run],
                            None,
                        );
                        let p = pt(x + w / 2.0, y + h / 2.0 - 8.0);
                        let _ = shaped.paint(
                            point(p.x - shaped.width() / 2.0, p.y),
                            px(16.0 * scale),
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                    // Paddle (the caret) — blinks subtly via slow-mo tint.
                    let mut q = fill(
                        rect(paddle_x - paddle_w / 2.0, PADDLE_Y, paddle_w, PADDLE_H),
                        if slow { accent } else { text_main },
                    );
                    q.corner_radii = gpui::Corners::all(px(6.0 * scale));
                    window.paint_quad(q);
                    // Balls (bullet points).
                    for b in &balls {
                        let r = BALL_R * scale;
                        let mut q = fill(
                            Bounds::new(
                                point(
                                    px(ox + (b.x - BALL_R) * scale),
                                    px(oy + (b.y - BALL_R) * scale),
                                ),
                                size(px(r * 2.0), px(r * 2.0)),
                            ),
                            accent,
                        );
                        q.corner_radii = gpui::Corners::all(px(r));
                        window.paint_quad(q);
                    }
                    // HUD.
                    let hud = format!("{score:>6}    lives {lives}    level {level}");
                    let run = TextRun {
                        len: hud.len(),
                        font: window.text_style().font(),
                        color: text_dim,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped = window.text_system().shape_line(
                        SharedString::from(hud),
                        px(13.0 * scale),
                        &[run],
                        None,
                    );
                    let _ = shaped.paint(
                        pt(16.0, 18.0),
                        px(18.0 * scale),
                        gpui::TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                    // Phase banner.
                    let msg = match phase {
                        Phase::Ready => Some("← → move · Space launches · P pauses · Esc leaves"),
                        Phase::Paused => Some("paused — P or Space resumes"),
                        Phase::Over => Some("game over — Space to try again"),
                        Phase::Won => Some("level cleared — Space for the next (+1 life)"),
                        Phase::Running => None,
                    };
                    if let Some(msg) = msg {
                        let run = TextRun {
                            len: msg.len(),
                            font: window.text_style().font(),
                            color: text_dim,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let shaped = window.text_system().shape_line(
                            SharedString::from(msg),
                            px(14.0 * scale),
                            &[run],
                            None,
                        );
                        let p = pt(W / 2.0, H * 0.62);
                        let _ = shaped.paint(
                            point(p.x - shaped.width() / 2.0, p.y),
                            px(20.0 * scale),
                            gpui::TextAlign::Left,
                            None,
                            window,
                            cx,
                        );
                    }
                },
            )
            .absolute()
            .size_full(),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_rect_overlap() {
        let br = Brick {
            x: 100.0,
            y: 100.0,
            w: 72.0,
            h: 24.0,
            kind: BrickKind::Quote,
            hp: 1,
        };
        assert!(circle_hits_rect(136.0, 112.0, 7.0, &br)); // center
        assert!(circle_hits_rect(95.0, 112.0, 7.0, &br)); // left edge graze
        assert!(!circle_hits_rect(80.0, 112.0, 7.0, &br)); // clear miss
        assert!(!circle_hits_rect(136.0, 140.0, 7.0, &br)); // below
    }

    #[test]
    fn levels_are_armored_varied_and_in_bounds() {
        let mut seed = 12345u32;
        let level = build_level(&mut || this_rng(&mut seed), 1);
        // The two armored rows always survive intact.
        assert!(
            level
                .iter()
                .take(10)
                .all(|b| b.kind == BrickKind::Code && b.hp == 3)
        );
        assert!(level.iter().skip(10).take(10).all(|b| b.hp == 2)); // bold row
        assert!(level.iter().all(|b| b.x >= 0.0 && b.x + b.w <= W));
        // Specials are scattered accents, not walls of them.
        let count = |k: BrickKind| level.iter().filter(|b| b.kind == k).count();
        assert_eq!(count(BrickKind::Wiki), 2);
        assert_eq!(count(BrickKind::Tag), 2);
        assert_eq!(count(BrickKind::Italic), 4);
        // Different seeds lay out differently (the per-run variety).
        let mut seed_b = 99999u32;
        let level_b = build_level(&mut || this_rng(&mut seed_b), 1);
        let sig = |l: &[Brick]| {
            l.iter()
                .map(|b| (b.kind.glyph(), b.x as i32, b.y as i32))
                .collect::<Vec<_>>()
        };
        assert_ne!(sig(&level), sig(&level_b));
        // Higher levels sprinkle more specials.
        let mut seed_c = 12345u32;
        let l4 = build_level(&mut || this_rng(&mut seed_c), 4);
        let c4 = |k: BrickKind| l4.iter().filter(|b| b.kind == k).count();
        assert!(c4(BrickKind::Wiki) > count(BrickKind::Wiki));
        assert!(c4(BrickKind::Italic) > count(BrickKind::Italic));
    }
}
