/*
RAMENLISTEN v0.7 — AI / LLM MAINTENANCE CONTRACT (IN SOURCE, NO AGENTS.md)
- 主維護對象是 LLM。優先易定位、易整段替換、低耦合；不必迎合人類傳統閱讀／排版慣例。
- 只保留：Rust + Qwen3-ASR 0.6B + Hy-MT2-1.8B + GPU + 單視窗三句字幕 + 原文存檔。
- v0.7：UI 自動跟隨 Windows 顯示語系；UI/翻譯目標互不影響；預設翻譯 zh-Hant（台灣）。
- 靜態 UI 翻譯集中 i18n.rs，不經過 AI；翻譯目標帶入每一筆 Hy-MT2 工作，繁體才進 OpenCC。
- 變更設定即時落盤 settings-v0.7.json，舊 v0.6 設定自動遷移；不建立 AGENTS.md。
- 嚴禁重新引入 Google、Argos、Python、uv、Moonshine、Ollama、自動進階 LLM 或 CPU 備援。
- 模型/執行元件自動下載、驗證格式、啟動預載；使用者只開啟 EXE，不要要求手動安裝。
- 原文只由 Event::Final 落盤；Preview、Translation 絕對不能存入逐字稿。
- 啟動預設只有無邊框半透明置頂字幕。上方拖曳列；設定面板必須手動開啟。
- source of truth: config.rs(設定), model.rs(ASR), translate.rs(Hy), work.rs(麥克風與事件),
  caption.rs(三句字幕), audio.rs(收音/VAD), main.rs(唯一視窗/存檔), event.rs(訊息),
  i18n.rs(固定 UI 詞庫／Windows 顯示語系)。
- 若要更新 llama.cpp 版本，需同步更新 translate.rs 釘選資產並驗證 Windows Vulkan 發行檔。
*/
mod audio;
mod caption;
mod config;
mod event;
mod i18n;
mod model;
mod translate;
mod work;

use crate::{config::Settings, event::Event, i18n::Key as K};
use chrono::Local;
use eframe::egui;
use std::{fs::{self, File}, io::Write, path::PathBuf,
    sync::{atomic::{AtomicBool, Ordering}, mpsc::{self, Receiver, Sender, SyncSender}, Arc, RwLock},
    time::Duration};

