use std::env;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use eframe::egui;
use egui::{Color32, FontId, Pos2, Rect, Stroke, Vec2};
use image::{Rgba, RgbaImage};
use serde::Deserialize;

static BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Deserialize, Default)]
struct ThemeState {
    colors: ThemeColors,
}

#[derive(Deserialize, Default)]
struct ThemeColors {
    background: Option<String>,
    foreground: Option<String>,
    cursor: Option<String>,
    color5: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Select,
    Pen,
    Rect,
    Arrow,
    Line,
    Ellipse,
    Highlighter,
    Pixelate,
}

#[derive(Clone)]
struct Ann {
    tool: Tool,
    points: Vec<Pos2>,
    start: Pos2,
    end: Pos2,
}

struct Overlay {
    image: RgbaImage,
    texture: egui::TextureHandle,
    selection: Option<Rect>,
    drag_start: Option<Pos2>,
    tool: Tool,
    active: Option<Ann>,
    annotations: Vec<Ann>,
    accent: Color32,
    background: Color32,
    foreground: Color32,
    screen_rect: Rect,
    capture_path: PathBuf,
    finished: bool,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("daemon") => daemon(),
        Some("trigger") => trigger(),
        Some("--overlay") => {
            if let Some(path) = args.get(2) {
                run_overlay(PathBuf::from(path))
            } else {
                eprintln!("uso: screenshotd --overlay <imagem>");
                std::process::exit(2);
            }
        }
        _ => {
            eprintln!("uso: screenshotd [daemon|trigger|--overlay <imagem>]");
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        eprintln!("screenshotd: {err}");
        std::process::exit(1);
    }
}

fn socket_path() -> Result<PathBuf, String> {
    let runtime =
        env::var("XDG_RUNTIME_DIR").map_err(|_| "XDG_RUNTIME_DIR não definido".to_string())?;
    Ok(PathBuf::from(runtime).join("screenshotd.sock"))
}

fn trigger() -> Result<(), String> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path)
        .map_err(|e| format!("daemon não está rodando em {}: {e}", path.display()))?;
    stream
        .write_all(b"capture\n")
        .map_err(|e| format!("falha ao enviar comando: {e}"))?;
    Ok(())
}

fn daemon() -> Result<(), String> {
    let socket = socket_path()?;
    let _ = fs::remove_file(&socket);
    let listener =
        UnixListener::bind(&socket).map_err(|e| format!("bind {}: {e}", socket.display()))?;

    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let mut request = String::new();
                let _ = stream.read_to_string(&mut request);
                if request.trim() == "capture" {
                    request_capture();
                }
            }
            Err(err) => eprintln!("screenshotd: conexão inválida: {err}"),
        }
    }
    Ok(())
}

fn request_capture() {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }

    let runtime = match env::var("XDG_RUNTIME_DIR") {
        Ok(value) => PathBuf::from(value),
        Err(_) => {
            BUSY.store(false, Ordering::SeqCst);
            return;
        }
    };
    let capture = runtime.join(format!("screenshotd-capture-{}.png", std::process::id()));
    let status = Command::new("grim").arg(&capture).status();
    if status.map(|s| !s.success()).unwrap_or(true) {
        eprintln!("screenshotd: grim falhou");
        let _ = fs::remove_file(&capture);
        BUSY.store(false, Ordering::SeqCst);
        return;
    }

    let exe = match env::current_exe() {
        Ok(value) => value,
        Err(_) => {
            BUSY.store(false, Ordering::SeqCst);
            return;
        }
    };
    match Command::new(exe).arg("--overlay").arg(&capture).spawn() {
        Ok(mut child) => {
            thread::spawn(move || {
                let _ = child.wait();
                let _ = fs::remove_file(capture);
                BUSY.store(false, Ordering::SeqCst);
            });
        }
        Err(err) => {
            eprintln!("screenshotd: falha ao abrir overlay: {err}");
            let _ = fs::remove_file(capture);
            BUSY.store(false, Ordering::SeqCst);
        }
    }
}

fn run_overlay(path: PathBuf) -> Result<(), String> {
    let image = image::open(&path)
        .map_err(|e| format!("falha ao abrir captura: {e}"))?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let theme = load_theme();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([width as f32, height as f32])
            .with_position([0.0, 0.0])
            .with_decorations(false)
            .with_resizable(false)
            .with_app_id("screenshotd")
            .with_title("screenshotd"),
        ..Default::default()
    };
    eframe::run_native(
        "screenshotd",
        options,
        Box::new(move |cc| Ok(Box::new(Overlay::new(cc, image, path, theme)))),
    )
    .map_err(|e| format!("eframe: {e}"))
}

