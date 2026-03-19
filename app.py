import tkinter as tk
from tkinter import ttk, scrolledtext, filedialog, Menu, messagebox
import threading
import os
import sys
import json
import queue
import time
import numpy as np
import pyaudio
import requests 
from vosk import Model, KaldiRecognizer
from deep_translator import GoogleTranslator

# ================= 系統核心參數 =================
BASE_MODEL_DIR = "model" 

VOLUME_THRESHOLD = 400 
# 註解掉原本寫死的停頓秒數，改由介面動態控制
# PAUSE_THRESHOLD = 0.3  

# ================= 選單與語言對應設定 =================
SPEECH_MODELS = {
    "英文 (快速)": "en_fast",
    "英文 (品質佳)": "en_quality",
    "日文 (快速)": "ja_fast",
    "日文 (品質佳)": "ja_quality"
}

LANGUAGE_OPTIONS = {
    "繁體中文 (台灣)": {"google": "zh-TW", "ai": "Traditional Chinese (Taiwan)"},
    "简体中文": {"google": "zh-CN", "ai": "Simplified Chinese"},
    "日本語 (Japanese)": {"google": "ja", "ai": "Japanese"},
    "English (英文)": {"google": "en", "ai": "English"},
    "한국어 (Korean)": {"google": "ko", "ai": "Korean"}
}
# ================================================

