//! TRANSLATE — 只有 Hy-MT2-1.8B 本機 GPU 翻譯。自動下載 GGUF + 官方 Vulkan llama-server。
//! 不使用 Python、uv、Argos、Google、Ollama 或任何遠端翻譯 API。
//! 狀態流程：啟動即準備模型與 llama-server → warm up → 常駐 → 短句 SSE 增量譯文。
use crate::{config, event::Event};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::{Client, Response};
use serde_json::{json, Value};
use std::{fs::{self, File}, io::{BufRead, BufReader, Read, Write}, net::TcpListener,
    path::{Path, PathBuf}, process::{Child, Command, Stdio}, sync::mpsc::{Receiver, Sender},
    thread, time::{Duration, Instant}};
use zip::ZipArchive;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

const SERVER_TAG: &str = "b12625";
const SERVER_ASSET: &str = "llama-b12625-bin-win-vulkan-x64.zip";
const SERVER_URL: &str = "https://github.com/ggml-org/llama.cpp/releases/download/b12625/llama-b12625-bin-win-vulkan-x64.zip";
const GGUF: &str = "Hy-MT2-1.8B-Q4_K_M.gguf";
const GGUF_URL: &str = "https://huggingface.co/tencent/Hy-MT2-1.8B-GGUF/resolve/main/Hy-MT2-1.8B-Q4_K_M.gguf";

#[derive(Clone)]
pub struct Job { pub mark: (u64, u32), pub text: String }

struct Engine { child: Child, endpoint: String, client: Client }
impl Drop for Engine { fn drop(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); } }

pub fn launch(rx: Receiver<Job>, events: Sender<Event>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut engine = match Engine::initialize(&events) {
            Ok(x) => x,
            Err(e) => { let _ = events.send(Event::TranslatorError(format!("Hy-MT2 初始化失敗：{e:#}"))); return; }
        };
        let _ = events.send(Event::TranslatorReady("Hy-MT2 1.8B Q4 • Vulkan GPU".into()));
        for job in rx {
            if job.text.trim().is_empty() { continue; }
            if let Err(e) = engine.stream_translate(&job, &events) {
                let _ = events.send(Event::Translation { mark: job.mark, text: "（本句翻譯失敗）".into() });
                let _ = events.send(Event::Warning(format!("本機翻譯：{e:#}")));
            }
        }
    })
}