fn load_theme() -> (Color32, Color32, Color32) {
    let home = env::var("HOME").unwrap_or_default();
    let path = Path::new(&home).join(".local/state/hypr/theme.json");
    let fallback = (
        Color32::from_rgb(10, 10, 10),
        Color32::from_rgb(242, 242, 242),
        Color32::from_rgb(198, 40, 40),
    );
    let Ok(text) = fs::read_to_string(path) else {
        return fallback;
    };
    let Ok(state) = serde_json::from_str::<ThemeState>(&text) else {
        return fallback;
    };
    let bg = state
        .colors
        .background
        .as_deref()
        .and_then(parse_hex)
        .unwrap_or(fallback.0);
    let fg = state
        .colors
        .foreground
        .as_deref()
        .and_then(parse_hex)
        .unwrap_or(fallback.1);
    let accent = state
        .colors
        .cursor
        .as_deref()
        .or(state.colors.color5.as_deref())
        .and_then(parse_hex)
        .unwrap_or(fallback.2);
    (bg, fg, accent)
}

fn parse_hex(value: &str) -> Option<Color32> {
    let value = value.trim().trim_start_matches('#');
    if value.len() != 6 && value.len() != 8 {
        return None;
    }
    let r = u8::from_str_radix(&value[0..2], 16).ok()?;
    let g = u8::from_str_radix(&value[2..4], 16).ok()?;
    let b = u8::from_str_radix(&value[4..6], 16).ok()?;
    let a = if value.len() == 8 {
        u8::from_str_radix(&value[6..8], 16).ok()?
    } else {
        255
    };
    Some(Color32::from_rgba_unmultiplied(r, g, b, a))
}

impl Overlay {
    fn new(
        cc: &eframe::CreationContext<'_>,
        image: RgbaImage,
        capture_path: PathBuf,
        theme: (Color32, Color32, Color32),
    ) -> Self {
        let (width, height) = image.dimensions();
        let rgba = image.as_raw();
        let color_image =
            egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], rgba);
        let texture =
            cc.egui_ctx
                .load_texture("screenshot", color_image, egui::TextureOptions::LINEAR);
        let (background, foreground, accent) = theme;
        Self {
            image,
            texture,
            selection: None,
            drag_start: None,
            tool: Tool::Select,
            active: None,
            annotations: Vec::new(),
            accent,
            background,
            foreground,
            screen_rect: Rect::from_min_size(Pos2::ZERO, Vec2::new(width as f32, height as f32)),
            capture_path,
            finished: false,
        }
    }

    fn confirm(&mut self) -> Result<(), String> {
        let selection = self
            .selection
            .ok_or_else(|| "nenhuma região selecionada".to_string())?
            .intersect(self.screen_rect);
        if selection.width() < 1.0 || selection.height() < 1.0 {
            return Err("região selecionada vazia".to_string());
        }
        let scale_x = self.image.width() as f32 / self.screen_rect.width().max(1.0);
        let scale_y = self.image.height() as f32 / self.screen_rect.height().max(1.0);
        let mut min_x = ((selection.min.x - self.screen_rect.min.x) * scale_x)
            .floor()
            .clamp(0.0, self.image.width() as f32 - 1.0) as u32;
        let mut min_y = ((selection.min.y - self.screen_rect.min.y) * scale_y)
            .floor()
            .clamp(0.0, self.image.height() as f32 - 1.0) as u32;
        let mut max_x = ((selection.max.x - self.screen_rect.min.x) * scale_x)
            .ceil()
            .clamp(1.0, self.image.width() as f32) as u32;
        let mut max_y = ((selection.max.y - self.screen_rect.min.y) * scale_y)
            .ceil()
            .clamp(1.0, self.image.height() as f32) as u32;
        if max_x <= min_x {
            max_x = (min_x + 1).min(self.image.width());
            min_x = min_x.min(max_x.saturating_sub(1));
        }
        if max_y <= min_y {
            max_y = (min_y + 1).min(self.image.height());
            min_y = min_y.min(max_y.saturating_sub(1));
        }
        let width = max_x.saturating_sub(min_x).max(1);
        let height = max_y.saturating_sub(min_y).max(1);
        let mut output =
            image::imageops::crop_imm(&self.image, min_x, min_y, width, height).to_image();
        let accent = Rgba([self.accent.r(), self.accent.g(), self.accent.b(), 255]);
        for ann in &self.annotations {
            draw_annotation(&mut output, ann, selection.min, scale_x, scale_y, accent);
        }
        if let Some(active) = &self.active {
            draw_annotation(&mut output, active, selection.min, scale_x, scale_y, accent);
        }

        let temp = env::temp_dir().join(format!("screenshotd-{}.png", std::process::id()));
        output
            .save(&temp)
            .map_err(|e| format!("falha ao salvar PNG temporário: {e}"))?;
        let data = fs::read(&temp).map_err(|e| format!("falha ao ler PNG temporário: {e}"))?;
        let _ = fs::remove_file(&temp);

        let mut child = Command::new("wl-copy")
            .arg("--type")
            .arg("image/png")
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("falha ao iniciar wl-copy: {e}"))?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(&data)
                .map_err(|e| format!("falha ao enviar para wl-copy: {e}"))?;
        }
        let _ = child.wait();
        Ok(())
    }
}

