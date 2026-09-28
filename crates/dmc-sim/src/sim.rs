//! The tick. Order within one step (fixed, and part of the parity contract):
//!
//! 1. record input, update lock-on, Devil Trigger toggle
//! 2. player control (start/chain/cancel moves, movement)
//! 3. enemy brains
//! 4. physics for every actor not frozen by hit-stop (root motion, gravity,
//!    integration, then either room collision (walls, ground, steps) or the
//!    flat graybox floor and arena bounds)
//! 5. hit resolution on this tick's active frames
//! 6. timers: move frames advance, hit-stun and hit-stop count down
//! 7. meters (style decay, DT drain/heal)

use crate::DT;
use crate::actor::{Actor, AttackState, State, Team};
use crate::ai::{self, Brain, Intent};
use crate::input::{Dir, InputBuffer, InputFrame, button};
use crate::math::{V3, point_segment_distance};
use crate::meters::{DevilTrigger, StyleMeter};
use crate::moves::{CancelInto, HitWindow, MoveDef, MoveSet, Stance, builtin};
use crate::rules::Rules;
use crate::world::{Body, World};
use serde::Serialize;

pub const PLAYER: usize = 0;
const LOCK_RANGE: f32 = 25.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Event {
    MoveStarted {
        actor: usize,
        move_id: String,
    },
    Hit {
        attacker: usize,
        target: usize,
        move_id: String,
        damage: f32,
    },
    Launched {
        target: usize,
    },
    Jumped {
        actor: usize,
    },
    Landed {
        actor: usize,
    },
    Died {
        actor: usize,
    },
    DevilTrigger {
        active: bool,
    },
    StyleRank {
        rank: Option<&'static str>,
    },
}

#[derive(Debug, Clone)]
pub struct Sim {
    pub tick: u64,
    pub rules: Rules,
    pub movesets: Vec<MoveSet>,
    pub actors: Vec<Actor>,
    pub input: InputBuffer,
    pub style: StyleMeter,
    pub dt: DevilTrigger,
    pub lock_target: Option<usize>,
    /// Room collision. `None` means the graybox: a flat floor at y = 0 inside
    /// `rules.arena_half_extent`.
    pub world: Option<World>,
    rng: u64,
    next_instance: u32,
    /// Ticks the player has spent frozen since last acting (extends the buffer).
    freeze_credit: usize,
    events: Vec<Event>,
}

struct PendingHit {
    attacker: usize,
    target: usize,
    window: usize,
    hit: HitWindow,
    move_id: String,
}

impl Sim {
    /// Player (sword move set) at the origin, no enemies.
    pub fn new(rules: Rules, seed: u64) -> Self {
        let movesets = vec![
            MoveSet::from_ron(builtin::PLAYER_SWORD).expect("built-in player move set"),
            MoveSet::from_ron(builtin::TRAINING_DUMMY).expect("built-in dummy move set"),
        ];
        Self {
            tick: 0,
            rules,
            movesets,
            actors: vec![Actor::new(Team::Player, 0, V3::ZERO, 1000.0)],
            input: InputBuffer::default(),
            style: StyleMeter::default(),
            dt: DevilTrigger::default(),
            lock_target: None,
            world: None,
            rng: seed | 1,
            next_instance: 0,
            freeze_credit: 0,
            events: Vec::new(),
        }
    }

    /// Use room collision instead of the graybox floor. The world is static,
    /// so it is not part of [`Sim::state_hash`].
    pub fn with_world(mut self, world: World) -> Self {
        self.world = Some(world);
        self
    }

    /// A training room: the player, plus one dummy two units ahead (inside
    /// sword reach).
    pub fn training_room(rules: Rules, brain: Brain) -> Self {
        let mut sim = Sim::new(rules, 0x0DD_C0FFEE);
        sim.spawn_enemy(V3::new(0.0, 0.0, 2.0), brain);
        sim
    }

    pub fn spawn_enemy(&mut self, pos: V3, brain: Brain) -> usize {
        let mut a = Actor::new(Team::Enemy, 1, pos, 400.0);
        a.facing = (self.actors[PLAYER].pos - pos)
            .flat()
            .normalize_or(-V3::FORWARD);
        a.brain = Some(brain);
        self.actors.push(a);
        self.actors.len() - 1
    }

    pub fn player(&self) -> &Actor {
        &self.actors[PLAYER]
    }

    pub fn move_def(&self, actor: usize, index: usize) -> &MoveDef {
        &self.movesets[self.actors[actor].moveset].moves[index]
    }

