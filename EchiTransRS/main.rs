/*
RAMENLISTEN v0.6 — AI / LLM MAINTENANCE CONTRACT (IN SOURCE, NO AGENTS.md)
- 主維護對象是 LLM。優先易定位、易整段替換、低耦合；不必迎合人類傳統閱讀／排版慣例。
- 只保留：Rust + Qwen3-ASR 0.6B + Hy-MT2-1.8B + GPU + 單視窗三句字幕 + 原文存檔。
- 嚴禁重新引入 Google、Argos、Python、uv、Moonshine、Ollama、自動進階 LLM 或 CPU 備援。
- 模型/執行元件自動下載、驗證格式、啟動預載；使用者只開啟 EXE，不要要求手動安裝。
- 原文只由 Event::Final 落盤；Preview、Translation 絕對不能存入逐字稿。
- 啟動預設只有無邊框半透明置頂字幕。上方拖曳列；設定面板必須手動開啟。
- source of truth: config.rs(設定), model.rs(ASR), translate.rs(Hy), work.rs(麥克風與事件),
  caption.rs(三句字幕), audio.rs(收音/VAD), main.rs(唯一視窗/存檔), event.rs(訊息)。
- 若要更新 llama.cpp 版本，需同步更新 translate.rs 釘選資產並驗證 Windows Vulkan 發行檔。
*/
mod audio;
mod caption;
mod config;
mod event;
mod model;
mod translate;
mod work;

use crate::{config::Settings, event::Event};
use chrono::Local;
use eframe::egui;
use std::{fs::{self, File}, io::Write, path::PathBuf,
    sync::{atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, Sender, SyncSender}, Arc},
    time::Duration};

