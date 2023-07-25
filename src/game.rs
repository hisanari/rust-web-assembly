use std::collections::HashMap;
use anyhow::{anyhow, Result};
use serde::Deserialize;
use web_sys::HtmlImageElement;
use async_trait::async_trait;
use crate::{browser, engine};
use crate::engine::{Game, KeyState, Point, Rect, Renderer};
use self::red_hat_boy_states::*;

#[derive(Deserialize, Clone)]
struct SheetRect {
    x: i16,
    y: i16,
    w: i16,
    h: i16,
}

#[derive(Deserialize, Clone)]
struct Cell {
    frame: SheetRect,
}

#[derive(Deserialize, Clone)]
pub struct Sheet {
    frames: HashMap<String, Cell>,
}

pub enum WalkTheDog {
    Loading,
    Loaded(RedHatBoy),
}

impl WalkTheDog {
    pub fn new() -> Self {
        WalkTheDog::Loading
    }
}

#[async_trait(?Send)]
impl Game for WalkTheDog {
    async fn initialize(&self) -> Result<Box<dyn Game>> {
        match self {
            WalkTheDog::Loading => {
                let json = browser::fetch_json("rhb.json").await?;

                let rhb = RedHatBoy::new(
                    json.into_serde::<Sheet>()?,
                    engine::load_image("rhb.png").await?
                );

                Ok(Box::new(WalkTheDog::Loaded(rhb)))
            }
            WalkTheDog::Loaded(_) => Err(anyhow!("Error: Game is already initialized!")),
        }
    }
    fn update(&mut self, keystate: &KeyState) {
        if let WalkTheDog::Loaded(rhb) = self {
            if keystate.is_pressed("ArrowRight") {
                rhb.run_right();
            }

            if keystate.is_pressed("ArrowDown") {
                rhb.slide();
            }

            if keystate.is_pressed("Space") {
                rhb.jump();
            }

            rhb.update();
        }
    }
    fn draw(&self, renderer: &Renderer) {
        renderer.clear(&Rect {
            x: 0.0,
            y: 0.0,
            w: 600.0,
            h: 600.0,
        });

        if let WalkTheDog::Loaded(rhb) = self {
            rhb.draw(renderer);
        }
    }
}

mod red_hat_boy_states {
    use crate::engine::Point;
    use crate::game::RedHatBoyStateMachine;

    const FLOOR: i16 = 475;
    const IDLE_FRAME_NAME: &str = "Idle";
    const RUN_FRAME_NAME: &str = "Run";
    const SLIDING_FRAME_NAME: &str = "Slide";
    const JUMP_FRAME_NAME: &str = "Jump";
    const IDLE_FRAMES: u8 = 29;
    const RUNNING_FRAMES: u8 = 23;
    const SLIDING_FRAMES: u8 = 14;
    const JUMP_FRAMES: u8 = 35;
    const RUNNING_SPEED: i16 = 3;
    const JUMP_SPEED: i16 = -25;
    const GRAVITY: i16 = 1;

    #[derive(Copy, Clone)]
    pub struct RedHatBoyState<S> {
        context: RedHatBoyContext,
        _state: S,
    }

    impl<S> RedHatBoyState<S> {
        pub fn context(&self) -> &RedHatBoyContext {
            &self.context
        }
    }

    #[derive(Copy, Clone)]
    pub struct RedHatBoyContext {
        pub frame: u8,
        pub position: Point,
        pub velocity: Point,
    }

    #[derive(Copy, Clone)]
    pub struct Idle;

    #[derive(Copy, Clone)]
    pub struct Running;

    #[derive(Copy, Clone)]
    pub struct Sliding;

    #[derive(Copy, Clone)]
    pub struct Jumping;

    impl RedHatBoyState<Idle> {
        pub fn run(self) -> RedHatBoyState<Running> {
            RedHatBoyState {
                context: self.context.reset_frame().run_right(),
                _state: Running {},
            }
        }

