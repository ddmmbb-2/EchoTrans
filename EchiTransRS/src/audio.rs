//! CPAL 音訊輸入與基本 VAD。錄音 callbacks 只做轉型、發送，不跑模型或網路。
use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use std::sync::{atomic::{AtomicUsize, Ordering}, mpsc::{SyncSender, TrySendError}, Arc};

pub struct AudioChunk { pub samples: Vec<f32> }
pub struct Microphone { pub stream: Stream, pub sample_rate: u32, pub channels: usize, pub dropped: Arc<AtomicUsize> }

fn send_chunk<T: Copy>(data: &[T], tx: &SyncSender<AudioChunk>, dropped: &AtomicUsize, convert: impl Fn(T) -> f32) {
    let samples: Vec<f32> = data.iter().copied().map(convert).collect();
    match tx.try_send(AudioChunk { samples }) {
        Err(TrySendError::Full(_)) => { dropped.fetch_add(1, Ordering::Relaxed); },
        Err(TrySendError::Disconnected(_)) | Ok(()) => {},
    }
}

pub fn open(tx: SyncSender<AudioChunk>) -> Result<Microphone> {
    let host = cpal::default_host();
    let device = host.default_input_device().context("未找到預設麥克風，請檢查 Windows 麥克風權限")?;
    let supported = device.default_input_config().context("無法取得麥克風音訊格式")?;
    let sample_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let format = supported.sample_format();
    let config = supported.config();
    let dropped = Arc::new(AtomicUsize::new(0));
    let err_fn = |e| eprintln!("麥克風驅動錯誤：{e}");
    let stream = match format {
        SampleFormat::F32 => { let d = dropped.clone(); device.build_input_stream(&config, move |s: &[f32], _| send_chunk(s, &tx, &d, |x| x), err_fn, None)? },
        SampleFormat::I16 => { let d = dropped.clone(); device.build_input_stream(&config, move |s: &[i16], _| send_chunk(s, &tx, &d, |x| x as f32 / 32768.0), err_fn, None)? },
        SampleFormat::U16 => { let d = dropped.clone(); device.build_input_stream(&config, move |s: &[u16], _| send_chunk(s, &tx, &d, |x| (x as f32 - 32768.0) / 32768.0), err_fn, None)? },
        other => return Err(anyhow!("麥克風格式暫不支援：{other:?}")),
    };
    stream.play()?;
    Ok(Microphone { stream, sample_rate, channels, dropped })
}

/// 分塊線性重採樣；保留上一塊尾端和相位，避免每個 callback 丟失音訊。
pub struct Resampler { step: f64, position: f64, prev: Option<f32> }
impl Resampler {
    pub fn new(input_rate: u32) -> Self { Self { step: input_rate as f64 / 16000.0, position: 0.0, prev: None } }
    pub fn process(&mut self, interleaved: &[f32], channels: usize) -> Vec<f32> {
        if channels == 0 { return Vec::new(); }
        let frames = interleaved.len() / channels;
        if frames == 0 { return Vec::new(); }
        let mut mono = Vec::with_capacity(frames + 1);
        if let Some(last) = self.prev { mono.push(last); }
        for frame in interleaved.chunks_exact(channels) {
            mono.push(frame.iter().sum::<f32>() / channels as f32);
        }
        let mut out = Vec::new();
        while self.position + 1.0 < mono.len() as f64 {
            let left = self.position.floor() as usize;
            let blend = (self.position - left as f64) as f32;
            out.push(mono[left] * (1.0 - blend) + mono[left + 1] * blend);
            self.position += self.step;
        }
        self.position -= (mono.len() - 1) as f64;
        self.prev = mono.last().copied();
        out
    }
}