struct RawLine { time: String, text: String }
struct App {
    settings: Settings,
    rx: Receiver<Event>,
    worker: SyncSender<work::Command>,
    translation_sender: Option<Sender<translate::Job>>,
    translator_pid: Option<u32>,
    stop: Option<Arc<AtomicBool>>,
    ready_asr: bool,
    ready_translate: bool,
    recording: bool,
    stopping: bool,
    status_asr: String,
    status_translate: String,
    warnings: Vec<String>,
    show_settings: bool,
    show_history: bool,
    captions: caption::History,
    original_now: String,
    translated_now: String,
    last_mark: (u64, u32),
    raw_log: Vec<RawLine>,
    file: Option<File>,
    path: Option<PathBuf>,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let (tx, rx) = mpsc::channel();
        // 兩個模型並行準備。按「開始」時不載入模型，只開啟麥克風。
        let (translation_sender, translation_rx) = mpsc::channel();
        translate::launch(translation_rx, tx.clone());
        let worker = work::launch(tx, translation_sender.clone());
        Self {
            settings: config::load(), rx, worker,
            translation_sender: Some(translation_sender), translator_pid: None,
            stop: None, ready_asr: false, ready_translate: false,
            recording: false, stopping: false,
            status_asr: "正在載入 Qwen3-ASR…".into(),
            status_translate: "正在載入 Hy-MT2…".into(),
            warnings: Vec::new(), show_settings: false, show_history: false,
            captions: caption::History::default(),
            original_now: "正在準備 GPU 模型…".into(),
            translated_now: "完成後即可開始聽寫".into(),
            last_mark: (0, 0), raw_log: Vec::new(), file: None, path: None,
        }
    }
    fn ready(&self) -> bool { self.ready_asr && self.ready_translate }
    fn begin(&mut self) {
        if !self.ready() || self.recording || self.stopping { return; }
        if let Err(e) = config::save(&self.settings) { self.warn(format!("無法儲存設定：{e}")); }
        let dir = config::dir().join("transcripts");
        if let Err(e) = fs::create_dir_all(&dir) { self.warn(format!("無法建立存檔資料夾：{e}")); return; }
        let file_path = dir.join(format!("original-{}.txt", Local::now().format("%Y%m%d-%H%M%S-%3f")));
        let result = (|| -> std::io::Result<File> {
            let mut f = File::create(&file_path)?;
            writeln!(f, "RamenListen — 原文逐字稿 | {}\n", Local::now().format("%Y-%m-%d %H:%M:%S"))?;
            Ok(f)
        })();
        match result {
            Ok(f) => { self.file = Some(f); self.path = Some(file_path); }
            Err(e) => { self.warn(format!("無法建立原文逐字稿：{e}")); return; }
        }
        self.captions.clear();
        self.last_mark = (0, 0);
        self.original_now = "正在開始收音…".into();
        self.translated_now = "等待語音…".into();
        let stop = Arc::new(AtomicBool::new(false));
        match self.worker.send(work::Command::Start { settings: self.settings.clone(), stop: stop.clone() }) {
            Ok(()) => { self.stop = Some(stop); self.recording = true; }
            Err(e) => { self.file = None; self.warn(format!("無法啟動收音：{e}")); }
        }
    }
    fn end(&mut self) {
        if let Some(stop) = &self.stop { stop.store(true, Ordering::Relaxed); }
        self.stopping = true;
    }
    fn warn(&mut self, msg: String) {
        self.warnings.push(msg);
        if self.warnings.len() > 30 { self.warnings.remove(0); }
    }
    fn poll(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::AsrStatus(s) => self.status_asr = s,
                Event::TranslatorStatus(s) => self.status_translate = s,
                Event::AsrReady(s) => { self.ready_asr = true; self.status_asr = s; }
                Event::TranslatorReady(s) => { self.ready_translate = true; self.status_translate = s; }
                Event::TranslatorPid(pid) => self.translator_pid = Some(pid),
                Event::AsrError(s) => { self.ready_asr = false; self.status_asr = s.clone(); self.warn(s); }
                Event::TranslatorError(s) => { self.ready_translate = false; self.status_translate = s.clone(); self.warn(s); }
                Event::Recording(on) => {
                    self.recording = on;
                    if !on { self.stop = None; self.file = None; self.stopping = false; }
                }
                Event::Preview { mark, text } => {
                    self.captions.upsert(mark, text.clone(), "正在聽寫…");
                    if mark >= self.last_mark { self.last_mark = mark; self.original_now = text; self.translated_now = "等待短句結束…".into(); }
                }
                Event::Final { mark, text } => {
                    self.captions.upsert(mark, text.clone(), "正在翻譯…");
                    if mark >= self.last_mark { self.last_mark = mark; self.original_now = text.clone(); self.translated_now = "正在翻譯…".into(); }
                    let time = Local::now().format("%H:%M:%S").to_string();
                    self.raw_log.push(RawLine { time: time.clone(), text: text.clone() });
                    // 資料邊界：嚴格只存 Final 原文，翻譯結果從不保存。
                    if let Some(file) = &mut self.file {
                        if let Err(e) = writeln!(file, "[{time}] {text}\n").and_then(|_| file.flush()) {
                            self.warn(format!("原文存檔失敗：{e}"));
                        }
                    }
                }
                Event::Translation { mark, text } => {
                    self.captions.translation(mark, text.clone());
                    if mark == self.last_mark { self.translated_now = text; }
                }
                Event::Warning(s) => self.warn(s),
            }
        }
        // MODEL-READY UI FIX (v0.6.1): 啟動佔位字幕必須跟隨模型就緒／失敗事件更新。
        // 僅在尚未開始聽寫且沒有字幕時顯示；絕不覆寫已辨識的原文或譯文。
        if !self.recording && !self.stopping && self.captions.recent().is_empty() {
            if self.ready() {
                if self.original_now != "模型已載入完成" {
                    self.original_now = "模型已載入完成".into();
                    self.translated_now = "點擊「▶ 開始」即可聽寫".into();
                }
            } else if self.status_asr.contains("失敗") || self.status_translate.contains("失敗") {
                if self.original_now != "模型初始化失敗" {
                    self.original_now = "模型初始化失敗".into();
                    self.translated_now = "請開啟右上角 ⚙ 設定查看錯誤訊息".into();
                }
            }
        }
    }
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("title")
            .frame(egui::Frame::default().fill(egui::Color32::from_rgba_unmultiplied(17, 24, 36, 228))
                .inner_margin(egui::Margin::symmetric(10, 5)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let label = if self.recording { "● 聽寫中" }
                        else if self.ready() { "● 已就緒" }
                        else if self.status_asr.contains("失敗") || self.status_translate.contains("失敗") { "⚠ 模型載入失敗" }
                        else { "◌ 模型準備中" };
                    let drag = ui.add(egui::Label::new(
                        egui::RichText::new(format!("⠿  RamenListen     {label}"))
                            .color(egui::Color32::from_rgb(194, 213, 231)))
                        .sense(egui::Sense::click_and_drag()));
                    if drag.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("關閉 RamenListen").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.small_button("⚙").on_hover_text("設定／原文紀錄").clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        if self.recording {
                            if ui.add_enabled(!self.stopping, egui::Button::new("■ 停止")).clicked() { self.end(); }
                        } else if ui.add_enabled(self.ready(), egui::Button::new("▶ 開始")).clicked() { self.begin(); }
                    });
                });
            });
    }
    fn caption_area(&mut self, ctx: &egui::Context) {
        let bg = self.settings.background_opacity;
        let font = self.settings.font_size;
        let lines = self.captions.recent();
        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(egui::Color32::from_rgba_unmultiplied(10, 16, 27, bg))
                .inner_margin(egui::Margin::same(10)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                    if lines.is_empty() {
                        subtitle(ui, &self.original_now, &self.translated_now, font, 0.98, 155);
                    } else {
                        for (i, line) in lines.iter().enumerate() {
                            let age = lines.len() - 1 - i;
                            let target: f32 = match age { 0 => 0.98, 1 => 0.58, _ => 0.30 };
                            let opacity = line.demoted_at.map_or(target, |since| {
                                let t = (since.elapsed().as_secs_f32() / 0.75).min(1.0);
                                0.98 + (target - 0.98) * t
                            });
                            let box_alpha = match age { 0 => 150, 1 => 75, _ => 26 };
                            subtitle(ui, &line.original, &line.translated, font, opacity, box_alpha);
                            ui.add_space(5.0);
                        }
                    }
                });
            });
    }
    fn settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings { return; }
        let mut open = self.show_settings;
        egui::Window::new("RamenListen 設定")
            .open(&mut open).default_width(490.0).resizable(true).scroll(true)
            .show(ctx, |ui| {
                ui.heading("顯示／聽寫");
                let mut changed = false;
                egui::ComboBox::from_label("來源語言")
                    .selected_text(self.settings.source.name()).show_ui(ui, |ui| {
                        for source in config::Source::ALL {
                            changed |= ui.selectable_value(&mut self.settings.source, source, source.name()).changed();
                        }
                    });
                changed |= ui.add(egui::Slider::new(&mut self.settings.pause_ms, 200..=800).text("短暫停頓 (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.preview_ms, 900..=3600).text("預覽更新 (ms)")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.font_size, 16.0..=42.0).text("字幕字體")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.background_opacity, 30..=230).text("背景不透明度")).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.vad_threshold, 0.004..=0.10).text("語音靈敏度")).changed();
                if changed { if let Err(e) = config::save(&self.settings) { self.warn(format!("儲存設定失敗：{e}")); } }
                ui.separator();
                ui.heading("模型狀態");
                ui.label(format!("ASR：{}", self.status_asr));
                ui.label(format!("翻譯：{}", self.status_translate));
                ui.label("僅支援具 Vulkan 能力的獨立 GPU；建議 8GB VRAM 以上。模型首次啟動自動下載。");
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("原文逐字稿").clicked() { self.show_history = !self.show_history; }
                    if ui.button("開啟存檔資料夾").clicked() {
                        let path = config::dir().join("transcripts"); let _ = fs::create_dir_all(&path);
                        #[cfg(windows)] { let _ = std::process::Command::new("explorer.exe").arg(path).spawn(); }
                    }
                    if ui.button("複製原文").clicked() {
                        ctx.copy_text(self.raw_log.iter().map(|x| format!("[{}] {}",x.time,x.text)).collect::<Vec<_>>().join("\n"));
                    }
                });
                if self.show_history {
                    egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                        for line in self.raw_log.iter().rev().take(60) { ui.label(format!("[{}] {}", line.time, line.text)); }
                    });
                }
                if let Some(path) = &self.path { ui.small(format!("本次原文：{}", path.display())); }
                if !self.warnings.is_empty() {
                    ui.separator();
                    ui.collapsing(format!("訊息／錯誤 ({})", self.warnings.len()), |ui| {
                        for line in self.warnings.iter().rev().take(12) { ui.colored_label(egui::Color32::LIGHT_RED, line); }
                    });
                }
            });
        self.show_settings = open;
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll();
        ctx.request_repaint_after(Duration::from_millis(80));
        self.top_bar(ctx);
        self.caption_area(ctx);
        self.settings(ctx);
    }
}
impl Drop for App {
    fn drop(&mut self) {
        if let Some(stop) = &self.stop { stop.store(true, Ordering::Relaxed); }
        let _ = self.worker.try_send(work::Command::Quit);
        // 將最後一個翻譯 Sender 釋放，翻譯工作執行緒隨之結束，殺掉自動啟動的 llama-server 子程序。
        self.translation_sender.take();
        // 防止 Windows 結束 GUI 時留下仍佔 GPU 的 llama-server 子程序。
        #[cfg(windows)]
        if let Some(pid) = self.translator_pid {
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("taskkill.exe")
                .arg("/PID").arg(pid.to_string()).args(["/T", "/F"])
                .creation_flags(0x0800_0000).status();
        }
    }
}
fn subtitle(ui: &mut egui::Ui, original: &str, translated: &str, font: f32, opacity: f32, background: u8) {
    let a = (opacity * 255.0).round().clamp(0.0, 255.0) as u8;
    egui::Frame::default()
        .fill(egui::Color32::from_rgba_unmultiplied(14, 21, 34, background))
        .corner_radius(egui::CornerRadius::same(7))
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(original).size((font * 0.65).max(12.0))
                .color(egui::Color32::from_rgba_unmultiplied(170, 214, 249, a)));
            ui.label(egui::RichText::new(translated).size(font)
                .color(egui::Color32::from_rgba_unmultiplied(255, 255, 255, a)));
        });
}
fn install_cjk(ctx: &egui::Context) {
    #[cfg(windows)]
    let paths = ["C:\\Windows\\Fonts\\msjh.ttc", "C:\\Windows\\Fonts\\msyh.ttc", "C:\\Windows\\Fonts\\mingliu.ttc"];
    #[cfg(not(windows))]
    let paths = ["/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc", "/System/Library/Fonts/PingFang.ttc"];
    if let Some(bytes) = paths.iter().find_map(|p| fs::read(p).ok()) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("CJK-System".into(), egui::FontData::from_owned(bytes).into());
        fonts.families.entry(egui::FontFamily::Proportional).or_default().insert(0, "CJK-System".into());
        ctx.set_fonts(fonts);
    }
}
fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default().with_title("RamenListen v0.6")
        .with_inner_size([760.0, 370.0]).with_min_inner_size([380.0, 220.0])
        .with_transparent(true).with_decorations(false).with_always_on_top().with_resizable(true);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("RamenListen", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
