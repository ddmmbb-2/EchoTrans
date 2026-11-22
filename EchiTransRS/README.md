# RamenListen v0.6 — GPU 本機即時字幕

**開啟只有字幕小視窗。Qwen3-ASR 0.6B 辨識、Hy-MT2-1.8B 翻成繁體中文，兩者皆在本機 GPU 執行。**

## 使用者

- Windows 10/11 x64、NVIDIA Vulkan 獨立 GPU，至少 8GB VRAM（8192 MiB），建議 12GB 以上；程式會用 `nvidia-smi` 檢查顯示記憶體。8GB 下的實際速度仍需實測。
- 開啟 `ramen-listen.exe`：第一次自動下載 Qwen3-ASR、Hy-MT2 Q4 GGUF、官方 Windows Vulkan llama-server 並預載；後續使用本機快取，聽寫與翻譯不需網路。
- 模型全部就緒後按字幕上方「▶ 開始」。頂端可拖曳、停止、開啟設定、關閉；整體半透明，最近三句越舊越淡，譯文會串流回填。
- 設定與原文紀錄藏在右上角齒輪選單。完整原文只在 ASR 確認定稿後寫進 `%LOCALAPPDATA%\RamenListen\transcripts`（依 OS 專案資料路徑而定），**絕不存快譯**。
- 首次下載及預熱可能較久；如果 GPU／驅動不相容或 VRAM 不足，顯示錯誤，不轉為 CPU 推論。

## 技術設計

- Rust eframe/egui 視窗 + cpal 麥克風 + qwen3-asr-wgpu（保留早期實測可用的固定 Git revision）。
- Hy-MT2：官方 [`tencent/Hy-MT2-1.8B-GGUF`](https://huggingface.co/tencent/Hy-MT2-1.8B-GGUF) 的 `Q4_K_M`；官方 [`llama.cpp`](https://github.com/ggml-org/llama.cpp/releases) 之固定版本 `b12625` Windows x64 Vulkan 發行包。
- `llama-server.exe` 為本程式自動管理的**本機子程序**，不是遠端翻譯 API，不需要使用者安裝或執行；綁定隨機 `127.0.0.1` 埠；在程式關閉時結束。
- `translate.rs` 透過 localhost OpenAI 相容串流介面把短句交給 Hy-MT2，逐步回傳譯文；`work.rs` 只負責收音、ASR 與送出翻譯事件。
- UI 只保存 `Final` 原文；預覽與翻譯是暫時顯示狀態。
- **明確刪除** Google、Argos/Python/uv、Moonshine、CPU 無 GPU 降級、段落 LLM、自動摘要、獨立大型主視窗、AGENTS.md。長期 LLM 維護規範直接放在 `src/main.rs` 檔頭，未來只需提供源碼。

## Windows 開發者建置

1. 安裝 Rust stable（MSVC x64）、Visual Studio C++ Build Tools、CMake。這些只有*編譯*需要；一般執行程式的使用者不必安裝開發工具。
2. 執行 `build-windows.cmd`。
3. 產物在 `dist\RamenListen\ramen-listen.exe`。不附大模型；第一次執行會下載。

若本機 Hy-MT2 無法預載，可在設定查看錯誤提示，`%LOCALAPPDATA%\RamenListen\hy-mt2\llama-server.log` 會保留本機推論服務日誌。

若要從 v0.5 升級，**建議直接建立新資料夾**解壓 v0.6，避免舊 `Cargo.toml`、`setup.rs`、Argos sidecar 殘留。既有 Qwen3-ASR 快取會沿用。

## 驗證狀態與限制

這份是供 Windows 實測的 **v0.6 原始碼**。已做靜態源碼與檔案結構檢查，但產出環境沒有 Rust／Windows GPU，故**尚未完成 cargo build、實際模型載入或最低 VRAM 測試**。尤其 Hy-MT2 GGUF 與特定 llama.cpp Vulkan 版的相容性，須以 Windows 測試回報為準，不應視為保證成功。初次下載需要網路，後續可離線使用。自動下載的來源是外部官方專案，正式發布前還應加入固定 SHA256 校驗與簽章驗證。
