use chrono_humanize::{Accuracy, HumanTime, Tense};
use std::time::Duration;

pub fn pretty_format(duration: Duration) -> String {
    let chrono_duration = chrono::Duration::from_std(duration).unwrap();
    HumanTime::from(chrono_duration).to_text_en(Accuracy::Precise, Tense::Present)
}
