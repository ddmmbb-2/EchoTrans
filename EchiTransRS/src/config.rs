//! CONFIG — v0.7 唯一持久設定來源；介面語言與翻譯語言完全獨立。
//! LLM 維護：不要把系統介面語言當作翻譯預設語言；舊 v0.6 檔案須安全遷移。
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source { Auto, English, Japanese, Mandarin, Korean }
impl Source {
    pub const ALL: [Self; 5] = [Self::Auto, Self::English, Self::Japanese, Self::Mandarin, Self::Korean];
    pub fn qwen_name(self) -> Option<&'static str> { match self {
        Self::Auto => None, Self::English => Some("English"), Self::Japanese => Some("Japanese"),
        Self::Mandarin => Some("Chinese"), Self::Korean => Some("Korean")
    } }
}

/// Hy-MT2 官方支援的語言清單；prompt 必須是完整語言名稱，code 只供保存與 UI 識別。
#[derive(Clone, Copy)]
pub struct Target { pub code: &'static str, pub label: &'static str, pub prompt: &'static str }
pub const TARGETS: &[Target] = &[
    Target { code:"zh-Hant", label:"繁體中文（台灣）", prompt:"Traditional Chinese" },
    Target { code:"zh", label:"簡體中文", prompt:"Simplified Chinese" },
    Target { code:"en", label:"English", prompt:"English" },
    Target { code:"ja", label:"日本語", prompt:"Japanese" },
    Target { code:"ko", label:"한국어", prompt:"Korean" },
    Target { code:"fr", label:"Français", prompt:"French" },
    Target { code:"de", label:"Deutsch", prompt:"German" },
    Target { code:"es", label:"Español", prompt:"Spanish" },
    Target { code:"pt", label:"Português", prompt:"Portuguese" },
    Target { code:"ru", label:"Русский", prompt:"Russian" },
    Target { code:"it", label:"Italiano", prompt:"Italian" },
    Target { code:"tr", label:"Türkçe", prompt:"Turkish" },
    Target { code:"ar", label:"العربية", prompt:"Arabic" },
    Target { code:"th", label:"ไทย", prompt:"Thai" },
    Target { code:"vi", label:"Tiếng Việt", prompt:"Vietnamese" },
    Target { code:"ms", label:"Bahasa Melayu", prompt:"Malay" },
    Target { code:"id", label:"Bahasa Indonesia", prompt:"Indonesian" },
    Target { code:"tl", label:"Filipino", prompt:"Filipino" },
    Target { code:"hi", label:"हिन्दी", prompt:"Hindi" },
    Target { code:"pl", label:"Polski", prompt:"Polish" },
    Target { code:"cs", label:"Čeština", prompt:"Czech" },
    Target { code:"nl", label:"Nederlands", prompt:"Dutch" },
    Target { code:"km", label:"ខ្មែរ", prompt:"Khmer" },
    Target { code:"my", label:"မြန်မာ", prompt:"Burmese" },
    Target { code:"fa", label:"فارسی", prompt:"Persian" },
    Target { code:"gu", label:"ગુજરાતી", prompt:"Gujarati" },
    Target { code:"ur", label:"اردو", prompt:"Urdu" },
    Target { code:"te", label:"తెలుగు", prompt:"Telugu" },
    Target { code:"mr", label:"मराठी", prompt:"Marathi" },
    Target { code:"he", label:"עברית", prompt:"Hebrew" },
    Target { code:"bn", label:"বাংলা", prompt:"Bengali" },
    Target { code:"ta", label:"தமிழ்", prompt:"Tamil" },
    Target { code:"uk", label:"Українська", prompt:"Ukrainian" },
    Target { code:"bo", label:"བོད་སྐད", prompt:"Tibetan" },
    Target { code:"kk", label:"Қазақша", prompt:"Kazakh" },
    Target { code:"mn", label:"Монгол", prompt:"Mongolian" },
    Target { code:"ug", label:"ئۇيغۇرچە", prompt:"Uyghur" },
    Target { code:"yue", label:"粵語", prompt:"Cantonese" },
];
pub fn target(code: &str) -> Target { TARGETS.iter().copied().find(|x| x.code == code).unwrap_or(TARGETS[0]) }
fn default_ui_language() -> String { "system".into() }
fn default_target_language() -> String { "zh-Hant".into() }

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub interface_language: String, // system | zh-Hant | zh-Hans | en | ja | ko | fr | de | es | pt | ru
    pub target_language: String,    // Hy-MT2 code; independent of interface_language
    pub source: Source,
    pub pause_ms: u32,
    pub preview_ms: u32,
    pub vad_threshold: f32,
    pub font_size: f32,
    pub background_opacity: u8,
}
impl Default for Settings {
    fn default() -> Self { Self {
        interface_language: default_ui_language(), target_language: default_target_language(),
        source: Source::Auto, pause_ms:300, preview_ms:1700, vad_threshold:0.018,
        font_size:25.0, background_opacity:135,
    } }
}
impl Settings {
    pub fn normalize(&mut self) {
        self.target_language = target(&self.target_language).code.into();
        if !crate::i18n::is_interface_preference(&self.interface_language) {
            self.interface_language = default_ui_language();
        }
    }
}
pub fn dir() -> PathBuf {
    ProjectDirs::from("org", "ddmmbb", "RamenListen")
        .map(|p| p.data_local_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("./ramen-listen-data"))
}
/// v0.7 優先讀取；只在新檔不存在時匯入 v0.6，避免讀取舊設定後覆蓋新選擇。
pub fn load() -> Settings {
    let new_path = dir().join("settings-v0.7.json");
    if let Ok(data) = fs::read_to_string(&new_path) {
        if let Ok(mut settings) = serde_json::from_str::<Settings>(&data) {
            settings.normalize(); return settings;
        }
    }
    let old_path = dir().join("settings-v0.6.json");
    if let Ok(data) = fs::read_to_string(old_path) {
        if let Ok(mut settings) = serde_json::from_str::<Settings>(&data) {
            settings.normalize();
            let _ = save(&settings);
            return settings;
        }
    }
    Settings::default()
}
pub fn save(settings: &Settings) -> anyhow::Result<()> {
    fs::create_dir_all(dir())?;
    let target_path = dir().join("settings-v0.7.json");
    let temp_path = dir().join("settings-v0.7.tmp");
    fs::write(&temp_path, serde_json::to_vec_pretty(settings)?)?;
    // Windows rename 不允許覆蓋既有檔，先嘗試原子替換；若失敗則採單檔覆寫。
    if fs::rename(&temp_path, &target_path).is_err() {
        fs::write(&target_path, serde_json::to_vec_pretty(settings)?)?;
        let _ = fs::remove_file(temp_path);
    }
    Ok(())
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn default_zh_hant_independent_of_ui() {
        let s = Settings::default();
        assert_eq!(s.target_language, "zh-Hant");
        assert_eq!(s.interface_language, "system");
        assert_eq!(s.source, Source::Auto);
    }
    #[test] fn migrate_v06_payload() {
        let s: Settings = serde_json::from_str(r#"{"source":"Japanese","font_size":32.0}"#).unwrap();
        assert_eq!(s.source, Source::Japanese);
        assert_eq!(s.target_language, "zh-Hant");
        assert_eq!(s.font_size, 32.0);
    }
    #[test] fn target_is_valid_and_persistent() {
        let mut s = Settings::default(); s.target_language = "ja".into();
        let mut decoded: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        decoded.normalize(); assert_eq!(decoded.target_language, "ja");
    }
}
