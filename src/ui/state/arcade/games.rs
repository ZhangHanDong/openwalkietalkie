//! Bounded, deterministic 50 Hz game simulation. No heap, clocks or hardware.
use crate::ui::layout::{self, Rect};
pub const W: i16 = 480;
pub const H: i16 = 480;
pub const BIRD_X: i16 = 112;
pub const BIRD_RADIUS: i16 = 20;
pub const PIPE_WIDTH: i16 = 56;
pub const PIPE_GAP: i16 = 80;
const PIPE_SPACING: i16 = 240;
pub const PLAYER_Y: i16 = H - 74;
pub const PLAYER_MIN_X: i16 = 38;
pub const PLAYER_MAX_X: i16 = W - PLAYER_MIN_X;

pub fn random(seed: &mut u32) -> u32 {
    *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
    *seed
}

#[derive(Clone, Copy)]
pub struct Pipe {
    pub x: i16,
    pub gap: i16,
    pub scored: bool,
}
#[derive(Clone, Copy)]
pub struct Flappy {
    pub y: i32,
    pub velocity: i32,
    pub pipes: [Pipe; 3],
    pub score: u16,
}
impl Flappy {
    pub const fn new() -> Self {
        Self {
            y: H as i32 / 2 * 256,
            velocity: -1200,
            pipes: [
                Pipe {
                    x: W,
                    gap: H / 2,
                    scored: false,
                },
                Pipe {
                    x: W + PIPE_SPACING,
                    gap: H / 2 - 50,
                    scored: false,
                },
                Pipe {
                    x: W + 2 * PIPE_SPACING,
                    gap: H / 2 + 50,
                    scored: false,
                },
            ],
            score: 0,
        }
    }
    pub fn flap(&mut self) {
        self.velocity = -1400;
    }
    /// Alive, scored a point.
    pub fn step(&mut self, seed: &mut u32) -> (bool, bool) {
        self.velocity = (self.velocity + 98).min(2100);
        self.y += self.velocity;
        let y = (self.y / 256) as i16;
        let mut scored = false;
        let mut alive = (BIRD_RADIUS..H - BIRD_RADIUS).contains(&y);
        let mut furthest = self.pipes.iter().map(|p| p.x).max().unwrap_or(W);
        for p in &mut self.pipes {
            p.x -= 3;
            if p.x < -PIPE_WIDTH - 4 {
                p.x = furthest + PIPE_SPACING;
                furthest = p.x;
                p.gap = PIPE_GAP + 24 + (random(seed) % (H - 2 * (PIPE_GAP + 24)) as u32) as i16;
                p.scored = false;
            }
            if p.x - 4 < BIRD_X + BIRD_RADIUS
                && p.x + PIPE_WIDTH + 4 > BIRD_X - BIRD_RADIUS
                && (y - BIRD_RADIUS < p.gap - PIPE_GAP || y + BIRD_RADIUS > p.gap + PIPE_GAP)
            {
                alive = false;
            }
            if !p.scored && p.x + PIPE_WIDTH + 4 < BIRD_X - BIRD_RADIUS {
                p.scored = true;
                self.score = self.score.saturating_add(1);
                scored = true;
            }
        }
        (alive, scored)
    }
}