impl eframe::App for Overlay {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            if self.confirm().is_ok() {
                self.finished = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::P)) {
            self.tool = Tool::Pen;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::R)) {
            self.tool = Tool::Rect;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::A)) {
            self.tool = Tool::Arrow;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::L)) {
            self.tool = Tool::Line;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.tool = Tool::Ellipse;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::H)) {
            self.tool = Tool::Highlighter;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::B)) {
            self.tool = Tool::Pixelate;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::S)) {
            self.tool = Tool::Select;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z)) {
            self.annotations.pop();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Backspace)) {
            self.annotations.pop();
        }

        let screen = ui.max_rect();
        let response = ui.interact(
            screen,
            ui.id().with("screenshot-canvas"),
            egui::Sense::click_and_drag(),
        );
        self.screen_rect = screen;
        let painter = ui.painter();
            painter.image(
                self.texture.id(),
                screen,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );

            if let Some(selection) = self.selection {
                let dim = Color32::from_black_alpha(120);
                painter.rect_filled(Rect::from_min_max(screen.min, Pos2::new(screen.max.x, selection.min.y)), 0.0, dim);
                painter.rect_filled(Rect::from_min_max(Pos2::new(screen.min.x, selection.max.y), screen.max), 0.0, dim);
                painter.rect_filled(Rect::from_min_max(Pos2::new(screen.min.x, selection.min.y), Pos2::new(selection.min.x, selection.max.y)), 0.0, dim);
                painter.rect_filled(Rect::from_min_max(Pos2::new(selection.max.x, selection.min.y), Pos2::new(screen.max.x, selection.max.y)), 0.0, dim);
                let clipped = painter.with_clip_rect(selection);
                for ann in &self.annotations {
                    draw_ann_egui(&clipped, ann, self.accent);
                }
                if let Some(active) = &self.active {
                    draw_ann_egui(&clipped, active, self.accent);
                }
                let border = Stroke::new(2.0, self.accent);
                painter.line_segment([selection.min, Pos2::new(selection.max.x, selection.min.y)], border);
                painter.line_segment([Pos2::new(selection.max.x, selection.min.y), selection.max], border);
                painter.line_segment([selection.max, Pos2::new(selection.min.x, selection.max.y)], border);
                painter.line_segment([Pos2::new(selection.min.x, selection.max.y), selection.min], border);
            } else {
                painter.rect_filled(screen, 0.0, Color32::from_black_alpha(80));
            }

            // Usa o estado bruto do ponteiro em vez de drag_started/dragged/drag_stopped.
            // O egui só dispara drag_started depois de alguns pixels de movimento, o que
            // fazia a seleção perder o canto inicial; primary_pressed captura o ponto exato.
            let pointer_pos = ctx.input(|i| i.pointer.interact_pos());
            let primary_pressed = ctx.input(|i| i.pointer.primary_pressed());
            let primary_down = ctx.input(|i| i.pointer.primary_down());
            let primary_released = ctx.input(|i| i.pointer.primary_released());

            if primary_pressed {
                if let Some(pos) = pointer_pos {
                    if screen.contains(pos) {
                        let pos = clamp_pos(pos, screen);
                        self.drag_start = Some(pos);
                        if self.tool == Tool::Select {
                            self.selection = Some(Rect::from_two_pos(pos, pos));
                        } else if let Some(selection) = self.selection {
                            if selection.contains(pos) {
                                self.active = Some(Ann {
                                    tool: self.tool,
                                    points: vec![pos],
                                    start: pos,
                                    end: pos,
                                });
                            }
                        }
                    }
                }
            }

            if primary_down {
                if let (Some(start), Some(pos)) = (self.drag_start, pointer_pos) {
                    let pos = clamp_pos(pos, screen);
                    match self.tool {
                        Tool::Select => {
                            self.selection = Some(Rect::from_two_pos(start, pos).intersect(screen));
                        }
                        Tool::Pen | Tool::Highlighter => {
                            if let Some(active) = &mut self.active {
                                if active.points.last().copied() != Some(pos) {
                                    active.points.push(pos);
                                }
                                active.end = pos;
                            }
                        }
                        Tool::Rect | Tool::Arrow | Tool::Line | Tool::Ellipse | Tool::Pixelate => {
                            if let Some(active) = &mut self.active {
                                active.end = pos;
                            }
                        }
                    }
                }
            }

            if primary_released {
                if self.tool == Tool::Select {
                    if let Some(selection) = self.selection {
                        let selection = selection.intersect(screen);
                        if selection.width() < 4.0 || selection.height() < 4.0 {
                            self.selection = None;
                        } else {
                            self.selection = Some(selection);
                        }
                    }
                } else if let Some(active) = self.active.take() {
                    if active.points.len() > 1 || active.start.distance(active.end) > 2.0 {
                        self.annotations.push(active);
                    }
                }
                self.drag_start = None;
            }

            let hint = format!(
                "Arraste para selecionar  |  P caneta  |  L linha  |  R retângulo  |  E elipse  |  A seta  |  H marca-texto  |  B pixelar  |  Ctrl+Z  |  Enter copiar  |  Esc   [{}]",
                match self.tool {
                    Tool::Select => "seleção",
                    Tool::Pen => "caneta",
                    Tool::Rect => "retângulo",
                    Tool::Arrow => "seta",
                    Tool::Line => "linha",
                    Tool::Ellipse => "elipse",
                    Tool::Highlighter => "marca-texto",
                    Tool::Pixelate => "pixelar",
                }
            );
            let galley = painter.layout_no_wrap(hint, FontId::proportional(13.0), self.foreground);
            let hint_size = galley.size() + Vec2::new(16.0, 10.0);
            let hint_bg = Rect::from_min_size(
                Pos2::new(
                    screen.center().x - hint_size.x / 2.0,
                    screen.max.y - hint_size.y - 24.0,
                ),
                hint_size,
            );
            painter.rect_filled(hint_bg, 6.0, Color32::from_black_alpha(190));
            painter.galley(hint_bg.min + Vec2::new(8.0, 5.0), galley, self.foreground);

            // Cursor de precisão: círculo branco com borda preta, sem a seta do sistema.
            ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::None);
            if let Some(pos) = ctx.pointer_latest_pos() {
                if screen.contains(pos) {
                    painter.circle_filled(pos, 5.0, Color32::WHITE);
                    painter.circle_stroke(pos, 5.0, Stroke::new(1.5, Color32::BLACK));
                }
            }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = fs::remove_file(&self.capture_path);
    }
}

