//! MODEL — Qwen3-ASR 0.6B GPU 專用。保留 v0.5 已實測的 wgpu 模型與快取。
//! 不要把網路下載放到 UI 或按下「開始」時才觸發。啟動 app 即檢查並載入。
use crate::{config, event::Event};
use anyhow::{anyhow, Context, Result};
use qwen3_asr_wgpu::{AsrInference, Backend, TranscribeOptions};
use reqwest::blocking::Client;
use std::{fs::{self, File}, io::{Read, Write}, path::{Path, PathBuf}, sync::mpsc::Sender, time::Duration};

const FILES: &[&str] = &[
    "config.json", "generation_config.json", "processor_config.json", "tokenizer_config.json",
    "chat_template.jinja", "tokenizer.json", "model.safetensors",
];

pub struct Model { recognizer: AsrInference, wav: PathBuf }
impl Model {
    pub fn load(events: &Sender<Event>) -> Result<Self> {
        events.send(Event::AsrStatus("啟動中：檢查 Qwen3-ASR 模型…".into())).ok();
        let dir = config::dir().join("models").join("qwen3-asr-0.6b");
        fs::create_dir_all(&dir)?;
        let client = Client::builder().connect_timeout(Duration::from_secs(12))
            .timeout(Duration::from_secs(1800)).user_agent("RamenListen/0.7").build()?;
        for (i, name) in FILES.iter().enumerate() {
            let destination = dir.join(name);
            if destination.metadata().map(|m| m.len() > 0).unwrap_or(false) { continue; }
            events.send(Event::AsrStatus(format!("首次下載模型 {}/{}：{}", i+1, FILES.len(), name))).ok();
            let url = format!("https://huggingface.co/Qwen/Qwen3-ASR-0.6B-hf/resolve/main/{name}");
            let mut res = client.get(url).send()?.error_for_status()?;
            let size = res.content_length();
            let part = destination.with_extension("part");
            let result: Result<()> = (|| {
                let mut out = File::create(&part)?;
                let mut buf = [0u8; 128 * 1024];
                let mut total = 0u64;
                let mut last = 0u64;
                loop {
                    let n = res.read(&mut buf)?;
                    if n == 0 { break; }
                    out.write_all(&buf[..n])?;
                    total += n as u64;
                    if total.saturating_sub(last) >= 6 * 1024 * 1024 {
                        last = total;
                        events.send(Event::AsrStatus(format!("正在下載 {}：{:.1} MiB", name, total as f64 / 1_048_576.0))).ok();
                    }
                }
                out.flush()?;
                if let Some(expected) = size { if expected != total { return Err(anyhow!("下載檔案大小不符：{name}")); } }
                Ok(())
            })();
            if let Err(e) = result { let _ = fs::remove_file(&part); return Err(e); }
            fs::rename(&part, &destination)?;
        }
        events.send(Event::AsrStatus("正在將 Qwen3-ASR 載入 GPU…".into())).ok();
        let recognizer = AsrInference::load(dir.as_path(), Backend::best()).context("載入 Qwen3-ASR 失敗")?;
        let device = recognizer.device_description();
        if !device.contains("DiscreteGpu") { return Err(anyhow!("需要獨立 GPU（最低 6GB VRAM；建議 8GB）；目前運算裝置：{device}")); }
        events.send(Event::AsrReady(format!("Qwen3-ASR 0.6B • {}", device))).ok();
        let wav = config::dir().join(format!("qwen-temp-{}.wav", std::process::id()));
        Ok(Self { recognizer, wav })
    }
    pub fn transcribe(&mut self, audio: &[f32], lang: config::Source) -> Result<String> {
        if audio.len() < 3200 { return Ok(String::new()); }
        write_wav(&self.wav, audio)?;
        let opts = match lang.qwen_name() {
            Some(name) => TranscribeOptions::default().with_language(name),
            None => TranscribeOptions::default(),
        };
        let wav = self.wav.to_str().context("音訊暫存路徑無法轉成 UTF-8")?;
        Ok(self.recognizer.transcribe(wav, opts)?.text.trim().to_string())
    }
}
impl Drop for Model {
    fn drop(&mut self) { let _ = fs::remove_file(&self.wav); }
}
fn write_wav(path: &Path, samples: &[f32]) -> Result<()> {
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut out = hound::WavWriter::create(path, spec)?;
    for x in samples { out.write_sample((x.clamp(-1.0, 1.0) * 32767.0) as i16)?; }
    out.finalize()?;
    Ok(())
}
