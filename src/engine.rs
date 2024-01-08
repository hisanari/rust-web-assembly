use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Mutex;
use futures::channel::oneshot::channel;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{AudioBuffer, AudioContext, CanvasRenderingContext2d, HtmlImageElement};
use crate::{browser, sound};
use anyhow::{anyhow, Result};
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use async_trait::async_trait;
use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct SheetRect {
    pub x: i16,
    pub y: i16,
    pub w: i16,
    pub h: i16,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    pub frame: SheetRect,
    pub sprite_source_size: SheetRect,
}

#[derive(Deserialize, Clone)]
pub struct Sheet {
    pub frames: HashMap<String, Cell>,
}

pub async fn load_image(source: &str) -> Result<HtmlImageElement> {
    let image = browser::new_image()?;

    let (complete_tx, complete_rx) = channel::<Result<()>>();
    let success_tx = Rc::new(Mutex::new(Some(complete_tx)));
    let error_tx = Rc::clone(&success_tx);
    let success_callback = browser::closure_once(move || {
        if let Some(success_tx) =
            success_tx
                .lock()
                .ok()
                .and_then(|mut opt| opt.take()) {
            let _ = success_tx.send(Ok(()));
        }
    });

    let error_callback: Closure<dyn FnMut(JsValue)> =
        browser::closure_once(move |err| {
            if let Some(error_tx) = error_tx.lock().ok()
                .and_then(|mut opt| opt.take()) {
                let _ = error_tx.send(Err(anyhow!("Error Loading Image: {:#?}", err)));
            }
        });

    image.set_onload(Some(success_callback.as_ref().unchecked_ref()));
    image.set_onerror(Some(error_callback.as_ref().unchecked_ref()));
    image.set_src(source);

    complete_rx.await??;

    Ok(image)
}

const FRAME_SIZE: f32 = 1.0 / 60.0 * 1000.0;
pub struct GameLoop {
    last_frame: f64,
    accumulated_delta: f32,
}

pub type LoopClosure = Closure<dyn FnMut(f64)>;
type SharedLoopClosure = Rc<RefCell<Option<LoopClosure>>>;

impl GameLoop {
    pub async fn start(game: impl Game + 'static) -> Result<()> {
        let mut keyevent_receiver = prepare_input()?;
        let mut game = game.initialize().await?;
        let mut game_loop = GameLoop {
            last_frame: browser::now()?,
            accumulated_delta: 0.0,
        };

        let renderer = Renderer {
            context: browser::context()?,
        };

        let  f: SharedLoopClosure = Rc::new(RefCell::new(None));
        let g = f.clone();

        let mut keystate = KeyState::new();
        *g.borrow_mut() = Some(browser::create_raf_closure(move |perf: f64| {
            process_input(&mut keystate, &mut keyevent_receiver);
            game_loop.accumulated_delta += (perf - game_loop.last_frame) as f32;
            while game_loop.accumulated_delta > FRAME_SIZE {
                game.update(&keystate);
                game_loop.accumulated_delta -= FRAME_SIZE;
            }
            game_loop.last_frame = perf;

            game.draw(&renderer);
            let _ = browser::request_animation_frame(f.borrow().as_ref().unwrap());
        }));

        browser::request_animation_frame(
            g.borrow()
                .as_ref()
                .ok_or_else(|| anyhow!("GameLoop: Loop is None"))?,
        )?;

        Ok(())
    }
}

#[async_trait(?Send)]
pub trait Game {
    async fn initialize(&self) -> Result<Box<dyn Game>>;
    fn update(&mut self, keystate: &KeyState);
    fn draw(&self, context: &Renderer);
}

pub struct Renderer {
    context: CanvasRenderingContext2d,
}

#[derive(Default)]
pub struct Rect {
    pub position: Point,
    pub w: i16,
    pub h: i16,
}

impl Rect {
    pub const fn new(position: Point, w: i16, h: i16) -> Self {
        Rect {
            position,
            w,
            h,
        }
    }

    pub const fn new_from_x_y(x: i16, y: i16, w: i16, h: i16) -> Self {
        Rect::new(Point { x, y }, w, h)
    }

    pub fn intersects(&self, rect: &Rect) -> bool {
        self.x() < (rect.x() + rect.w) && self.x() + self.w > rect.x()
            && self.y() < rect.bottom() && self.bottom() > rect.y()
    }

    pub fn right(&self) -> i16 {
        self.x() + self.w
    }

    pub fn bottom(&self) -> i16 {
        self.y() + self.h
    }

    pub fn x(&self) -> i16 {
        self.position.x
    }

    pub fn y(&self) -> i16 {
        self.position.y
    }

    pub fn set_x(&mut self, x: i16){
        self.position.x = x
    }
}

impl Renderer {
    pub fn clear(&self, rect: &Rect) {
        self.context.clear_rect(
            rect.x().into(),
            rect.y().into(),
            rect.w.into(),
            rect.h.into(),
        );
    }

