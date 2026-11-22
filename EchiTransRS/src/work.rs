//! WORK — 獨立收音/ASR 工作：模型開啟即預載，開始只啟動麥克風；翻譯單向投遞 Hy-MT2。
//! 保證：音訊 callback 絕不等待 ASR/GPU/HTTP；Final 不被預覽覆蓋，翻譯不阻塞辨識。
use crate::{audio, config::Settings, event::Event, model::Model, translate};
use anyhow::Result;
use std::{sync::{atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender, SyncSender, TryRecvError}, Arc}, thread, time::{Duration, Instant}};

pub enum Command { Start { settings: Settings, stop: Arc<AtomicBool> }, Quit }
struct Clip { phrase: u64, samples: Vec<f32> }
static NEXT_PHRASE: AtomicU64 = AtomicU64::new(1);

pub fn launch(events: Sender<Event>, translation: Sender<translate::Job>) -> SyncSender<Command> {
    let (tx, rx) = mpsc::sync_channel(4);
    thread::spawn(move || {
        let mut model = match Model::load(&events) {
            Ok(m) => m,
            Err(e) => { let _ = events.send(Event::AsrError(format!("語音模型載入失敗：{e:#}"))); return; }
        };
        let _ = events.send(Event::AsrStatus("Qwen3-ASR 已載入 GPU，等待開始聽寫".into()));
        while let Ok(cmd) = rx.recv() {
            match cmd {
                Command::Quit => break,
                Command::Start { settings, stop } => {
                    if let Err(e) = session(&mut model, &settings, stop, &events, &translation) {
                        let _ = events.send(Event::Warning(format!("聽寫工作中斷：{e:#}")));
                    }
                    let _ = events.send(Event::Recording(false));
                    let _ = events.send(Event::AsrStatus("已停止聽寫，模型仍保留在 GPU".into()));
                }
            }
        }
    });
    tx
}

fn session(model: &mut Model, settings: &Settings, stop: Arc<AtomicBool>, events: &Sender<Event>,
           translator: &Sender<translate::Job>) -> Result<()> {
    let (final_tx, final_rx) = mpsc::sync_channel::<Clip>(32);
    let (preview_tx, preview_rx) = mpsc::sync_channel::<Clip>(1);
    let source = settings.source;
    let capture_events = events.clone();
    let capture_settings = settings.clone();
    let capture = thread::spawn(move || {
        if let Err(e) = capture_audio(&capture_settings, stop, final_tx, preview_tx, &capture_events) {
            let _ = capture_events.send(Event::Warning(format!("麥克風：{e:#}")));
        }
    });
    let mut final_closed = false;
    let mut preview_closed = false;
    let mut last_final = 0u64;
    let mut revision = 0u32;
    let mut last_preview_warning = Instant::now();
    while !(final_closed && preview_closed) {
        match final_rx.try_recv() {
            Ok(clip) => {
                last_final = clip.phrase;
                revision = 0;
                match model.transcribe(&clip.samples, source) {
                    Ok(text) if !text.is_empty() => {
                        let mark = (clip.phrase, u32::MAX);
                        let _ = events.send(Event::Final { mark, text: text.clone() });
                        let _ = translator.send(translate::Job { mark, text });
                    }
                    Ok(_) => {},
                    Err(e) => { let _ = events.send(Event::Warning(format!("語音辨識：{e:#}"))); }
                }
                continue;
            }
            Err(TryRecvError::Empty) => {},
            Err(TryRecvError::Disconnected) => final_closed = true,
        }
        match preview_rx.try_recv() {
            Ok(clip) => {
                if clip.phrase <= last_final { continue; }
                revision = revision.saturating_add(1);
                match model.transcribe(&clip.samples, source) {
                    Ok(text) if !text.is_empty() => {
                        let _ = events.send(Event::Preview { mark: (clip.phrase, revision), text });
                    }
                    Ok(_) => {},
                    Err(e) => {
                        if last_preview_warning.elapsed() > Duration::from_secs(12) {
                            let _ = events.send(Event::Warning(format!("預覽辨識跳過：{e}")));
                            last_preview_warning = Instant::now();
                        }
                    }
                }
            }
            Err(TryRecvError::Empty) => { thread::sleep(Duration::from_millis(15)); }
            Err(TryRecvError::Disconnected) => { preview_closed = true; thread::sleep(Duration::from_millis(10)); }
        }
    }
    if capture.join().is_err() { let _ = events.send(Event::Warning("收音執行緒異常終止".into())); }
    Ok(())
}

fn capture_audio(settings: &Settings, stop: Arc<AtomicBool>, final_tx: SyncSender<Clip>,
                 preview_tx: SyncSender<Clip>, events: &Sender<Event>) -> Result<()> {
    let (audio_tx, audio_rx) = mpsc::sync_channel(256);
    let microphone = audio::open(audio_tx)?;
    let _ = events.send(Event::Recording(true));
    let mut resampler = audio::Resampler::new(microphone.sample_rate);
    let mut segmenter = audio::Segmenter::new(settings.pause_ms);
    let mut phrase = NEXT_PHRASE.fetch_add(1, Ordering::Relaxed);
    let mut preview_clock = Instant::now();
    let mut loss_clock = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let chunk = match audio_rx.recv_timeout(Duration::from_millis(75)) {
            Ok(v) => v,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let pcm = resampler.process(&chunk.samples, microphone.channels);
        for samples in segmenter.push(&pcm, settings.vad_threshold.clamp(0.004, 0.15)) {
            if final_tx.send(Clip { phrase, samples }).is_err() { return Ok(()); }
            phrase = NEXT_PHRASE.fetch_add(1, Ordering::Relaxed);
            preview_clock = Instant::now();
        }
        if segmenter.active && segmenter.segment.len() >= 32_000 &&
            preview_clock.elapsed() >= Duration::from_millis(settings.preview_ms.clamp(900, 3600) as u64) {
            let _ = preview_tx.try_send(Clip { phrase, samples: segmenter.segment.clone() });
            preview_clock = Instant::now();
        }
        if loss_clock.elapsed() > Duration::from_secs(6) {
            let dropped = microphone.dropped.swap(0, Ordering::Relaxed);
            if dropped > 0 { let _ = events.send(Event::Warning(format!("收音緩衝遺失 {dropped} 區塊"))); }
            loss_clock = Instant::now();
        }
    }
    if let Some(samples) = segmenter.finalize_segment() {
        let _ = final_tx.send(Clip { phrase, samples });
    }
    drop(microphone.stream);
    Ok(())
}