struct RawLine { time: String, text: String }
struct App {
    settings: Settings,
    system_ui_lang: String,
    target_current: Arc<RwLock<String>>,
    rx: Receiver<Event>,
    worker: SyncSender<work::Command>,
    translation_sender: Option<Sender<translate::Job>>,
    translator_pid: Option<u32>,
    stop: Option<Arc<AtomicBool>>,
    ready_asr: bool,
    ready_translate: bool,
    failed_asr: bool,
    failed_translate: bool,
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
        let settings = config::load();
        let system_ui_lang = i18n::windows_display_locale();
        let lang = i18n::resolve(&settings.interface_language, &system_ui_lang);
        let target_current = Arc::new(RwLock::new(settings.target_language.clone()));
        let (tx, rx) = mpsc::channel();
        // Qwen3-ASR 與 Hy-MT2 並行預載；設定目標語言不會影響 UI 語言。
        let (translation_sender, translation_rx) = mpsc::channel();
        translate::launch(translation_rx, tx.clone());
        let worker = work::launch(tx, translation_sender.clone(), target_current.clone());
        Self {
            settings, system_ui_lang, target_current, rx, worker,
            translation_sender: Some(translation_sender), translator_pid: None,
            stop: None, ready_asr: false, ready_translate: false,
            failed_asr: false, failed_translate: false,
            recording: false, stopping: false,
            status_asr: i18n::tr(lang, K::LoadingAsr).into(),
            status_translate: i18n::tr(lang, K::LoadingTranslator).into(),
            warnings: Vec::new(), show_settings: false, show_history: false,
            captions: caption::History::default(),
            original_now: i18n::tr(lang, K::PreparingGpu).into(),
            translated_now: i18n::tr(lang, K::WhenReady).into(),
            last_mark: (0, 0), raw_log: Vec::new(), file: None, path: None,
        }
    }
    fn lang(&self) -> &'static str { i18n::resolve(&self.settings.interface_language, &self.system_ui_lang) }
    fn t(&self, key: K) -> &'static str { i18n::tr(self.lang(), key) }
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
        self.original_now = self.t(K::StartingMic).into();
        self.translated_now = self.t(K::WaitingSpeech).into();
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
                Event::AsrReady(s) => { self.ready_asr = true; self.failed_asr = false; self.status_asr = s; }
                Event::TranslatorReady(s) => { self.ready_translate = true; self.failed_translate = false; self.status_translate = s; }
                Event::TranslatorPid(pid) => self.translator_pid = Some(pid),
                Event::AsrError(s) => { self.ready_asr = false; self.failed_asr = true; self.status_asr = s.clone(); self.warn(s); }
                Event::TranslatorError(s) => { self.ready_translate = false; self.failed_translate = true; self.status_translate = s.clone(); self.warn(s); }
                Event::Recording(on) => {
                    self.recording = on;
                    if !on { self.stop = None; self.file = None; self.stopping = false; }
                }
                Event::Preview { mark, text } => {
                    let hint = self.t(K::Transcribing);
                    self.captions.upsert(mark, text.clone(), hint);
                    if mark >= self.last_mark { self.last_mark = mark; self.original_now = text; self.translated_now = self.t(K::WaitingPause).into(); }
                }
                Event::Final { mark, text } => {
                    let hint = self.t(K::Translating);
                    self.captions.upsert(mark, text.clone(), hint);
                    if mark >= self.last_mark { self.last_mark = mark; self.original_now = text.clone(); self.translated_now = self.t(K::Translating).into(); }
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
                Event::TranslationError { mark } => {
                    let text = self.t(K::TranslationFailed).to_owned();
                    self.captions.translation(mark, text.clone());
                    if mark == self.last_mark { self.translated_now = text; }
                }
                Event::Warning(s) => self.warn(s),
            }
        }
        // v0.7：用事件旗標判斷失敗，不再以中文「失敗」字串判斷，避免多語 UI 壞掉。
        // 無字幕時每幀使用當前 UI 語言設定待機文字；已有辨識結果絕不覆蓋。
        if !self.recording && !self.stopping && self.captions.recent().is_empty() {
            if self.ready() {
                self.original_now = self.t(K::ModelLoaded).into();
                self.translated_now = self.t(K::ClickStart).into();
            } else if self.failed_asr || self.failed_translate {
                self.original_now = self.t(K::InitFailed).into();
                self.translated_now = self.t(K::SeeSettings).into();
            } else {
                self.original_now = self.t(K::PreparingGpu).into();
                self.translated_now = self.t(K::WhenReady).into();
            }
        }
    }
    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("title")
            .frame(egui::Frame::default().fill(egui::Color32::from_rgba_unmultiplied(17, 24, 36, 228))
                .inner_margin(egui::Margin::symmetric(10, 5)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let status = if self.recording { K::Recording }
                        else if self.ready() { K::Ready }
                        else if self.failed_asr || self.failed_translate { K::Failed }
                        else { K::Preparing };
                    let drag = ui.add(egui::Label::new(
                        egui::RichText::new(format!("⠿  RamenListen     {}", self.t(status)))
                            .color(egui::Color32::from_rgb(194, 213, 231)))
                        .sense(egui::Sense::click_and_drag()));
                    if drag.drag_started() { ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag); }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text(self.t(K::Close)).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.small_button("⚙").on_hover_text(self.t(K::SettingsTip)).clicked() {
                            self.show_settings = !self.show_settings;
                        }
                        if self.recording {
                            if ui.add_enabled(!self.stopping, egui::Button::new(self.t(K::Stop))).clicked() { self.end(); }
                        } else if ui.add_enabled(self.ready(), egui::Button::new(self.t(K::Start))).clicked() { self.begin(); }
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
        let lang = self.lang();
        egui::Window::new(self.t(K::WindowTitle))
            .open(&mut open).default_width(490.0).resizable(true).scroll(true)
            .show(ctx, |ui| {
                ui.heading(i18n::tr(lang, K::DisplaySection));
                let mut changed = false;
                egui::ComboBox::from_label(i18n::tr(lang, K::InterfaceLanguage))
                    .selected_text(if self.settings.interface_language == "system" {
                        i18n::tr(lang, K::FollowSystem).to_owned()
                    } else { i18n::INTERFACE_CHOICES.iter()
                        .find(|(code, _)| *code == self.settings.interface_language.as_str())
                        .map(|(_, label)| *label).unwrap_or("English").to_owned()
                    })
                    .show_ui(ui, |ui| {
                        for (code, label) in i18n::INTERFACE_CHOICES {
                            let text = if *code == "system" { i18n::tr(lang, K::FollowSystem) } else { *label };
                            changed |= ui.selectable_value(&mut self.settings.interface_language, (*code).into(), text).changed();
                        }
                    });
                egui::ComboBox::from_label(i18n::tr(lang, K::AsrLanguage))
                    .selected_text(source_label(lang, self.settings.source)).show_ui(ui, |ui| {
                        for source in config::Source::ALL {
                            changed |= ui.selectable_value(&mut self.settings.source, source, source_label(lang, source)).changed();
                        }
                    });
                egui::ComboBox::from_label(i18n::tr(lang, K::TargetLanguage))
                    .selected_text(config::target(&self.settings.target_language).label)
                    .show_ui(ui, |ui| {
                        for t in config::TARGETS {
                            changed |= ui.selectable_value(&mut self.settings.target_language, t.code.into(), t.label).changed();
                        }
                    });
                ui.small(i18n::tr(lang, K::IndependentHint));
                ui.small(i18n::tr(lang, K::ChangeNextPhrase));
                changed |= ui.add(egui::Slider::new(&mut self.settings.pause_ms, 200..=800).text(i18n::tr(lang, K::Pause))).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.preview_ms, 900..=3600).text(i18n::tr(lang, K::Preview))).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.font_size, 16.0..=42.0).text(i18n::tr(lang, K::Font))).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.background_opacity, 30..=230).text(i18n::tr(lang, K::Background))).changed();
                changed |= ui.add(egui::Slider::new(&mut self.settings.vad_threshold, 0.004..=0.10).text(i18n::tr(lang, K::Vad))).changed();
                if changed {
                    // 改目標即同步到工作執行緒；已排隊翻譯句維持舊目標，新句用新語言。
                    if let Ok(mut target) = self.target_current.write() { *target = self.settings.target_language.clone(); }
                    if let Err(e) = config::save(&self.settings) { self.warn(format!("{}: {e}", i18n::tr(lang, K::Messages))); }
                }
                ui.separator();
                ui.heading(i18n::tr(lang, K::ModelStatus));
                ui.label(format!("{}: {}", i18n::tr(lang, K::AsrLabel), self.status_asr));
                ui.label(format!("{}: {}", i18n::tr(lang, K::TranslateLabel), self.status_translate));
                ui.label(i18n::tr(lang, K::ModelNote));
                ui.small(i18n::tr(lang, K::ErrorsTechnical));
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(i18n::tr(lang, K::Transcript)).clicked() { self.show_history = !self.show_history; }
                    if ui.button(i18n::tr(lang, K::OpenFolder)).clicked() {
                        let path = config::dir().join("transcripts"); let _ = fs::create_dir_all(&path);
                        #[cfg(windows)] { let _ = std::process::Command::new("explorer.exe").arg(path).spawn(); }
                    }
                    if ui.button(i18n::tr(lang, K::CopyOriginal)).clicked() {
                        ctx.copy_text(self.raw_log.iter().map(|x| format!("[{}] {}",x.time,x.text)).collect::<Vec<_>>().join("\n"));
                    }
                });
                if self.show_history {
                    egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
                        for line in self.raw_log.iter().rev().take(60) { ui.label(format!("[{}] {}", line.time, line.text)); }
                    });
                }
                if let Some(path) = &self.path { ui.small(format!("{}: {}", i18n::tr(lang, K::CurrentOriginal), path.display())); }
                if !self.warnings.is_empty() {
                    ui.separator();
                    ui.collapsing(format!("{} ({})", i18n::tr(lang, K::Messages), self.warnings.len()), |ui| {
                        for line in self.warnings.iter().rev().take(12) { ui.colored_label(egui::Color32::LIGHT_RED, line); }
                    });
                }
            });
        self.show_settings = open;
    }
}