fn clamp_pos(pos: Pos2, rect: Rect) -> Pos2 {
    Pos2::new(
        pos.x.clamp(rect.min.x, rect.max.x),
        pos.y.clamp(rect.min.y, rect.max.y),
    )
}

fn draw_ann_egui(painter: &egui::Painter, ann: &Ann, color: Color32) {
    let stroke = Stroke::new(3.0, color);
    let highlight = Stroke::new(
        14.0,
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 80),
    );
    match ann.tool {
        Tool::Pen => {
            for pair in ann.points.windows(2) {
                painter.line_segment([pair[0], pair[1]], stroke);
            }
        }
        Tool::Highlighter => {
            for pair in ann.points.windows(2) {
                painter.line_segment([pair[0], pair[1]], highlight);
            }
        }
        Tool::Rect => {
            let rect = Rect::from_two_pos(ann.start, ann.end);
            painter.line_segment([rect.min, Pos2::new(rect.max.x, rect.min.y)], stroke);
            painter.line_segment([Pos2::new(rect.max.x, rect.min.y), rect.max], stroke);
            painter.line_segment([rect.max, Pos2::new(rect.min.x, rect.max.y)], stroke);
            painter.line_segment([Pos2::new(rect.min.x, rect.max.y), rect.min], stroke);
        }
        Tool::Line => {
            painter.line_segment([ann.start, ann.end], stroke);
        }
        Tool::Ellipse => {
            draw_ellipse_egui(painter, ann.start, ann.end, stroke);
        }
        Tool::Arrow => {
            painter.line_segment([ann.start, ann.end], stroke);
            draw_arrow_head_egui(painter, ann.start, ann.end, color);
        }
        Tool::Pixelate => {
            let rect = Rect::from_two_pos(ann.start, ann.end);
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(120));
            painter.line_segment([rect.min, Pos2::new(rect.max.x, rect.min.y)], stroke);
            painter.line_segment([Pos2::new(rect.max.x, rect.min.y), rect.max], stroke);
            painter.line_segment([rect.max, Pos2::new(rect.min.x, rect.max.y)], stroke);
            painter.line_segment([Pos2::new(rect.min.x, rect.max.y), rect.min], stroke);
        }
        Tool::Select => {}
    }
}

