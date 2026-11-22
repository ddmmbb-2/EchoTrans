//! EVENT — UI 的唯一非同步入口。僅 Final 可寫入持久原文；譯文只用來顯示。
#[derive(Debug)]
pub enum Event {
    AsrStatus(String),
    TranslatorStatus(String),
    AsrReady(String),
    TranslatorReady(String),
    TranslatorPid(u32),
    AsrError(String),
    TranslatorError(String),
    Recording(bool),
    Preview { mark: (u64, u32), text: String },
    Final { mark: (u64, u32), text: String },
    Translation { mark: (u64, u32), text: String },
    TranslationError { mark: (u64, u32) },
    Warning(String),
}