fn source_label(lang: &str, source: config::Source) -> &'static str {
    match source {
        config::Source::Auto => i18n::tr(lang, K::SourceAuto),
        config::Source::English => "English",
        config::Source::Japanese => "日本語",
        config::Source::Mandarin => i18n::tr(lang, K::SourceChinese),
        config::Source::Korean => "한국어",
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
/// 使用系統已安裝字體作為多語字幕後備；不打包、下載、散布任何字型檔。
fn install_cjk(ctx: &egui::Context) {
    #[cfg(windows)]
    let paths: &[(&str, &str)] = &[
        ("CJK-System", "C:\\Windows\\Fonts\\msjh.ttc"),
        ("CJK-Simplified", "C:\\Windows\\Fonts\\msyh.ttc"),
        ("Japanese-System", "C:\\Windows\\Fonts\\meiryo.ttc"),
        ("Korean-System", "C:\\Windows\\Fonts\\malgun.ttf"),
        ("Latin-System", "C:\\Windows\\Fonts\\segoeui.ttf"),
        ("Indic-System", "C:\\Windows\\Fonts\\Nirmala.ttf"),
        ("Thai-System", "C:\\Windows\\Fonts\\LeelawUI.ttf"),
    ];
    #[cfg(not(windows))]
    let paths: &[(&str, &str)] = &[
        ("CJK-System", "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        ("CJK-Alternative", "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc"),
        ("Mac-System", "/System/Library/Fonts/PingFang.ttc"),
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut count = 0;
    for (name, path) in paths {
        if let Ok(bytes) = fs::read(path) {
            fonts.font_data.insert((*name).into(), egui::FontData::from_owned(bytes).into());
            let families = fonts.families.entry(egui::FontFamily::Proportional).or_default();
            if count == 0 { families.insert(0, (*name).into()); }
            else { families.push((*name).into()); }
            count += 1;
        }
    }
    if count > 0 { ctx.set_fonts(fonts); }
}
fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default().with_title("RamenListen v0.7")
        .with_inner_size([760.0, 370.0]).with_min_inner_size([380.0, 220.0])
        .with_transparent(true).with_decorations(false).with_always_on_top().with_resizable(true);
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("RamenListen", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
