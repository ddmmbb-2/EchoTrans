# RamenListen v0.7 — Rust 原始碼

這是根據 v0.6 的九份檔案更新的完整 Cargo 專案。**不是已編譯的 Windows EXE**；必須在 Windows Rust 環境建置才能實際測試 Vulkan 與麥克風功能。

## 使用方式

解壓縮後在專案根目錄執行：

```powershell
cargo test
cargo build --release
```

編譯完成後，執行 `target\release\ramen-listen.exe`。模型和本機 Vulkan 執行元件仍依舊架構在首次啟動時自動下載，使用者端不需 Python、uv、Ollama、Argos 或 Google 翻譯。建置環境本身仍需 Rust 工具鏈。Windows 版本要求 NVIDIA 獨立 GPU，最低 6 GiB VRAM（測試支援）、建議 8 GiB；雙模型在 6 GiB 卡上的穩定性仍需持續驗證。

## v0.7 實際修改

- `src/config.rs`：介面語言 `interface_language = "system"`，翻譯目標 `target_language = "zh-Hant"`，語音來源預設 `Auto`；兩者獨立。自動匯入 `settings-v0.6.json`，另存 `settings-v0.7.json`，不破壞舊版設定。
- `src/i18n.rs`：以 Windows `GetUserPreferredUILanguages` 偵測**顯示語言**（非日期／地區格式）；介面具繁體中文、簡體中文、英文、日文、韓文、法文、德文、西班牙文、葡萄牙文、俄文共十套本機靜態詞庫。不在清單內的系統語言退回英文。
- `src/main.rs`：新增兩個獨立語言選項，修改後即時保存；對於下一個辨識完成的片段立即套用新的翻譯目標，不需重啟模型。狀態列、按鈕、設定項、字幕佔位提示與翻譯失敗提示使用本機詞庫；模型技術診斷文字仍可能保留原始語言。
- `src/work.rs`：翻譯任務附帶當時的 `target`，避免等待中的句子因為設定變動而用錯語言；ASR 預設維持 Qwen3-ASR 自動語言偵測。
- `src/translate.rs`：依目標語言動態組合官方建議類型的 Hy-MT2 提示；zh-Hant 特別指定台灣用字，接著以純 Rust `ferrous-opencc` 的 `S2twp` 本機轉換處理譯文（不碰原文），支援增量字幕；若 OpenCC 初始化失敗，照常使用模型原始譯文。
- `src/event.rs`：翻譯失敗改為 UI 自行翻譯的事件。
- `src/model.rs`：更新狀態說明與版本識別。
- `src/audio.rs`：保留收音與 VAD。`src/caption.rs`：擴充為最多 500 句字幕歷史，畫面預設顯示最新約三句，可用滾輪向上回看，回到底部恢復追蹤新字幕。
- `src/main.rs`：Windows GUI 子系統啟動不顯示黑色終端；`translate.rs` 也隱藏 GPU 偵測及翻譯子程序的終端。

## 注意

本次為原始碼修改。由於目前執行環境沒有 `rustc` / `cargo`、Windows 與 NVIDIA GPU，**尚未完成真正的 Rust 編譯、模型下載與實機執行測試**。建議在 Windows 先執行 `cargo test` 和 `cargo build --release`；原本的第三方 Git ASR 相依版本與 Vulkan `llama.cpp` 發行版釘選維持不變。

LLM 維護規約已寫在 `src/main.rs` 檔頭，**沒有 AGENTS.md**。
