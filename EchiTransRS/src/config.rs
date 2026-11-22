//! CONFIG — 唯一設定來源。設定介面預設隱藏，啟動只顯示字幕。
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source { Auto, English, Japanese, Mandarin, Korean }
impl Source {
    pub const ALL: [Self; 5] = [Self::Auto, Self::English, Self::Japanese, Self::Mandarin, Self::Korean];
    pub fn name(self) -> &'static str { match self {
        Self::Auto => "多語自動偵測", Self::English => "English", Self::Japanese => "日本語",
        Self::Mandarin => "中文", Self::Korean => "한국어" } }
    pub fn qwen_name(self) -> Option<&'static str> { match self {
        Self::Auto => None, Self::English => Some("English"), Self::Japanese => Some("Japanese"),
        Self::Mandarin => Some("Chinese"), Self::Korean => Some("Korean") } }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub source: Source,
    pub pause_ms: u32,
    pub preview_ms: u32,
    pub vad_threshold: f32,
    pub font_size: f32,
    pub background_opacity: u8,
}
impl Default for Settings {
    fn default() -> Self { Self { source: Source::Auto, pause_ms: 300, preview_ms: 1700,
        vad_threshold: 0.018, font_size: 25.0, background_opacity: 135 } }
}
pub fn dir() -> PathBuf {
    ProjectDirs::from("org", "ddmmbb", "RamenListen")
        .map(|p| p.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("./ramen-listen-data"))
}
pub fn load() -> Settings {
    fs::read_to_string(dir().join("settings-v0.6.json")).ok()
        .and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
}
pub fn save(settings: &Settings) -> anyhow::Result<()> {
    fs::create_dir_all(dir())?;
    fs::write(dir().join("settings-v0.6.json"), serde_json::to_vec_pretty(settings)?)?;
    Ok(())
}
