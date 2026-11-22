//! CAPTION — 最近三句字幕的唯一真實資料來源；晚到譯文以 phrase+revision 回填。
use std::{collections::VecDeque, time::Instant};
#[derive(Clone)]
pub struct Line {
    pub phrase: u64,
    pub mark: (u64, u32),
    pub original: String,
    pub translated: String,
    pub demoted_at: Option<Instant>,
}
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
        while self.lines.len() > 3 { self.lines.pop_front(); }
    }
    pub fn translation(&mut self, mark: (u64, u32), value: String) -> bool {
        if let Some(x) = self.lines.iter_mut().find(|x| x.mark == mark) {
            x.translated = value; true
        } else { false }
    }
    pub fn recent(&self) -> Vec<Line> { self.lines.iter().cloned().collect() }
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
    }
}