    /// Id of the move `actor` is performing, if any.
    pub fn current_move(&self, actor: usize) -> Option<(&str, u16)> {
        let a = self.actors[actor].attack()?;
        Some((self.move_def(actor, a.move_index).id.as_str(), a.frame))
    }

    fn next_rng(&mut self) -> u32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        (self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }

    pub fn step(&mut self, input: InputFrame) -> &[Event] {
        self.events.clear();
        self.input.push(self.tick, input);
        self.update_lock_on(input);
        if self
            .input
            .take_press(button::DEVIL_TRIGGER, self.rules.input_buffer_ticks)
            && self.dt.toggle()
        {
            self.events.push(Event::DevilTrigger {
                active: self.dt.active,
            });
        }

        let frozen: Vec<bool> = self.actors.iter().map(|a| a.hitstop > 0).collect();
        if frozen[PLAYER] {
            self.freeze_credit += 1;
        } else {
            self.control_player(input);
            self.freeze_credit = 0;
        }
        for (i, &f) in frozen.iter().enumerate().skip(1) {
            if !f {
                self.control_enemy(i);
            }
        }
        for (i, &f) in frozen.iter().enumerate() {
            if !f {
                self.physics(i);
            }
        }
        self.resolve_hits(&frozen);
        for (i, &was_frozen) in frozen.iter().enumerate() {
            self.timers(i, was_frozen);
        }

        let rank = self.style.rank();
        self.style.tick(self.rules.style_decay_per_tick);
        if self.style.rank() != rank {
            self.events.push(Event::StyleRank {
                rank: self.style.rank(),
            });
        }
        if self.dt.tick(self.rules.dt_drain_per_tick) {
            self.events.push(Event::DevilTrigger { active: false });
        }
        if self.dt.active {
            let p = &mut self.actors[PLAYER];
            p.health = (p.health + self.rules.dt_heal_per_tick).min(p.max_health);
        }
        self.tick += 1;
        &self.events
    }

    fn update_lock_on(&mut self, input: InputFrame) {
        if !input.held(button::LOCK_ON) {
            self.lock_target = None;
            return;
        }
        let p = self.actors[PLAYER].pos;
        let valid = |a: &Actor| {
            a.team == Team::Enemy && a.alive() && (a.pos - p).flat().length() <= LOCK_RANGE
        };
        if self.lock_target.is_some_and(|t| valid(&self.actors[t])) {
            return;
        }
        self.lock_target = (1..self.actors.len())
            .filter(|&i| valid(&self.actors[i]))
            .min_by(|&a, &b| {
                let da = (self.actors[a].pos - p).flat().length();
                let db = (self.actors[b].pos - p).flat().length();
                da.total_cmp(&db).then(a.cmp(&b))
            });
    }

    fn face_toward(&mut self, actor: usize, target: usize) {
        let d = (self.actors[target].pos - self.actors[actor].pos).flat();
        let f = self.actors[actor].facing;
        self.actors[actor].facing = d.normalize_or(f);
    }

    fn start_move(&mut self, actor: usize, index: usize) {
        let instance = self.next_instance;
        self.next_instance = self.next_instance.wrapping_add(1);
        let a = &mut self.actors[actor];
        if a.grounded {
            a.vel = V3::new(0.0, a.vel.y, 0.0);
        }
        a.state = State::Attack(AttackState {
            move_index: index,
            frame: 0,
            instance,
            queued: None,
            connected: Vec::new(),
        });
        let move_id = self.movesets[a.moveset].moves[index].id.clone();
        self.events.push(Event::MoveStarted { actor, move_id });
    }

    fn jump(&mut self, actor: usize) {
        let a = &mut self.actors[actor];
        a.vel.y = self.rules.jump_speed;
        a.grounded = false;
        a.state = State::Air;
        self.events.push(Event::Jumped { actor });
    }

