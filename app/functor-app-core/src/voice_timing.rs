//! When each stage of a spoken request happened, from the headset's clip to
//! the agent's answer. The shell stamps the clip and its transcription, the
//! core the rest; the task keeps its latest utterance's, shown as one line,
//! and the shell logs it once its run ends.

use serde::{Deserialize, Serialize};

/// Stage times: the shell's wall clock, ms since the Unix epoch (`None`:
/// not reached yet).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(default)]
pub struct VoiceTiming {
    /// The headset's number for the utterance.
    pub utterance: u64,
    /// The clip's `Start` arrived: the button went down.
    pub start: u64,
    pub first_chunk: Option<u64>,
    /// Its `End` arrived: the clip is whole.
    pub end: u64,
    /// The clip's length.
    pub audio_ms: u64,
    pub transcribe_start: u64,
    pub transcribe_end: u64,
    /// Of transcribing, the time spent loading the model (`None`: loaded).
    pub model_load_ms: Option<u64>,
    /// The core placed it: a task started, steered, queued or added to.
    pub routed: Option<u64>,
    /// The run it started (its user message's index), once it has one.
    pub run: Option<u64>,
    pub run_started: Option<u64>,
    /// The tool server listened and the agent process was started.
    pub agent_spawned: Option<u64>,
    /// The agent's first event (its start).
    pub agent_event: Option<u64>,
    pub first_text: Option<u64>,
    pub finished: Option<u64>,
    /// [`VoiceTiming::summary`] as of the last stage, for the task page.
    pub line: String,
}

impl VoiceTiming {
    /// When the user let go, as near as the desktop can tell: the clip's
    /// end if it streamed in step, else its start plus its length (the
    /// headset streams while the button is held).
    fn released(&self) -> u64 {
        self.end.min(self.start + self.audio_ms)
    }

    /// Each reached stage and its time since the release, in ms.
    pub fn stages(&self) -> Vec<(&'static str, u64)> {
        let at = self.released();
        let since = |t: u64| t.saturating_sub(at);
        let mut stages = vec![("heard", since(self.end))];
        if self.transcribe_end > 0 {
            stages.push(("transcribed", since(self.transcribe_end)));
        }
        let rest = [
            ("agent started", self.agent_event),
            ("first reply", self.first_text),
            ("done", self.finished),
        ];
        stages.extend(rest.into_iter().filter_map(|(name, t)| Some((name, since(t?)))));
        stages
    }

    /// "heard 0.1 s · transcribed 1.1 s (model load 0.4 s) · agent started
    /// 3.0 s · first reply 6.2 s · done 32.0 s", each since the release.
    pub fn summary(&self) -> String {
        let secs = |ms: u64| format!("{:.1} s", ms as f64 / 1000.0);
        let parts: Vec<String> = (self.stages().into_iter())
            .map(|(name, ms)| match (name, self.model_load_ms) {
                ("transcribed", Some(load)) => {
                    format!("{name} {} (model load {})", secs(ms), secs(load))
                }
                _ => format!("{name} {}", secs(ms)),
            })
            .collect();
        parts.join(" · ")
    }

    /// Sets `stage` to `now` unless already set (0: an unknown clock);
    /// whether it changed.
    pub fn reach(&mut self, stage: fn(&mut VoiceTiming) -> &mut Option<u64>, now: u64) -> bool {
        let slot = stage(self);
        if slot.is_some() || now == 0 {
            return false;
        }
        *slot = Some(now);
        self.line = self.summary();
        true
    }

    /// The log line: the stages since the release and the raw times.
    pub fn log_line(&self, task: u64) -> String {
        let stages: serde_json::Map<String, serde_json::Value> = (self.stages().into_iter())
            .map(|(name, ms)| (name.replace(' ', "_") + "_ms", ms.into()))
            .collect();
        let line = serde_json::json!({ "task": task, "since_release": stages, "timing": self });
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heard() -> VoiceTiming {
        let mut t = VoiceTiming {
            utterance: 7,
            start: 10_000,
            first_chunk: Some(10_050),
            end: 13_000,
            audio_ms: 2_880,
            transcribe_start: 13_001,
            transcribe_end: 14_100,
            model_load_ms: Some(400),
            ..VoiceTiming::default()
        };
        t.line = t.summary();
        t
    }

    #[test]
    fn stages_count_from_the_release() {
        let mut t = heard();
        // Released at 12_880 (start + length), whole at 13_000.
        assert_eq!(t.line, "heard 0.1 s · transcribed 1.2 s (model load 0.4 s)");
        assert!(t.reach(|t| &mut t.routed, 14_120));
        assert!(t.reach(|t| &mut t.agent_event, 16_000));
        assert!(t.reach(|t| &mut t.first_text, 19_000));
        assert!(t.reach(|t| &mut t.finished, 45_000));
        assert_eq!(
            t.line,
            "heard 0.1 s · transcribed 1.2 s (model load 0.4 s) · agent started 3.1 s · \
first reply 6.1 s · done 32.1 s"
        );
    }

    #[test]
    fn a_stage_is_stamped_once_and_never_by_an_unknown_clock() {
        let mut t = heard();
        assert!(!t.reach(|t| &mut t.finished, 0));
        assert!(t.reach(|t| &mut t.finished, 20_000));
        assert!(!t.reach(|t| &mut t.finished, 30_000));
        assert_eq!(t.finished, Some(20_000));
    }

    #[test]
    fn a_clip_that_lagged_counts_from_its_length_and_one_in_step_from_its_end() {
        // In step (the start arrived late): the release is the end.
        let t = VoiceTiming { start: 1_000, end: 2_000, audio_ms: 2_500, ..VoiceTiming::default() };
        assert_eq!(t.stages(), vec![("heard", 0)]);
        // A warm model says no load.
        let t = VoiceTiming { transcribe_end: 2_300, ..t };
        assert_eq!(t.summary(), "heard 0.0 s · transcribed 0.3 s");
    }

    #[test]
    fn the_log_line_has_the_stages_and_the_times() {
        let line: serde_json::Value = serde_json::from_str(&heard().log_line(3)).unwrap();
        assert_eq!(line["task"], 3);
        assert_eq!(line["since_release"]["transcribed_ms"], 1_220);
        assert_eq!(line["timing"]["utterance"], 7);
    }
}