impl Engine {
    fn initialize(events: &Sender<Event>) -> Result<Self> {
        #[cfg(not(windows))]
        bail!("v0.6 僅提供 Windows x64 Vulkan 安裝器");
        check_gpu()?;
        let root = config::dir().join("hy-mt2");
        let runtime = root.join(format!("llama-{SERVER_TAG}"));
        let model = root.join("models").join(GGUF);
        fs::create_dir_all(&runtime)?;
        fs::create_dir_all(model.parent().context("模型路徑錯誤")?)?;
        let client = Client::builder().user_agent("RamenListen/0.6")
            .connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(1800)).build()?;
        let bin = match locate_server(&runtime) {
            Some(p) => p,
            None => {
                let archive = root.join(SERVER_ASSET);
                download(&client, SERVER_URL, &archive, events, "首次下載本機 Vulkan 翻譯執行元件")?;
                extract_zip(&archive, &runtime).context("解壓官方 llama.cpp 發行包失敗")?;
                let _ = fs::remove_file(archive);
                locate_server(&runtime).context("找不到 llama-server.exe，官方壓縮包結構可能改變")?
            }
        };
        if !is_amd64_pe(&bin)? { bail!("翻譯執行元件不是 Windows x64 格式：{}", bin.display()); }
        download(&client, GGUF_URL, &model, events, "首次下載 Hy-MT2 1.8B Q4 模型")?;
        // 獨佔隨機 localhost 埠；不連接使用者已有的 Ollama/llama-server，也不暴露網路服務。
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let log_path = root.join("llama-server.log");
        let log = File::create(&log_path)?;
        let mut cmd = Command::new(&bin);
        cmd.arg("--model").arg(&model).args(["--host", "127.0.0.1", "--port"])
            .arg(port.to_string()).args(["-ngl", "99", "-c", "2048", "--parallel", "1"])
            .current_dir(bin.parent().context("找不到執行元件工作目錄")?)
            .stdin(Stdio::null()).stdout(Stdio::from(log.try_clone()?)).stderr(Stdio::from(log));
        #[cfg(windows)] { cmd.creation_flags(0x0800_0000); } // CREATE_NO_WINDOW
        let mut child = cmd.spawn().context("無法啟動本機 llama-server")?;
        let _ = events.send(Event::TranslatorPid(child.id()));
        let endpoint = format!("http://127.0.0.1:{port}");
        let health_client = Client::builder().timeout(Duration::from_secs(2)).build()?;
        let start = Instant::now();
        loop {
            if let Some(exit) = child.try_wait()? { bail!("翻譯服務提前退出：{exit}（可能 GPU 不支援或 VRAM 不足）；日誌：{}", log_path.display()); }
            if let Ok(resp) = health_client.get(format!("{endpoint}/health")).send() {
                if resp.status().is_success() { break; }
            }
            if start.elapsed() > Duration::from_secs(150) {
                let _ = child.kill(); let _ = child.wait();
                bail!("本機翻譯模型預熱逾時，請檢查 GPU 驅動與 VRAM；日誌：{}", log_path.display());
            }
            if start.elapsed().as_secs() % 10 == 0 {
                let _ = events.send(Event::TranslatorStatus(format!("正在預載 Hy-MT2（{} 秒）", start.elapsed().as_secs())));
            }
            thread::sleep(Duration::from_millis(300));
        }
        let client = Client::builder().timeout(Duration::from_secs(90)).build()?;
        Ok(Self { child, endpoint, client })
    }

    fn stream_translate(&mut self, job: &Job, events: &Sender<Event>) -> Result<()> {
        // 官方 Hy-MT2 的單純翻譯提示；不使用 system prompt，不累積會拖慢反應的段落。
        let prompt = format!(
            "Translate the following text into Traditional Chinese. Note that you should only output the translated result without any additional explanation:\n{}",
            job.text.trim()
        );
        let body = json!({"messages": [{"role":"user","content":prompt}],
            "stream":true, "temperature":0.7, "top_p":0.6, "top_k":20,
            "repeat_penalty":1.05, "max_tokens":192 });
        let resp = self.client.post(format!("{}/v1/chat/completions", self.endpoint))
            .json(&body).send()?.error_for_status().context("本機翻譯服務回傳錯誤")?;
        parse_stream(resp, job.mark, events)
    }
}

fn parse_stream(resp: Response, mark: (u64, u32), events: &Sender<Event>) -> Result<()> {
    let mut output = String::new();
    let mut got_done = false;
    let mut last_emit = Instant::now();
    for line in BufReader::new(resp).lines() {
        let line = line?;
        let Some(payload) = line.strip_prefix("data:") else { continue; };
        let payload = payload.trim();
        if payload == "[DONE]" { got_done = true; break; }
        let data: Value = match serde_json::from_str(payload) { Ok(x) => x, Err(_) => continue };
        if let Some(err) = data.get("error") { bail!("本機翻譯錯誤：{err}"); }
        if let Some(delta) = data["choices"][0]["delta"]["content"].as_str() {
            if !delta.is_empty() {
                output.push_str(delta);
                if last_emit.elapsed() >= Duration::from_millis(65) {
                    let _ = events.send(Event::Translation { mark, text: output.clone() });
                    last_emit = Instant::now();
                }
            }
        }
    }
    if output.trim().is_empty() { bail!("Hy-MT2 沒有產生譯文（SSE done={got_done}）"); }
    let _ = events.send(Event::Translation { mark, text: output.trim().to_owned() });
    Ok(())
}