    /// First move (command moves before plain ones) whose trigger is satisfied
    /// by a buffered press; the press is consumed. `only` restricts the search.
    fn triggered_move(&mut self, actor: usize, window: usize, only: Option<&str>) -> Option<usize> {
        let a = &self.actors[actor];
        let cur = self.input.current();
        let lock_held = cur.held(button::LOCK_ON);
        let dir = Dir::of(&cur, a.facing);
        let moves = &self.movesets[a.moveset].moves;
        let specific = |m: &MoveDef| {
            m.trigger
                .as_ref()
                .is_some_and(|t| t.lock_on || t.direction != Dir::Any)
        };
        let order = moves
            .iter()
            .enumerate()
            .filter(|(_, m)| specific(m))
            .chain(moves.iter().enumerate().filter(|(_, m)| !specific(m)));
        let mut found = None;
        for (i, m) in order {
            if only.is_some_and(|id| id != m.id) {
                continue;
            }
            let Some(t) = &m.trigger else { continue };
            let stance_ok = match t.stance {
                Stance::Ground => a.grounded,
                Stance::Air => !a.grounded,
                Stance::Either => true,
            };
            if !stance_ok || (t.lock_on && !lock_held) || !t.direction.accepts(dir) {
                continue;
            }
            if let Some(press) = self.input.pending_press(t.button.mask(), window) {
                found = Some((i, t.button.mask(), press));
                break;
            }
        }
        let (i, mask, press) = found?;
        self.input.consume(mask, press);
        Some(i)
    }

