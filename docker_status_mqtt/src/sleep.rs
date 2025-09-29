use std::time::Duration;

use chrono_humanize::{Accuracy, HumanTime, Tense};
use tokio::time;

pub struct SleepResult {
    pub completed: bool,
}
pub async fn sleep_cancellable(duration: Duration) -> SleepResult {
    tokio::select! {
        _ = time::sleep(duration) => {
            if log_enabled!(log::Level::Info) {
                let chrono_duration = chrono::Duration::from_std(duration).unwrap();
                let duration_human = HumanTime::from(chrono_duration).to_text_en(Accuracy::Precise, Tense::Present);
                trace!("Waited {duration_human} before continuing...");
            }
            SleepResult { completed: true }
        }
        _ = tokio::signal::ctrl_c() => {
            SleepResult { completed: false }
        }
    }
}