        pub fn new() -> Self {
            RedHatBoyState {
                context: RedHatBoyContext {
                    frame: 0,
                    position: Point { x: 0, y: FLOOR },
                    velocity: Point { x: 0, y: 0 },
                },
                _state: Idle {},
            }
        }

        pub fn frame_name(&self) -> &str {
            IDLE_FRAME_NAME
        }

        pub fn update(mut self) -> Self {
            self.context = self.context.update(IDLE_FRAMES);
            self
        }
    }

    impl RedHatBoyState<Running> {
        pub fn frame_name(&self) -> &str {
            RUN_FRAME_NAME
        }

        pub fn update(mut self) -> Self {
            self.context = self.context.update(RUNNING_FRAMES);
            self
        }

        pub fn slide(self) -> RedHatBoyState<Sliding> {
            RedHatBoyState {
                context: self.context.reset_frame(),
                _state: Sliding {},
            }
        }

        pub fn jump(self) -> RedHatBoyState<Jumping> {
            RedHatBoyState {
                context: self.context.set_vertical_velocity(JUMP_SPEED).reset_frame(),
                _state: Jumping {},
            }
        }
    }

    impl RedHatBoyState<Sliding> {
        pub fn frame_name(&self) -> &str {
            SLIDING_FRAME_NAME
        }

        pub fn update(mut self) -> SlidingEndState {
            self.context = self.context.update(SLIDING_FRAMES);

            if self.context.frame >= SLIDING_FRAMES {
                SlidingEndState::Complete(self.stand())
            } else {
                SlidingEndState::Sliding(self)
            }
        }

        pub fn stand(self) -> RedHatBoyState<Running> {
            RedHatBoyState {
                context: self.context.reset_frame(),
                _state: Running,
            }
        }
    }

    impl RedHatBoyState<Jumping> {
        pub fn frame_name(&self) -> &str {
            JUMP_FRAME_NAME
        }

        pub fn update(mut self) -> JumpingEndState {
            self.context = self.context.update(JUMP_FRAMES);
            if self.context.position.y >= FLOOR {
                JumpingEndState::Complete(self.land())
            } else {
                JumpingEndState::Jumping(self)
            }
        }

        pub fn land(self) -> RedHatBoyState<Running> {
            RedHatBoyState {
                context: self.context.reset_frame(),
                _state: Running,
            }
        }
    }

    pub enum SlidingEndState {
        Complete(RedHatBoyState<Running>),
        Sliding(RedHatBoyState<Sliding>)
    }

    impl From<SlidingEndState> for RedHatBoyStateMachine {
        fn from(end_state: SlidingEndState) -> Self {
            match end_state {
                SlidingEndState::Complete(running_state) => running_state.into(),
                SlidingEndState::Sliding(sliding_state) => sliding_state.into(),
            }
        }
    }

    pub enum JumpingEndState {
        Complete(RedHatBoyState<Running>),
        Jumping(RedHatBoyState<Jumping>),
    }

    impl From<JumpingEndState> for RedHatBoyStateMachine {
        fn from(end_state: JumpingEndState) -> Self {
            match end_state {
                JumpingEndState::Complete(running_state) => running_state.into(),
                JumpingEndState::Jumping(jumping_state) => jumping_state.into(),
            }
        }
    }

    impl RedHatBoyContext {
        pub fn update(mut self, frame_count: u8) -> Self {
            self.velocity.y += GRAVITY;
            if self.frame < frame_count {
                self.frame += 1;
            } else {
                self.frame = 0;
            }
            self.position.x += self.velocity.x;
            self.position.y += self.velocity.y;
            if self.position.y > FLOOR {
                self.position.y = FLOOR;
            }
            self
        }

        fn reset_frame(mut self) -> Self {
            self.frame = 0;
            self
        }

        fn run_right(mut self) -> Self {
            self.velocity.x += RUNNING_SPEED;
            self
        }

        fn set_vertical_velocity(mut self, y: i16) -> Self {
            self.velocity.y = y;
            self
        }
    }
}