pub const CELL: i32 = 20;
pub const COLS: u8 = (W as i32 / CELL) as u8;
pub const ROWS: u8 = (H as i32 / CELL) as u8;
pub const CELLS: usize = COLS as usize * ROWS as usize;
/// Only fully visible cells can hold a snake or food. The excluded corner
/// tiles are drawn as walls, matching the actual display's rounded corners.
pub const fn playable(c: Cell) -> bool {
    c.x < COLS
        && c.y < ROWS
        && layout::in_safe_area(&Rect::new(
            c.x as i32 * CELL + 1,
            c.y as i32 * CELL + 1,
            CELL - 2,
            CELL - 2,
        ))
}
pub const PLAYABLE_CELLS: usize = {
    let mut count = 0;
    let mut i = 0;
    while i < CELLS {
        if playable(Cell {
            x: (i % COLS as usize) as u8,
            y: (i / COLS as usize) as u8,
        }) {
            count += 1;
        }
        i += 1;
    }
    count
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub x: u8,
    pub y: u8,
}
#[derive(Clone, Copy)]
pub struct Snake {
    pub body: [Cell; CELLS],
    pub len: usize,
    pub direction: u8,
    pub food: Cell,
    turns: [i8; 2],
    queued: usize,
    ticks: u8,
    pub won: bool,
}
impl Snake {
    pub const fn new() -> Self {
        let mut body = [Cell { x: 0, y: 0 }; CELLS];
        body[0] = Cell { x: 7, y: ROWS / 2 };
        body[1] = Cell { x: 6, y: ROWS / 2 };
        body[2] = Cell { x: 5, y: ROWS / 2 };
        body[3] = Cell { x: 4, y: ROWS / 2 };
        Self {
            body,
            len: 4,
            direction: 1,
            food: Cell { x: 15, y: ROWS / 2 },
            turns: [0; 2],
            queued: 0,
            ticks: 0,
            won: false,
        }
    }
    pub fn turn(&mut self, right: bool) {
        if self.queued < 2 {
            self.turns[self.queued] = if right { 1 } else { -1 };
            self.queued += 1;
        }
    }
    pub fn score(&self) -> u16 {
        self.len.saturating_sub(4) as u16
    }
    fn food(&mut self, seed: &mut u32) {
        if self.len == PLAYABLE_CELLS {
            self.won = true;
            return;
        }
        let mut nth = random(seed) as usize % (PLAYABLE_CELLS - self.len);
        for y in 0..ROWS {
            for x in 0..COLS {
                let c = Cell { x, y };
                if playable(c) && !self.body[..self.len].contains(&c) {
                    if nth == 0 {
                        self.food = c;
                        return;
                    }
                    nth -= 1;
                }
            }
        }
    }
    pub fn step(&mut self, seed: &mut u32) -> (bool, bool) {
        self.ticks += 1;
        if self.ticks < 8u8.saturating_sub((self.score() / 5).min(4) as u8) {
            return (true, false);
        }
        self.ticks = 0;
        if self.queued > 0 {
            self.direction = (self.direction as i8 + self.turns[0]).rem_euclid(4) as u8;
            self.turns[0] = self.turns[1];
            self.queued -= 1;
        }
        let head = self.body[0];
        let (dx, dy) = [(0, -1), (1, 0), (0, 1), (-1, 0)][self.direction as usize];
        let (x, y) = (i16::from(head.x) + dx, i16::from(head.y) + dy);
        if !(0..i16::from(COLS)).contains(&x) || !(0..i16::from(ROWS)).contains(&y) {
            return (false, false);
        }
        let next = Cell {
            x: x as u8,
            y: y as u8,
        };
        if !playable(next) {
            return (false, false);
        }
        let eating = next == self.food;
        // The old tail vacates its cell unless eating.
        if self.body[..self.len - usize::from(!eating)].contains(&next) {
            return (false, false);
        }
        if eating {
            self.len += 1;
        }
        self.body.copy_within(..self.len - 1, 1);
        self.body[0] = next;
        if eating {
            self.food(seed);
        }
        (!self.won, eating)
    }
}