    fn control_player(&mut self, input: InputFrame) {
        let window = self.rules.input_buffer_ticks + self.freeze_credit;
        let lock = self.lock_target;
        let p = &self.actors[PLAYER];
        match p.state.clone() {
            State::Dead | State::HitStun { .. } => {}
            State::Locomotion | State::Air => {
                if let Some(t) = lock {
                    self.face_toward(PLAYER, t);
                }
                if let Some(i) = self.triggered_move(PLAYER, window, None) {
                    self.start_move(PLAYER, i);
                    return;
                }
                if self.actors[PLAYER].grounded && self.input.take_press(button::JUMP, window) {
                    self.jump(PLAYER);
                    return;
                }
                let desired = input.direction() * self.rules.run_speed;
                let p = &mut self.actors[PLAYER];
                if p.grounded {
                    p.vel = V3::new(desired.x, p.vel.y, desired.z);
                } else {
                    let k = self.rules.air_control * 0.2;
                    p.vel.x += (desired.x * self.rules.air_control - p.vel.x) * k;
                    p.vel.z += (desired.z * self.rules.air_control - p.vel.z) * k;
                }
                if lock.is_none() && desired.length() > 0.1 {
                    p.facing = desired.normalize_or(p.facing);
                }
            }
            State::Attack(st) => {
                let m = self.move_def(PLAYER, st.move_index).clone();
                let f = st.frame;
                if let Some(t) = lock
                    && f < m.track_frames
                {
                    self.face_toward(PLAYER, t);
                }
                let mut queued = st.queued;
                if let Some(c) = &m.combo {
                    if queued.is_none()
                        && (c.input.0..=c.input.1).contains(&f)
                        && self.input.take_press(c.button.mask(), window)
                    {
                        queued = self.movesets[self.actors[PLAYER].moveset].index(&c.next);
                    }
                    if let Some(q) = queued
                        && f >= c.link_frame
                    {
                        self.start_move(PLAYER, q);
                        return;
                    }
                }
                if let State::Attack(a) = &mut self.actors[PLAYER].state {
                    a.queued = queued;
                }
                for cw in m
                    .cancels
                    .iter()
                    .filter(|c| (c.frames.0..=c.frames.1).contains(&f))
                {
                    match &cw.into {
                        CancelInto::AnyMove => {
                            if let Some(i) = self.triggered_move(PLAYER, window, None) {
                                self.start_move(PLAYER, i);
                                return;
                            }
                        }
                        CancelInto::Move(id) => {
                            if let Some(i) = self.triggered_move(PLAYER, window, Some(id)) {
                                self.start_move(PLAYER, i);
                                return;
                            }
                        }
                        CancelInto::Jump => {
                            if self.actors[PLAYER].grounded
                                && self.input.take_press(button::JUMP, window)
                            {
                                self.jump(PLAYER);
                                return;
                            }
                        }
                        CancelInto::Movement => {
                            if input.direction().length() > 0.3 && queued.is_none() {
                                let p = &mut self.actors[PLAYER];
                                p.state = p.neutral_state();
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    fn control_enemy(&mut self, i: usize) {
        if !self.actors[i].alive() {
            return;
        }
        let to_player = (self.actors[PLAYER].pos - self.actors[i].pos).flat();
        let distance = to_player.length();
        if let Some(st) = self.actors[i].attack() {
            if st.frame < self.move_def(i, st.move_index).track_frames {
                self.face_toward(i, PLAYER);
            }
            return;
        }
        let can_act =
            matches!(self.actors[i].state, State::Locomotion) && self.actors[PLAYER].alive();
        let rng = self.next_rng();
        let Some(mut brain) = self.actors[i].brain.take() else {
            return;
        };
        let intent = ai::think(&mut brain, distance, can_act, rng);
        let walk = match &brain {
            Brain::Melee { params, .. } => params.walk_speed,
            Brain::Passive => 0.0,
        };
        self.actors[i].brain = Some(brain);
        match intent {
            Intent::Idle => {
                let a = &mut self.actors[i];
                if a.grounded && matches!(a.state, State::Locomotion) {
                    a.vel = V3::new(0.0, a.vel.y, 0.0);
                }
            }
            Intent::Walk { toward_player } => {
                self.face_toward(i, PLAYER);
                let a = &mut self.actors[i];
                let dir = if toward_player { a.facing } else { -a.facing };
                a.vel = V3::new(dir.x * walk, a.vel.y, dir.z * walk);
            }
            Intent::Attack(id) => {
                self.face_toward(i, PLAYER);
                if let Some(m) = self.movesets[self.actors[i].moveset].index(&id) {
                    self.start_move(i, m);
                }
            }
        }
    }

    fn physics(&mut self, i: usize) {
        let rules = &self.rules;
        let a = &mut self.actors[i];
        let mut gravity = rules.gravity;
        match &a.state {
            State::Attack(st) => {
                let m = &self.movesets[a.moveset].moves[st.move_index];
                gravity *= m.gravity_scale;
                let seg = m
                    .motion
                    .iter()
                    .find(|s| (s.frames.0..=s.frames.1).contains(&st.frame));
                match seg {
                    Some(s) => {
                        if s.forward != 0.0 || a.grounded {
                            a.vel =
                                V3::new(a.facing.x * s.forward, a.vel.y, a.facing.z * s.forward);
                        }
                        if let Some(v) = s.vertical
                            && st.frame == s.frames.0
                        {
                            a.vel.y = v;
                            a.grounded = a.grounded && v <= 0.0;
                        }
                    }
                    None if a.grounded => a.vel = V3::new(0.0, a.vel.y, 0.0),
                    None => {}
                }
                // Air attacks hold height rather than keep falling fast.
                if !a.grounded && m.gravity_scale < 1.0 && a.vel.y < 0.0 {
                    a.vel.y *= 0.8;
                }
            }
            State::HitStun { .. } => {
                if !a.grounded && a.team == Team::Enemy {
                    gravity = rules.juggle_gravity;
                }
                if a.grounded {
                    a.vel = V3::new(a.vel.x * 0.85, a.vel.y, a.vel.z * 0.85);
                }
            }
            State::Dead => {
                a.vel = V3::new(0.0, a.vel.y, 0.0);
            }
            State::Locomotion | State::Air => {}
        }
        if !a.grounded {
            a.vel.y -= gravity * DT;
        }
        a.pos += a.vel * DT;
        let ground = match &self.world {
            Some(w) => {
                let body = Body::PLAYER;
                a.pos = w.push_out(a.pos, body);
                if a.vel.y <= 0.0 {
                    // Look up far enough to catch this tick's whole fall, so a
                    // fast drop can't pass through a floor.
                    let above = body.step_up.max(-a.vel.y * DT);
                    let below = if a.grounded { body.snap_down } else { 0.0 };
                    w.ground(a.pos, above, below).map(|g| g.height)
                } else {
                    None
                }
            }
            None => (a.pos.y <= 0.0).then_some(0.0),
        };
        if let Some(h) = ground {
            a.pos.y = h;
            if !a.grounded {
                a.grounded = true;
                a.vel.y = 0.0;
                if matches!(a.state, State::Air) {
                    a.state = State::Locomotion;
                }
                self.events.push(Event::Landed { actor: i });
            }
        } else {
            a.grounded = false;
            if matches!(a.state, State::Locomotion) {
                a.state = State::Air;
            }
        }
        if self.world.is_none() {
            let e = rules.arena_half_extent;
            a.pos.x = a.pos.x.clamp(-e, e);
            a.pos.z = a.pos.z.clamp(-e, e);
        }
    }

    fn resolve_hits(&mut self, frozen: &[bool]) {
        let mut pending = Vec::new();
        for (att, a) in self.actors.iter().enumerate() {
            let (Some(st), false) = (a.attack(), frozen[att]) else {
                continue;
            };
            let m = &self.movesets[a.moveset].moves[st.move_index];
            for (wi, h) in m.hits.iter().enumerate() {
                if !(h.frames.0..=h.frames.1).contains(&st.frame) {
                    continue;
                }
                let centre = a.pos + V3::local_to_world(a.facing, h.offset);
                for (ti, t) in self.actors.iter().enumerate() {
                    if ti == att
                        || t.team == a.team
                        || !t.alive()
                        || st.connected.contains(&(ti, wi))
                    {
                        continue;
                    }
                    let bottom = t.pos + V3::UP * t.radius;
                    let top = t.pos + V3::UP * (t.height - t.radius).max(t.radius);
                    if point_segment_distance(centre, bottom, top) <= h.radius + t.radius {
                        pending.push(PendingHit {
                            attacker: att,
                            target: ti,
                            window: wi,
                            hit: h.clone(),
                            move_id: m.id.clone(),
                        });
                    }
                }
            }
        }
        for p in pending {
            self.apply_hit(p);
        }
    }

    fn apply_hit(&mut self, p: PendingHit) {
        let h = &p.hit;
        let mult = if p.attacker == PLAYER && self.dt.active {
            self.rules.dt_damage_multiplier
        } else {
            1.0
        };
        let damage = h.damage * mult;
        let facing = self.actors[p.attacker].facing;
        if let State::Attack(st) = &mut self.actors[p.attacker].state {
            st.connected.push((p.target, p.window));
        }
        self.actors[p.attacker].hitstop = self.actors[p.attacker].hitstop.max(h.hitstop);

        let t = &mut self.actors[p.target];
        t.health -= damage;
        t.hitstop = h.hitstop;
        t.vel = V3::new(facing.x * h.knockback, t.vel.y, facing.z * h.knockback);
        let was_grounded = t.grounded;
        if h.lift != 0.0 {
            t.vel.y = h.lift;
            if h.lift > 0.0 {
                t.grounded = false;
            }
        } else if !t.grounded {
            t.vel.y = t.vel.y.max(2.0);
        }
        let died = t.health <= 0.0;
        t.state = if died {
            State::Dead
        } else {
            State::HitStun { ticks: h.stun }
        };

        self.events.push(Event::Hit {
            attacker: p.attacker,
            target: p.target,
            move_id: p.move_id.clone(),
            damage,
        });
        if was_grounded && h.lift > 0.0 {
            self.events.push(Event::Launched { target: p.target });
        }
        if died {
            self.events.push(Event::Died { actor: p.target });
        }
        if p.attacker == PLAYER {
            self.style.on_hit(&p.move_id, h.style);
            self.dt.gain(h.dt_gain);
        }
        if p.target == PLAYER {
            self.style.on_damaged();
        }
    }

    fn timers(&mut self, i: usize, was_frozen: bool) {
        let a = &mut self.actors[i];
        if was_frozen {
            a.hitstop = a.hitstop.saturating_sub(1);
            return;
        }
        match &mut a.state {
            State::Attack(st) => {
                st.frame += 1;
                if st.frame >= self.movesets[a.moveset].moves[st.move_index].total_frames {
                    a.state = a.neutral_state();
                }
            }
            State::HitStun { ticks } => {
                *ticks = ticks.saturating_sub(1);
                if *ticks == 0 && a.grounded {
                    a.state = State::Locomotion;
                }
            }
            _ => {}
        }
    }

    /// FNV-1a over everything that defines the simulation state.
    pub fn state_hash(&self) -> u64 {
        let mut h: u64 = 0xCBF2_9CE4_8422_2325;
        let mut eat = |v: u64| {
            for b in v.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
            }
        };
        eat(self.tick);
        for a in &self.actors {
            for v in [a.pos, a.vel, a.facing] {
                v.bits().into_iter().for_each(|b| eat(b as u64));
            }
            eat(a.state.tag() as u64);
            match &a.state {
                State::Attack(st) => {
                    eat(st.move_index as u64);
                    eat(st.frame as u64);
                    eat(st.queued.map_or(u64::MAX, |q| q as u64));
                }
                State::HitStun { ticks } => eat(*ticks as u64),
                _ => {}
            }
            eat(a.health.to_bits() as u64);
            eat(a.hitstop as u64);
            eat(a.grounded as u64);
        }
        eat(self.style.points.to_bits() as u64);
        eat(self.dt.gauge.to_bits() as u64);
        eat(self.dt.active as u64);
        eat(self.lock_target.map_or(u64::MAX, |t| t as u64));
        eat(self.rng);
        h
    }
}