/// First release targets NVIDIA GPUs with at least 8 GiB VRAM; refuse silent CPU/hybrid inference.
fn check_gpu() -> Result<()> {
    #[cfg(not(windows))]
    bail!("v0.6 只提供 Windows NVIDIA GPU 版本");
    let out = Command::new("nvidia-smi.exe")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .output().context("找不到 NVIDIA 驅動工具 nvidia-smi；本版要求 NVIDIA 獨立 GPU")?;
    if !out.status.success() { bail!("無法取得 NVIDIA GPU VRAM 資訊"); }
    let text = String::from_utf8_lossy(&out.stdout);
    let max_mib = text.lines().filter_map(|line| line.trim().parse::<u64>().ok()).max()
        .context("NVIDIA GPU VRAM 資訊格式無法辨識")?;
    if max_mib < 8192 { bail!("GPU VRAM 不足：{max_mib} MiB；RamenListen v0.6 至少需要 8192 MiB"); }
    Ok(())
}

fn download(client: &Client, url: &str, path: &Path, events: &Sender<Event>, label: &str) -> Result<()> {
    if path.metadata().map(|m| m.len() > 1_000_000).unwrap_or(false) { return Ok(()); }
    let _ = events.send(Event::TranslatorStatus(label.to_owned()));
    let tmp = path.with_extension("download");
    let mut resp = client.get(url).send()?.error_for_status()?;
    let size = resp.content_length();
    let result: Result<()> = (|| {
        let mut out = File::create(&tmp)?;
        let mut bytes = 0u64;
        let mut last = 0u64;
        let mut buf = [0u8; 256*1024];
        loop {
            let n = resp.read(&mut buf)?;
            if n == 0 { break; }
            out.write_all(&buf[..n])?;
            bytes += n as u64;
            if bytes.saturating_sub(last) >= 12*1024*1024 {
                last = bytes;
                let suffix = size.map(|s| format!(" / {:.0} MiB", s as f64 / 1_048_576.0)).unwrap_or_default();
                let _ = events.send(Event::TranslatorStatus(format!("{label}：{:.0} MiB{suffix}", bytes as f64 / 1_048_576.0)));
            }
        }
        out.flush()?;
        if let Some(s) = size { if bytes != s { bail!("下載大小不符：{bytes} vs {s}"); } }
        if bytes < 1_000_000 { bail!("下載內容異常過小：{bytes} bytes"); }
        Ok(())
    })();
    if let Err(e) = result { let _ = fs::remove_file(&tmp); return Err(e); }
    if path.exists() { fs::remove_file(path)?; }
    fs::rename(&tmp, path)?;
    Ok(())
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<()> {
    let mut zip = ZipArchive::new(File::open(archive)?)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        // 所有成員都只允許落在隔離的安裝資料夾內。
        let Some(enclosed) = entry.enclosed_name() else { bail!("壓縮檔內含不安全路徑"); };
        let dst = destination.join(enclosed);
        if entry.is_dir() { fs::create_dir_all(dst)?; continue; }
        if let Some(parent) = dst.parent() { fs::create_dir_all(parent)?; }
        let mut output = File::create(dst)?;
        std::io::copy(&mut entry, &mut output)?;
    }
    Ok(())
}
fn locate_server(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() { if let Some(p) = locate_server(&path) { return Some(p); } }
        if path.file_name().is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("llama-server.exe")) {
            return Some(path);
        }
    }
    None
}
fn is_amd64_pe(path: &Path) -> Result<bool> {
    let mut f = File::open(path)?;
    let mut data = [0u8; 4096];
    let n = f.read(&mut data)?;
    if n < 0x40 || &data[..2] != b"MZ" { return Ok(false); }
    let pos = u32::from_le_bytes(data[0x3c..0x40].try_into()?) as usize;
    if pos + 6 > n || &data[pos..pos+4] != b"PE\0\0" { return Ok(false); }
    Ok(u16::from_le_bytes([data[pos+4], data[pos+5]]) == 0x8664)
}

#[cfg(test)]
mod tests {
    #[test] fn official_assets_are_pinned() {
        assert!(super::SERVER_URL.contains(super::SERVER_TAG));
        assert!(super::GGUF_URL.ends_with(super::GGUF));
    }
}