fn draw_arrow_head_egui(painter: &egui::Painter, start: Pos2, end: Pos2, color: Color32) {
    let dir = end - start;
    let len = dir.length();
    if len < 1.0 {
        return;
    }
    let unit = dir / len;
    let normal = Vec2::new(-unit.y, unit.x);
    let base = end - unit * 14.0;
    let stroke = Stroke::new(3.0, color);
    painter.line_segment([end, base + normal * 7.0], stroke);
    painter.line_segment([end, base - normal * 7.0], stroke);
}

fn ellipse_points(start: Pos2, end: Pos2) -> Vec<Pos2> {
    let center = Pos2::new((start.x + end.x) * 0.5, (start.y + end.y) * 0.5);
    let rx = (end.x - start.x).abs() * 0.5;
    let ry = (end.y - start.y).abs() * 0.5;
    (0..=64)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / 64.0;
            Pos2::new(center.x + rx * a.cos(), center.y + ry * a.sin())
        })
        .collect()
}

fn draw_ellipse_egui(painter: &egui::Painter, start: Pos2, end: Pos2, stroke: Stroke) {
    let points = ellipse_points(start, end);
    for pair in points.windows(2) {
        painter.line_segment([pair[0], pair[1]], stroke);
    }
}

fn draw_annotation(
    image: &mut RgbaImage,
    ann: &Ann,
    origin: Pos2,
    scale_x: f32,
    scale_y: f32,
    color: Rgba<u8>,
) {
    let map = |p: Pos2| {
        (
            ((p.x - origin.x) * scale_x).round() as i32,
            ((p.y - origin.y) * scale_y).round() as i32,
        )
    };
    match ann.tool {
        Tool::Pen => {
            for pair in ann.points.windows(2) {
                let (x0, y0) = map(pair[0]);
                let (x1, y1) = map(pair[1]);
                draw_line(image, x0, y0, x1, y1, color);
            }
        }
        Tool::Rect => {
            let (x0, y0) = map(ann.start);
            let (x1, y1) = map(ann.end);
            draw_line(image, x0, y0, x1, y0, color);
            draw_line(image, x1, y0, x1, y1, color);
            draw_line(image, x1, y1, x0, y1, color);
            draw_line(image, x0, y1, x0, y0, color);
        }
        Tool::Line => {
            let (x0, y0) = map(ann.start);
            let (x1, y1) = map(ann.end);
            draw_line(image, x0, y0, x1, y1, color);
        }
        Tool::Ellipse => {
            let points: Vec<(i32, i32)> =
                ellipse_points(ann.start, ann.end).into_iter().map(map).collect();
            for pair in points.windows(2) {
                draw_line(image, pair[0].0, pair[0].1, pair[1].0, pair[1].1, color);
            }
        }
        Tool::Highlighter => {
            let highlight = Rgba([color.0[0], color.0[1], color.0[2], 90]);
            for pair in ann.points.windows(2) {
                let (x0, y0) = map(pair[0]);
                let (x1, y1) = map(pair[1]);
                draw_thick_line(image, x0, y0, x1, y1, 14, highlight);
            }
        }
        Tool::Pixelate => {
            let (mut x0, mut y0) = map(ann.start);
            let (mut x1, mut y1) = map(ann.end);
            if x0 > x1 {
                std::mem::swap(&mut x0, &mut x1);
            }
            if y0 > y1 {
                std::mem::swap(&mut y0, &mut y1);
            }
            pixelate_region(image, x0, y0, x1, y1, 12);
        }
        Tool::Arrow => {
            let (x0, y0) = map(ann.start);
            let (x1, y1) = map(ann.end);
            draw_line(image, x0, y0, x1, y1, color);
            let dx = (x1 - x0) as f32;
            let dy = (y1 - y0) as f32;
            let len = (dx * dx + dy * dy).sqrt();
            if len > 1.0 {
                let ux = dx / len;
                let uy = dy / len;
                let bx = x1 as f32 - ux * 14.0;
                let by = y1 as f32 - uy * 14.0;
                draw_line(
                    image,
                    x1,
                    y1,
                    (bx - uy * 7.0) as i32,
                    (by + ux * 7.0) as i32,
                    color,
                );
                draw_line(
                    image,
                    x1,
                    y1,
                    (bx + uy * 7.0) as i32,
                    (by - ux * 7.0) as i32,
                    color,
                );
            }
        }
        Tool::Select => {}
    }
}