#[derive(Clone, Copy)]
pub struct Sprite {
    pub x: i16,
    pub y: i16,
    pub active: bool,
}
impl Sprite {
    const EMPTY: Self = Self {
        x: 0,
        y: 0,
        active: false,
    };
}
#[derive(Clone, Copy)]
pub struct Raiden {
    pub x: i16,
    pub lives: u8,
    pub invulnerable: u8,
    pub score: u16,
    pub ticks: u32,
    pub shots: [Sprite; 16],
    pub enemies: [Sprite; 8],
    pub bullets: [Sprite; 12],
}
impl Raiden {
    pub const fn new() -> Self {
        Self {
            x: W / 2,
            lives: 3,
            invulnerable: 0,
            score: 0,
            ticks: 0,
            shots: [Sprite::EMPTY; 16],
            enemies: [Sprite::EMPTY; 8],
            bullets: [Sprite::EMPTY; 12],
        }
    }
    fn spawn(slots: &mut [Sprite], x: i16, y: i16) {
        if let Some(s) = slots.iter_mut().find(|s| !s.active) {
            *s = Sprite { x, y, active: true };
        }
    }
    pub fn step(&mut self, seed: &mut u32, left: bool, right: bool) -> (bool, bool, bool, bool) {
        self.ticks = self.ticks.wrapping_add(1);
        self.x =
            (self.x + (i16::from(right) - i16::from(left)) * 5).clamp(PLAYER_MIN_X, PLAYER_MAX_X);
        self.invulnerable = self.invulnerable.saturating_sub(1);
        let shot = self.ticks % 9 == 0;
        if shot {
            Self::spawn(&mut self.shots, self.x, PLAYER_Y - 32);
        }
        if self.ticks % (45 - u32::from((self.score / 100).min(20))) == 1 {
            Self::spawn(
                &mut self.enemies,
                28 + (random(seed) % (W - 56) as u32) as i16,
                -24,
            );
        }
        for s in &mut self.shots {
            if s.active {
                s.y -= 9;
                s.active = s.y >= 0;
            }
        }
        for e in &mut self.enemies {
            if e.active {
                e.y += 2;
                e.active = e.y < H + 24;
                if self.ticks % 65 == 0 {
                    Self::spawn(&mut self.bullets, e.x, e.y + 22);
                }
            }
        }
        for b in &mut self.bullets {
            if b.active {
                b.y += 5;
                b.active = b.y < H;
            }
        }
        let mut kill = false;
        for s in &mut self.shots {
            if s.active {
                for e in &mut self.enemies {
                    if e.active && (s.x - e.x).abs() < 24 && (s.y - e.y).abs() < 25 {
                        s.active = false;
                        e.active = false;
                        self.score = self.score.saturating_add(10);
                        kill = true;
                        break;
                    }
                }
            }
        }
        let mut hit = false;
        if self.invulnerable == 0 {
            for s in self.enemies.iter_mut().chain(self.bullets.iter_mut()) {
                if s.active && (s.x - self.x).abs() < 24 && (s.y - PLAYER_Y).abs() < 26 {
                    s.active = false;
                    hit = true;
                }
            }
            if hit {
                self.lives = self.lives.saturating_sub(1);
                self.invulnerable = 75;
            }
        }
        (self.lives > 0, kill, hit, shot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_panel_board_keeps_food_and_players_visible() {
        assert_eq!((W, H), (480, 480));
        for c in [
            Cell { x: COLS / 2, y: 0 },
            Cell {
                x: COLS / 2,
                y: ROWS - 1,
            },
            Cell { x: 0, y: ROWS / 2 },
            Cell {
                x: COLS - 1,
                y: ROWS / 2,
            },
        ] {
            assert!(playable(c), "all four flat edges are playable");
        }
        assert!(!playable(Cell { x: 0, y: 0 }));
        assert!(!playable(Cell {
            x: COLS - 1,
            y: ROWS - 1
        }));
        assert!(PLAYABLE_CELLS > CELLS * 9 / 10);
        let mut snake = Snake::new();
        let mut seed = 1;
        for _ in 0..2048 {
            snake.food(&mut seed);
            assert!(playable(snake.food));
            assert!(!snake.body[..snake.len].contains(&snake.food));
        }
        // The entire enlarged player sprite, including its flame, stays visible.
        for x in [PLAYER_MIN_X, PLAYER_MAX_X] {
            assert!(layout::in_safe_area(&Rect::new(
                i32::from(x) - 26,
                i32::from(PLAYER_Y) - 24,
                52,
                62,
            )));
        }
    }
    #[test]
    fn snake_corner_wall_collides_and_last_visible_cell_wins() {
        let mut g = Snake::new();
        g.body[0] = Cell { x: 4, y: 0 };
        g.len = 1;
        g.direction = 3;
        g.ticks = 7;
        assert!(!g.step(&mut 1).0);
        // Food selection must still find the sole remaining visible cell.
        let last = Cell {
            x: COLS - 1,
            y: ROWS / 2,
        };
        let mut len = 0;
        for y in 0..ROWS {
            for x in 0..COLS {
                let c = Cell { x, y };
                if playable(c) && c != last {
                    g.body[len] = c;
                    len += 1;
                }
            }
        }
        g.len = len;
        assert_eq!(g.len, PLAYABLE_CELLS - 1);
        g.food(&mut 1);
        assert_eq!(g.food, last);
        // Put the head next to the food; all other cells remain occupied.
        let head = g.body[..g.len]
            .iter()
            .position(|c| {
                *c == Cell {
                    x: last.x - 1,
                    y: last.y,
                }
            })
            .unwrap();
        g.body.swap(0, head);
        g.direction = 1;
        g.ticks = 7;
        assert_eq!(g.step(&mut 1), (false, true));
        assert!(g.won);
        assert_eq!(g.len, PLAYABLE_CELLS);
    }
    #[test]
    fn bird_flap_and_pipe_collision() {
        let mut g = Flappy::new();
        g.velocity = 1000;
        g.flap();
        assert!(g.velocity < 0);
        g.pipes[0].x = BIRD_X;
        g.y = 10 * 256;
        assert!(!g.step(&mut 1).0);
        g = Flappy::new();
        g.pipes[0].x = BIRD_X - BIRD_RADIUS - PIPE_WIDTH - 4;
        g.y = H as i32 / 2 * 256;
        g.velocity = 0;
        assert!(g.step(&mut 1).1);
        assert!(!g.step(&mut 1).1);
        assert_eq!(g.score, 1);
    }
    #[test]
    fn snake_turns_queue_and_eats() {
        let mut g = Snake::new();
        g.turn(true);
        g.turn(true);
        g.turn(true);
        for _ in 0..8 {
            assert!(g.step(&mut 1).0);
        }
        assert_eq!(g.direction, 2);
        for _ in 0..8 {
            assert!(g.step(&mut 1).0);
        }
        assert_eq!(g.direction, 3);
        g = Snake::new();
        g.food = Cell { x: 8, y: ROWS / 2 };
        for _ in 0..8 {
            g.step(&mut 2);
        }
        assert_eq!(g.len, 5);
        assert!(!g.body[..g.len].contains(&g.food));
    }
    #[test]
    fn snake_tail_vacates_and_full_board_wins() {
        let mut g = Snake::new();
        g.body[..4].copy_from_slice(&[
            Cell { x: 6, y: 6 },
            Cell { x: 6, y: 7 },
            Cell { x: 7, y: 7 },
            Cell { x: 7, y: 6 },
        ]);
        g.ticks = 7;
        assert!(g.step(&mut 1).0);
        g.len = PLAYABLE_CELLS;
        g.food(&mut 1);
        assert!(g.won);
    }
    #[test]
    fn shooter_bounds_hits_and_invulnerability() {
        let mut g = Raiden::new();
        for _ in 0..200 {
            g.step(&mut 1, true, false);
        }
        assert_eq!(g.x, PLAYER_MIN_X);
        g = Raiden::new();
        g.enemies[0] = Sprite {
            x: 100,
            y: 100,
            active: true,
        };
        g.shots[0] = Sprite {
            x: 100,
            y: 109,
            active: true,
        };
        assert!(g.step(&mut 1, false, false).1);
        assert_eq!(g.score, 10);
        g.bullets[0] = Sprite {
            x: g.x,
            y: PLAYER_Y - 5,
            active: true,
        };
        assert!(g.step(&mut 1, false, false).2);
        assert_eq!(g.lives, 2);
        g.bullets[0] = Sprite {
            x: g.x,
            y: PLAYER_Y - 5,
            active: true,
        };
        assert!(!g.step(&mut 1, false, false).2);
        assert_eq!(g.lives, 2);
    }
}