    pub fn draw_image(&self, image: &HtmlImageElement, frame: &Rect, destination: &Rect) {
        self.context
            .draw_image_with_html_image_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &image,
                frame.x().into(),
                frame.y().into(),
                frame.w.into(),
                frame.h.into(),
                destination.x().into(),
                destination.y().into(),
                destination.w.into(),
                destination.h.into()
            )
            .expect("Drawing is throwing exceptions! Unrecoverable error.");
    }

    pub fn draw_entire_image(
        &self,
        image: &HtmlImageElement,
        position: &Point
    ) {
        self.context
            .draw_image_with_html_image_element(
                image, position.x.into(), position.y.into())
            .expect("Drawing is throwing exceptions! Unrecoverable error.")
    }
}

enum KeyPress {
    KeyUp(web_sys::KeyboardEvent),
    KeyDown(web_sys::KeyboardEvent),
}

fn prepare_input() -> Result<UnboundedReceiver<KeyPress>> {
    let (keydown_sender, keyevent_receiver) = unbounded();
    let keydown_sender = Rc::new(RefCell::new(keydown_sender));
    let keyup_sender = Rc::clone(&keydown_sender);

    let onkeydown = browser::closure_wrap(
        Box::new(move |keycode: web_sys::KeyboardEvent| {
            keydown_sender
                .borrow_mut()
                .start_send(KeyPress::KeyDown(keycode));
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);

    let onkeyup = browser::closure_wrap(
        Box::new(move |keycode: web_sys::KeyboardEvent| {
            keyup_sender
                .borrow_mut()
                .start_send(KeyPress::KeyUp(keycode));
        }) as Box<dyn FnMut(web_sys::KeyboardEvent)>);

    browser::window()?.set_onkeydown(Some(onkeydown.as_ref().unchecked_ref()));
    browser::window()?.set_onkeyup(Some(onkeyup.as_ref().unchecked_ref()));
    onkeydown.forget();
    onkeyup.forget();
    Ok(keyevent_receiver)
}

pub struct KeyState {
    pressed_keys: HashMap<String, web_sys::KeyboardEvent>,
}

impl KeyState {
    fn new() -> Self {
        KeyState {
            pressed_keys: HashMap::new(),
        }
    }

    pub fn is_pressed(&self, code: &str) -> bool {
        self.pressed_keys.contains_key(code)
    }

    fn set_pressed(&mut self, code: &str, event: web_sys::KeyboardEvent) {
        self.pressed_keys.insert(code.into(), event);
    }

    fn set_released(&mut self, code: &str) {
        self.pressed_keys.remove(code.into());
    }
}

fn process_input(
    state: &mut KeyState,
    keyevent_receiver: &mut UnboundedReceiver<KeyPress>,
) {
    loop {
        match keyevent_receiver.try_next() {
            Ok(None) => break,
            Err(_err) => break,
            Ok(Some(evt)) => match evt {
                KeyPress::KeyUp(evt) => state.set_released(&evt.code()),
                KeyPress::KeyDown(evt) => state.set_pressed(&evt.code(), evt),
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Point {
    pub x: i16,
    pub y: i16,
}

pub struct Image {
    element: HtmlImageElement,
    bounding_box: Rect,
}

impl Image {
    pub fn new(element: HtmlImageElement, position: Point) -> Self {
        let bounding_box = Rect::new(
            position,
            element.width() as i16,
            element.height() as i16
        );

        Self {
            element,
            bounding_box,
        }
    }

    pub fn draw(&self, renderer: &Renderer) {
        renderer.draw_entire_image(&self.element, &self.bounding_box.position)
    }

    pub fn bounding_box(&self) -> &Rect {
        &self.bounding_box
    }

    pub fn move_horizontally(&mut self, distance: i16) {
      self.set_x(self.bounding_box.x() + distance);
    }

    pub fn set_x(&mut self, x: i16) {
        self.bounding_box.set_x(x);
    }

    pub fn right(&self) -> i16 {
        self.bounding_box.right()
    }
}

pub struct SpriteSheet {
    sheet: Sheet,
    image: HtmlImageElement,
}

impl SpriteSheet {
    pub fn new(sheet: Sheet, image: HtmlImageElement) -> Self {
        Self {
            sheet,
            image,
        }
    }

    pub fn cell(&self, name: &str) -> Option<&Cell> {
        self.sheet.frames.get(name)
    }

    pub fn draw(&self, renderer: &Renderer, source: &Rect, destination: &Rect) {
        renderer.draw_image(&self.image, source, destination)
    }
}

#[derive(Clone)]
pub struct Audio {
    context: AudioContext,
}

impl Audio {
    pub fn new() -> Result<Self> {
        Ok(Audio {
            context: sound::create_audio_context()?,
        })
    }

    pub async fn load_sound(&self, filename: &str) -> Result<Sound> {
        let array_buffer = browser::fetch_array_buffer(filename).await?;
        let audio_buffer = sound::decode_audio_data(&self.context, &array_buffer).await?;
        Ok(Sound {
            buffer: audio_buffer,
        })
    }

    pub fn play_sound(&self, sound: &Sound) -> Result<()> {
        sound::play_sound(&self.context, &sound.buffer, sound::LOOPING::NO)
    }

    pub fn play_looping_sound(&self, sound: &Sound) -> Result<()> {
        sound::play_sound(&self.context, &sound.buffer, sound::LOOPING::YES)
    }
}

#[derive(Clone)]
pub struct Sound {
    buffer: AudioBuffer,
}