fn blend_pixel(image: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x < 0 || y < 0 || x as u32 >= image.width() || y as u32 >= image.height() {
        return;
    }
    let dst = image.get_pixel_mut(x as u32, y as u32);
    let a = color.0[3] as f32 / 255.0;
    for i in 0..3 {
        dst.0[i] = (color.0[i] as f32 * a + dst.0[i] as f32 * (1.0 - a)).round() as u8;
    }
    dst.0[3] = 255;
}

fn draw_line_alpha(image: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgba<u8>) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        blend_pixel(image, x, y, color);
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn draw_thick_line(
    image: &mut RgbaImage,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: i32,
    color: Rgba<u8>,
) {
    let dx = (x1 - x0) as f32;
    let dy = (y1 - y0) as f32;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1.0 {
        return;
    }
    let nx = -dy / len;
    let ny = dx / len;
    let half = thickness / 2;
    for offset in -half..=half {
        let ox = (nx * offset as f32).round() as i32;
        let oy = (ny * offset as f32).round() as i32;
        draw_line_alpha(image, x0 + ox, y0 + oy, x1 + ox, y1 + oy, color);
    }
}

fn pixelate_region(image: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, block: u32) {
    let x0 = x0.max(0).min(image.width() as i32);
    let y0 = y0.max(0).min(image.height() as i32);
    let x1 = x1.max(0).min(image.width() as i32);
    let y1 = y1.max(0).min(image.height() as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let mut by = y0 as u32;
    while by < y1 as u32 {
        let mut bx = x0 as u32;
        while bx < x1 as u32 {
            let ex = (bx + block).min(x1 as u32);
            let ey = (by + block).min(y1 as u32);
            let mut sum = [0u64; 3];
            let mut count = 0u64;
            for y in by..ey {
                for x in bx..ex {
                    let px = image.get_pixel(x, y);
                    sum[0] += px.0[0] as u64;
                    sum[1] += px.0[1] as u64;
                    sum[2] += px.0[2] as u64;
                    count += 1;
                }
            }
            if count > 0 {
                let avg = Rgba([
                    (sum[0] / count) as u8,
                    (sum[1] / count) as u8,
                    (sum[2] / count) as u8,
                    255,
                ]);
                for y in by..ey {
                    for x in bx..ex {
                        image.put_pixel(x, y, avg);
                    }
                }
            }
            bx += block;
        }
        by += block;
    }
}

fn draw_line(image: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, color: Rgba<u8>) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let (mut x, mut y) = (x0, y0);
    loop {
        for oy in -1..=1 {
            for ox in -1..=1 {
                let px = x + ox;
                let py = y + oy;
                if px >= 0 && py >= 0 && (px as u32) < image.width() && (py as u32) < image.height()
                {
                    image.put_pixel(px as u32, py as u32, color);
                }
            }
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}