class UltimateCourseApp:
    def __init__(self):
        self.root = tk.Tk()
        self.root.title("AI 課程翻譯工作台 (自訂API + 停頓微調版)")
        
        self.available_models = self.scan_models()
        if not self.available_models:
            messagebox.showerror("錯誤", f"找不到語音模型！\n請確認 '{BASE_MODEL_DIR}' 內有正確的英文資料夾 (如 en_fast)。")
            sys.exit()

        self.target_lang_var = tk.StringVar(value="繁體中文 (台灣)")
        self.speech_model_var = tk.StringVar(value=self.available_models[0])
        self.api_url_var = tk.StringVar(value="http://127.0.0.1:11434/v1/chat/completions")
        self.model_name_var = tk.StringVar(value="gemma3:12b")
        
        # 新增變數：停頓秒數設定 (預設 0.3 秒)
        self.pause_threshold_var = tk.StringVar(value="0.3")
        
        self.is_advanced_shown = False
        
        self.setup_window()
        
        self.is_running = True
        self.audio_queue = queue.Queue()
        self.last_trans_time = 0 
        self.is_loading_model = False
        self.pending_vosk_model = None
        
        initial_folder = self.get_folder_name(self.available_models[0])
        initial_model_path = os.path.join(BASE_MODEL_DIR, initial_folder)
        self.vosk_model = Model(initial_model_path)
        
        threading.Thread(target=self.vosk_loop, daemon=True).start()

    def scan_models(self):
        if not os.path.exists(BASE_MODEL_DIR): return []
        available = []
        for display_name, folder_name in SPEECH_MODELS.items():
            if os.path.isdir(os.path.join(BASE_MODEL_DIR, folder_name)):
                available.append(display_name)
        for d in os.listdir(BASE_MODEL_DIR):
            if os.path.isdir(os.path.join(BASE_MODEL_DIR, d)):
                if d not in SPEECH_MODELS.values() and d not in available:
                    available.append(d)
        return available

    def get_folder_name(self, display_name):
        for name, folder in SPEECH_MODELS.items():
            if name == display_name:
                return folder
        return display_name 

    def setup_window(self):
        sw, sh = self.root.winfo_screenwidth(), self.root.winfo_screenheight()
        win_w, win_h = int(sw * 0.85), 320 
        self.root.geometry(f"{win_w}x{win_h}+{int(sw*0.075)}+{sh-380}")
        self.root.overrideredirect(True)
        self.root.attributes("-topmost", True)
        self.root.attributes("-alpha", 0.9)
        self.root.config(bg='#1c1c1c')

        self.paned = tk.PanedWindow(self.root, orient=tk.HORIZONTAL, bg='#333333', sashwidth=4)
        self.paned.pack(fill='both', expand=True)

        # 左欄
        self.left_frame = tk.Frame(self.paned, bg='#1c1c1c')
        self.paned.add(self.left_frame, width=win_w*0.55)
        self.en_label = tk.Label(self.left_frame, text="System Ready...", font=("Consolas", 16, "italic"), fg="#00FF00", bg="#1c1c1c", anchor='nw', justify='left', wraplength=int(win_w*0.5))
        self.en_label.pack(fill='x', padx=25, pady=(25, 0))
        self.zh_label = tk.Label(self.left_frame, text="等待語音...", font=("Microsoft JhengHei", 24, "bold"), fg="#FFFFFF", bg="#1c1c1c", anchor='nw', justify='left', wraplength=int(win_w*0.5))
        self.zh_label.pack(fill='x', padx=25, pady=10)

        # 右欄
        self.right_frame = tk.Frame(self.paned, bg='#252525')
        self.paned.add(self.right_frame, width=win_w*0.45)
        
        ctrl_frame = tk.Frame(self.right_frame, bg='#252525')
        ctrl_frame.pack(fill='x', padx=5, pady=5)
        
        # 1. 常駐控制 (語音模型、語言)
        row1 = tk.Frame(ctrl_frame, bg='#252525')
        row1.pack(fill='x', pady=2)
        tk.Label(row1, text="🎙️ 聽寫模型:", font=("Microsoft JhengHei", 10), fg="#AAAAAA", bg='#252525', width=10, anchor='e').pack(side='left')
        self.model_combo = ttk.Combobox(row1, textvariable=self.speech_model_var, values=self.available_models, state="readonly", width=18)
        self.model_combo.pack(side='left', fill='x', expand=True, padx=5)
        self.model_combo.bind("<<ComboboxSelected>>", self.on_model_change)

        row2 = tk.Frame(ctrl_frame, bg='#252525')
        row2.pack(fill='x', pady=2)
        tk.Label(row2, text="🌐 輸出語言:", font=("Microsoft JhengHei", 10), fg="#AAAAAA", bg='#252525', width=10, anchor='e').pack(side='left')
        self.lang_combo = ttk.Combobox(row2, textvariable=self.target_lang_var, values=list(LANGUAGE_OPTIONS.keys()), state="readonly", width=18)
        self.lang_combo.pack(side='left', fill='x', expand=True, padx=5)

        # 2. 展開/收合按鈕
        self.toggle_btn = tk.Button(ctrl_frame, text="🔽 展開進階設定 (API / 停頓)", font=("Microsoft JhengHei", 9), bg="#444444", fg="#FFFFFF", relief="flat", command=self.toggle_advanced)
        self.toggle_btn.pack(fill='x', pady=(5, 0), padx=5)

        # 3. 進階設定區域 (預設隱藏)
        self.advanced_frame = tk.Frame(ctrl_frame, bg='#252525')
        
        row_api = tk.Frame(self.advanced_frame, bg='#252525')
        row_api.pack(fill='x', pady=2)
        tk.Label(row_api, text="🔗 API 網址:", font=("Microsoft JhengHei", 10), fg="#AAAAAA", bg='#252525', width=10, anchor='e').pack(side='left')
        self.api_entry = ttk.Entry(row_api, textvariable=self.api_url_var)
        self.api_entry.pack(side='left', fill='x', expand=True, padx=5)

        row_model_name = tk.Frame(self.advanced_frame, bg='#252525')
        row_model_name.pack(fill='x', pady=2)
        tk.Label(row_model_name, text="🤖 AI 模型:", font=("Microsoft JhengHei", 10), fg="#AAAAAA", bg='#252525', width=10, anchor='e').pack(side='left')
        self.model_name_entry = ttk.Entry(row_model_name, textvariable=self.model_name_var)
        self.model_name_entry.pack(side='left', fill='x', expand=True, padx=5)

        # 新增：停頓秒數設定
        row_pause = tk.Frame(self.advanced_frame, bg='#252525')
        row_pause.pack(fill='x', pady=2)
        tk.Label(row_pause, text="⏱️ 停頓秒數:", font=("Microsoft JhengHei", 10), fg="#AAAAAA", bg='#252525', width=10, anchor='e').pack(side='left')
        # 使用 Spinbox 讓使用者方便增減數字
        self.pause_spin = ttk.Spinbox(row_pause, textvariable=self.pause_threshold_var, from_=0.1, to=3.0, increment=0.1)
        self.pause_spin.pack(side='left', fill='x', expand=True, padx=5)

        # Log 區域
        self.log_area = scrolledtext.ScrolledText(self.right_frame, font=("Microsoft JhengHei", 11), bg="#101010", fg="#CCCCCC", state='disabled', borderwidth=0)
        self.log_area.pack(fill='both', expand=True, padx=5, pady=(5, 5))

        self.menu = Menu(self.root, tearoff=0)
        self.menu.add_command(label="💾 儲存今日課程檔", command=self.save_log)
        self.menu.add_command(label="🧹 清空紀錄", command=self.clear_log)
        self.menu.add_separator()
        self.menu.add_command(label="❌ 關閉程式", command=self.on_close)
        self.root.bind("<Button-3>", lambda e: self.menu.post(e.x_root, e.y_root))

    def toggle_advanced(self):
        self.is_advanced_shown = not self.is_advanced_shown
        if self.is_advanced_shown:
            self.toggle_btn.config(text="🔼 收合進階設定 (API / 停頓)")
            self.advanced_frame.pack(fill='x', pady=(2, 0)) 
        else:
            self.toggle_btn.config(text="🔽 展開進階設定 (API / 停頓)")
            self.advanced_frame.pack_forget()

    def get_pause_threshold(self):
        """安全取得停頓秒數，若輸入無效則回傳預設值 0.3"""
        try:
            val = float(self.pause_threshold_var.get())
            return max(0.1, val) # 最少 0.1 秒，防止設定為 0 造成無限觸發
        except ValueError:
            return 0.3

    def on_model_change(self, event=None):
        selected_display = self.speech_model_var.get()
        self.update_ui_en(f"[系統] 正在載入模型: {selected_display} ...")
        self.update_ui_zh("請稍候，切換中...")
        threading.Thread(target=self._load_new_model_task, args=(selected_display,), daemon=True).start()

    def _load_new_model_task(self, selected_display):
        self.is_loading_model = True
        try:
            folder_name = self.get_folder_name(selected_display)
            path = os.path.join(BASE_MODEL_DIR, folder_name)
            new_model = Model(path)
            self.pending_vosk_model = new_model
        except Exception as e:
            self.update_ui_en(f"[模型載入失敗] {e}")
            self.update_ui_zh("請確認模型資料夾是否完整")
        finally:
            self.is_loading_model = False

    def get_current_langs(self):
        choice = self.target_lang_var.get()
        return LANGUAGE_OPTIONS.get(choice, LANGUAGE_OPTIONS["繁體中文 (台灣)"])

    def audio_callback(self, in_data, frame_count, time_info, status):
        self.audio_queue.put(in_data)
        return (None, pyaudio.paContinue)

    def vosk_loop(self):
        p = pyaudio.PyAudio()
        stream = p.open(format=pyaudio.paInt16, channels=1, rate=16000, input=True, frames_per_buffer=1000, stream_callback=self.audio_callback)
        rec = KaldiRecognizer(self.vosk_model, 16000)
        silence_start = None
        has_sent_final = False

        while self.is_running:
            if self.pending_vosk_model:
                self.vosk_model = self.pending_vosk_model
                rec = KaldiRecognizer(self.vosk_model, 16000)
                self.pending_vosk_model = None
                self.audio_queue.queue.clear()
                self.update_ui_en("System Ready...")
                self.update_ui_zh("語音模型切換完成，等待語音...")
                continue

            if self.is_loading_model:
                time.sleep(0.1)
                continue

            data = self.audio_queue.get()
            audio_np = np.frombuffer(data, dtype=np.int16)
            rms = np.sqrt(np.mean(audio_np.astype(np.float64)**2))

            if rec.AcceptWaveform(data):
                res = json.loads(rec.Result())
                text = res.get("text", "").strip()
                if text: self.trigger_ai_task(text)
                has_sent_final = True
            else:
                partial = json.loads(rec.PartialResult())
                p_text = partial.get("partial", "").strip()
                if p_text:
                    self.update_ui_en(p_text)
                    if time.time() - self.last_trans_time > 0.8:
                        self.trigger_google_fast(p_text)
                        self.last_trans_time = time.time()
                
                if rms < VOLUME_THRESHOLD:
                    if silence_start is None: silence_start = time.time()
                    # 這裡動態讀取介面上的停頓秒數
                    if (time.time() - silence_start) >= self.get_pause_threshold() and not has_sent_final:
                        if p_text:
                            self.trigger_ai_task(p_text)
                            has_sent_final = True
                            rec.Reset()
                else:
                    silence_start, has_sent_final = None, False

        stream.stop_stream()
        stream.close()
        p.terminate()

    def trigger_google_fast(self, text):
        threading.Thread(target=self._google_fast_task, args=(text,), daemon=True).start()

    def _google_fast_task(self, text):
        try:
            target_code = self.get_current_langs()["google"]
            res = GoogleTranslator(source='auto', target=target_code).translate(text)
            self.update_ui_zh(res)
        except: pass

    def trigger_ai_task(self, text):
        threading.Thread(target=self._ai_with_fallback, args=(text,), daemon=True).start()

    def _ai_with_fallback(self, text):
        current_api_url = self.api_url_var.get().strip()
        current_model_name = self.model_name_var.get().strip()
        
        ai_target = self.get_current_langs()["ai"]
        payload = {
            "model": current_model_name,
            "messages": [
                {"role": "system", "content": f"You are a professional translator. Translate to natural {ai_target}. Only output the translation."},
                {"role": "user", "content": text}
            ],
            "temperature": 0.3, "stream": False
        }
        try:
            response = requests.post(current_api_url, json=payload, timeout=12)
            if response.status_code == 200:
                zh_res = response.json()['choices'][0]['message']['content'].strip()
                self.append_log(text, zh_res)
            else:
                raise Exception(f"Server Error {response.status_code}")
        except Exception as e:
            try:
                target_code = self.get_current_langs()["google"]
                fallback_zh = GoogleTranslator(source='auto', target=target_code).translate(text)
                self.append_log(text, f"{fallback_zh} (Google 備援: {e})")
            except:
                self.append_log(text, "[網路或 API 錯誤 - 僅存原文]")

    def update_ui_en(self, text): self.root.after(0, lambda: self.en_label.config(text=text))
    def update_ui_zh(self, text): self.root.after(0, lambda: self.zh_label.config(text=text))

    def append_log(self, en, zh):
        def _append():
            self.log_area.config(state='normal')
            ts = time.strftime("%H:%M:%S")
            self.log_area.insert(tk.END, f"[{ts}]\nEN: {en}\nZH: {zh}\n\n")
            self.log_area.see(tk.END)
            self.log_area.config(state='disabled')
        self.root.after(0, _append)

    def save_log(self):
        content = self.log_area.get("1.0", tk.END).strip()
        if not content: return
        path = filedialog.asksaveasfilename(defaultextension=".txt", filetypes=[("Text", "*.txt")])
        if path:
            with open(path, "w", encoding="utf-8") as f: f.write(content)
            messagebox.showinfo("成功", "課程紀錄已儲存")

    def clear_log(self):
        self.log_area.config(state='normal')
        self.log_area.delete("1.0", tk.END)
        self.log_area.config(state='disabled')

    def on_close(self):
        self.is_running = False
        self.root.destroy()
        sys.exit()

if __name__ == "__main__":
    app = UltimateCourseApp()
    app.root.mainloop()