/// 每 100ms 為一框；短停頓切句，長停頓交給 pipeline 聚合段落。
pub struct Segmenter {
    pub frame_buffer: std::collections::VecDeque<f32>,
    pre_roll: std::collections::VecDeque<f32>,
    pub segment: Vec<f32>,
    pub quiet_frames: usize,
    pub active: bool,
    pub voiced_frames: usize,
    silence_limit_frames: usize,
    /// 本次 push 是否有出現語音，用於長停頓偵測。
    pub saw_voice: bool,
}
impl Segmenter {
    pub fn new(pause_ms: u32) -> Self { Self { frame_buffer: Default::default(), pre_roll: Default::default(), segment: Vec::new(), quiet_frames: 0, active: false, voiced_frames: 0, silence_limit_frames: ((pause_ms.clamp(200, 800) + 99) / 100) as usize, saw_voice: false } }
    pub fn push(&mut self, samples: &[f32], threshold: f32) -> Vec<Vec<f32>> {
        self.saw_voice = false;
        self.frame_buffer.extend(samples.iter().copied());
        let mut finalized = Vec::new();
        while self.frame_buffer.len() >= 1600 {
            let frame: Vec<f32> = self.frame_buffer.drain(..1600).collect();
            let rms = (frame.iter().map(|x| x*x).sum::<f32>()/1600.0).sqrt();
            let speaking = rms > threshold;
            self.saw_voice |= speaking;
            if !self.active {
                if speaking {
                    self.active = true;
                    self.segment.extend(self.pre_roll.iter().copied());
                    self.segment.extend(&frame);
                    self.voiced_frames = 1;
                    self.quiet_frames = 0;
                    self.pre_roll.clear();
                } else {
                    self.pre_roll.extend(frame.iter().copied());
                    while self.pre_roll.len() > 4800 { self.pre_roll.pop_front(); }
                }
            } else {
                self.segment.extend(&frame);
                if speaking { self.voiced_frames += 1; self.quiet_frames = 0; }
                else { self.quiet_frames += 1; }
                if self.quiet_frames >= self.silence_limit_frames || self.segment.len() >= 16_000 * 6 {
                    if let Some(done) = self.finalize_segment() { finalized.push(done); }
                }
            }
        }
        finalized
    }
    pub fn finalize_segment(&mut self) -> Option<Vec<f32>> {
        let mut samples = std::mem::take(&mut self.segment);
        if self.quiet_frames > 0 {
            let trim = (self.quiet_frames * 1600).saturating_sub(1600).min(samples.len());
            samples.truncate(samples.len() - trim);
        }
        let voiced = self.voiced_frames;
        self.active = false; self.quiet_frames = 0; self.voiced_frames = 0; self.pre_roll.clear();
        if voiced < 2 || samples.len() < 3200 { None } else { Some(samples) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn silence_doesnt_trigger() {
        let mut s = Segmenter::new(250);
        for _ in 0..20 { assert!(s.push(&[0.0; 1600], 0.02).is_empty()); }
    }
    #[test]
    fn speech_then_silence_closes() {
        let mut s = Segmenter::new(250);
        for _ in 0..7 { assert!(s.push(&[0.15; 1600], 0.02).is_empty()); }
        let mut done = Vec::new();
        for _ in 0..8 { done.extend(s.push(&[0.; 1600], 0.02)); }
        assert_eq!(done.len(),1);
        assert!(done[0].len() > 8000);
    }
    #[test]
    fn short_pause_closes_before_seven_hundred_ms() {
        let mut s = Segmenter::new(200);
        for _ in 0..6 { assert!(s.push(&[0.18; 1600], 0.02).is_empty()); }
        let mut completed = Vec::new();
        for _ in 0..3 { completed.extend(s.push(&[0.0; 1600], 0.02)); }
        assert_eq!(completed.len(), 1, "200ms 靜音應立刻切句");
    }
    #[test]
    fn resample_48k_to_16k() {
        let mut r = Resampler::new(48000);
        let mut n = 0;
        for _ in 0..100 { n += r.process(&vec![0.5; 480], 1).len(); }
        assert!((n as i32 - 16000).abs() <= 2, "{n}");
    }
}