#[derive(Copy, Clone)]
enum RedHatBoyStateMachine {
    Idle(RedHatBoyState<Idle>),
    Running(RedHatBoyState<Running>),
    Sliding(RedHatBoyState<Sliding>),
    Jump(RedHatBoyState<Jumping>),
}

impl From<RedHatBoyState<Running>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Running>) -> Self {
        RedHatBoyStateMachine::Running(state)
    }
}

impl From<RedHatBoyState<Sliding>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Sliding>) -> Self {
        RedHatBoyStateMachine::Sliding(state)
    }
}

impl From<RedHatBoyState<Idle>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Idle>) -> Self {
        RedHatBoyStateMachine::Idle(state)
    }
}

impl From<RedHatBoyState<Jumping>> for RedHatBoyStateMachine {
    fn from(state: RedHatBoyState<Jumping>) -> Self {
        RedHatBoyStateMachine::Jump(state)
    }
}

impl RedHatBoyStateMachine {
    fn transition(self, event: Event) -> Self {
        match (self, event) {
            (RedHatBoyStateMachine::Idle(state), Event::Run) => state.run().into(),
            (RedHatBoyStateMachine::Idle(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::Running(state), Event::Slide) => state.slide().into(),
            (RedHatBoyStateMachine::Running(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::Running(state), Event::Jump) => state.jump().into(),
            (RedHatBoyStateMachine::Sliding(state), Event::Update) => state.update().into(),
            (RedHatBoyStateMachine::Jump(state), Event::Jump) => state.update().into(),
            (RedHatBoyStateMachine::Jump(state), Event::Update) => state.update().into(),
            _ => self,
        }
    }

    fn frame_name(&self) -> &str {
        match self {
            RedHatBoyStateMachine::Idle(state) => state.frame_name(),
            RedHatBoyStateMachine::Running(state) => state.frame_name(),
            RedHatBoyStateMachine::Sliding(state) => state.frame_name(),
            RedHatBoyStateMachine::Jump(state) => state.frame_name(),
        }
    }

    fn context(&self) ->&RedHatBoyContext {
        match self {
            RedHatBoyStateMachine::Idle(state) => &state.context(),
            RedHatBoyStateMachine::Running(state) => &state.context(),
            RedHatBoyStateMachine::Sliding(state) => &state.context(),
            RedHatBoyStateMachine::Jump(state) => &state.context(),
        }
    }

    fn update(self) -> Self {
        self.transition(Event::Update)
    }
}

pub enum Event {
    Run,
    Slide,
    Update,
    Jump,
}

struct RedHatBoy {
    state_machine: RedHatBoyStateMachine,
    sprite_sheet: Sheet,
    image: HtmlImageElement,
}

impl RedHatBoy {
    fn new(sheet: Sheet, image: HtmlImageElement) -> Self {
        RedHatBoy {
            state_machine: RedHatBoyStateMachine::Idle(RedHatBoyState::new()),
            sprite_sheet: sheet,
            image,
        }
    }

    fn draw(&self, renderer: &Renderer) {
        let frame_name = format!(
            "{} ({}).png",
            self.state_machine.frame_name(),
            (self.state_machine.context().frame / 3) + 1,
        );

        let sprite = self
            .sprite_sheet
            .frames
            .get(&frame_name)
            .expect("Cell not found");

        renderer.draw_image(
            &self.image,
            &Rect {
                x: sprite.frame.x.into(),
                y: sprite.frame.y.into(),
                w: sprite.frame.w.into(),
                h: sprite.frame.h.into(),
            },
            &Rect {
                x: self.state_machine.context().position.x.into(),
                y: self.state_machine.context().position.y.into(),
                w: sprite.frame.w.into(),
                h: sprite.frame.h.into(),
            }
        )
    }

    fn update(&mut self) {
        self.state_machine = self.state_machine.update();
    }

    fn run_right(&mut self) {
        self.state_machine = self.state_machine.transition(Event::Run);
    }

    fn slide(&mut self) {
        self.state_machine = self.state_machine.transition(Event::Slide);
    }

    fn jump(&mut self) {
        self.state_machine = self.state_machine.transition(Event::Jump);
    }
}

