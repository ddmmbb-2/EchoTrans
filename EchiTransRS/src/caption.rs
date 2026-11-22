//! CAPTION — 本次字幕保留最多 500 句；畫面優先顯示最新約三句，可向上捲回歷史。
//! 晚到譯文僅依 phrase+revision 精準回填，不混寫原始逐字稿；不額外建立持久譯文。
use std::{collections::VecDeque, time::Instant};
#[derive(Clone)]
pub struct Line {
    pub phrase: u64,
    pub mark: (u64, u32),
    pub original: String,
    pub translated: String,
    pub demoted_at: Option<Instant>,
}
pub const MAX_CAPTION_HISTORY: usize = 500; // 有界記憶體，原文的完整歷史仍由 main.rs 落盤。
#[derive(Default)]
pub struct History { lines: VecDeque<Line> }
impl History {
    pub fn upsert(&mut self, mark: (u64, u32), original: String, hint: &str) {
        if let Some(x) = self.lines.iter_mut().find(|x| x.phrase == mark.0) {
            if mark >= x.mark {
                if mark != x.mark || x.original != original { x.translated = hint.to_owned(); }
                x.mark = mark; x.original = original;
            }
            return;
        }
        if self.lines.back().is_some_and(|x| mark.0 < x.phrase) { return; }
        let t = Instant::now();
        for x in &mut self.lines { x.demoted_at = Some(t); }
        self.lines.push_back(Line { phrase: mark.0, mark, original, translated: hint.into(), demoted_at: None });
        while self.lines.len() > MAX_CAPTION_HISTORY { self.lines.pop_front(); }
    }
    pub fn translation(&mut self, mark: (u64, u32), value: String) -> bool {
        if let Some(x) = self.lines.iter_mut().find(|x| x.mark == mark) {
            x.translated = value; true
        } else { false }
    }
    /// 快速待機判斷不應在每一幀複製數百句。
    pub fn is_empty(&self) -> bool { self.lines.is_empty() }
    /// UI 滾動區使用本次全部保留的句子。
    pub fn all(&self) -> Vec<Line> { self.lines.iter().cloned().collect() }
    /// 最近三句僅作視覺聚焦與相容呼叫，不裁掉之前的字幕。
    pub fn recent(&self) -> Vec<Line> { self.lines.iter().rev().take(3).cloned().collect::<Vec<_>>().into_iter().rev().collect() }
    pub fn clear(&mut self) { self.lines.clear(); }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn preserve_last_three_and_final_revision() {
        let mut h=History::default();
        h.upsert((1, 1), "one".into(), "waiting");
        h.upsert((1, u32::MAX), "one final".into(), "translating");
        assert!(!h.translation((1, 1), "old".into()));
        assert!(h.translation((1, u32::MAX), "一".into()));
        for n in 2..=4 { h.upsert((n, u32::MAX), n.to_string(), "wait"); }
        let latest=h.recent();
        assert_eq!(latest.len(), 3);
        assert_eq!(latest[0].phrase, 2);
        assert_eq!(latest[2].phrase, 4);
        assert_eq!(h.all().len(), 4, "可以回捲查看第一句");
        assert!(h.translation((1, u32::MAX), "晚到的譯文".into()), "舊句譯文仍能回填");
    }
    #[test] fn history_is_bounded_without_discarding_previous_three() {
        let mut h = History::default();
        for n in 1..=(MAX_CAPTION_HISTORY as u64 + 3) {
            h.upsert((n, u32::MAX), n.to_string(), "...");
        }
        let all = h.all();
        assert_eq!(all.len(), MAX_CAPTION_HISTORY);
        assert_eq!(all.first().unwrap().phrase, 4);
        assert_eq!(h.recent().len(), 3);
        h.clear();
        assert!(h.is_empty());
    }
}
