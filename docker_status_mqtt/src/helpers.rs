use chrono_humanize::{Accuracy, HumanTime, Tense};

pub trait AsyncMap {
    async fn async_map<F, Fut, T, U>(self, f: F) -> Vec<U>
    where
        F: Fn(T) -> Fut,
        Fut: std::future::Future<Output = U>,
        Self: Sized + IntoIterator<Item = T>,
    {
        futures::future::join_all(self.into_iter().map(f)).await
    }

    #[allow(dead_code)] // todo: use this somewhere or remove it
    async fn async_foreach<F, Fut, T, U>(self, f: F)
    where
        F: Fn(T) -> Fut,
        Fut: std::future::Future<Output = U>,
        Self: Sized + IntoIterator<Item = T>,
    {
        futures::future::join_all(self.into_iter().map(f)).await;
    }
}

impl<T, I> AsyncMap for I where I: IntoIterator<Item = T> {}

pub fn slugify(name: impl Into<String>) -> String {
    guls::slugify(name.into()).replace('-', "_")
}

pub fn pretty_format(duration: impl Into<DurationConvert>) -> String {
    let chrono_duration: chrono::Duration = duration.into().into();
    HumanTime::from(chrono_duration).to_text_en(Accuracy::Precise, Tense::Present)
}

pub enum DurationConvert {
    ChronoDuration(chrono::Duration),
    StdDuration(std::time::Duration),
}

impl From<chrono::Duration> for DurationConvert {
    fn from(value: chrono::Duration) -> Self {
        DurationConvert::ChronoDuration(value)
    }
}

impl From<std::time::Duration> for DurationConvert {
    fn from(value: std::time::Duration) -> Self {
        DurationConvert::StdDuration(value)
    }
}

impl From<DurationConvert> for std::time::Duration {
    fn from(duration_convert: DurationConvert) -> Self {
        match duration_convert {
            DurationConvert::ChronoDuration(d) => d.to_std().unwrap(),
            DurationConvert::StdDuration(d) => d,
        }
    }
}
impl From<DurationConvert> for chrono::Duration {
    fn from(duration_convert: DurationConvert) -> Self {
        match duration_convert {
            DurationConvert::ChronoDuration(d) => d,
            DurationConvert::StdDuration(d) => chrono::Duration::from_std(d).unwrap(),
        }
    }
}

pub fn hostname() -> String {
    #[cfg(test)]
    {
        "MY_HOSTNAME".into()
    }
    #[cfg(not(test))]
    {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(rustix::system::uname().nodename().to_bytes().to_vec())
            .to_string_lossy()
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_slugify_comprehensive() {
        // Basic cases
        assert_eq!(slugify("Hello World"), "hello_world");
        assert_eq!(slugify("test"), "test");
        assert_eq!(slugify("TEST"), "test");

        // Special characters
        assert_eq!(slugify("Test@Device#123!"), "test_device_123");
        assert_eq!(slugify("user+name"), "user_name");
        assert_eq!(slugify("file.name.txt"), "file_name_txt");

        // Multiple spaces and dashes
        assert_eq!(slugify("Multiple   Spaces"), "multiple_spaces");
        assert_eq!(slugify("dash-separated-words"), "dash_separated_words");
        assert_eq!(slugify("mixed-dash and_space"), "mixed_dash_and_space");

        // Unicode and special cases
        assert_eq!(slugify("Café"), "cafe");
        assert_eq!(slugify("naïve"), "naive");

        // Edge cases
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("123"), "123");
        assert_eq!(slugify("_leading"), "leading");
        assert_eq!(slugify("trailing_"), "trailing");

        // preserving underscores
        assert_eq!(slugify("already_good_format"), "already_good_format");
        assert_eq!(slugify("has_under_scores"), "has_under_scores");
        assert_eq!(slugify("_leading_underscore"), "leading_underscore");
        assert_eq!(slugify("trailing_underscore_"), "trailing_underscore");

        // replacing dashes with underscores
        assert_eq!(slugify("with-dashes"), "with_dashes");
        assert_eq!(slugify("multiple-dash-case"), "multiple_dash_case");
        assert_eq!(slugify("-leading-dash"), "leading_dash");
    }

    #[test]
    fn test_slugify_mqtt_node_ids() {
        assert_eq!(slugify("mqtt_docker"), "mqtt_docker");
        assert_eq!(slugify("MQTT Docker"), "mqtt_docker");
        assert_eq!(slugify("mqtt-docker-node"), "mqtt_docker_node");
        assert_eq!(slugify("My Home Server"), "my_home_server");
    }

    #[test]
    fn test_pretty_format_std_duration_seconds() {
        let duration = std::time::Duration::from_secs(45);
        let result = pretty_format(duration);
        assert_eq!(result, "45 seconds");
    }

    #[test]
    fn test_pretty_format_std_duration_minutes() {
        let duration = std::time::Duration::from_secs(120);
        let result = pretty_format(duration);
        assert_eq!(result, "2 minutes");
    }

    #[test]
    fn test_pretty_format_std_duration_hours() {
        let duration = std::time::Duration::from_secs(3661);
        let result = pretty_format(duration);
        assert_eq!(result, "1 hour, 1 minute and 1 second");
    }

    #[test]
    fn test_pretty_format_std_duration_days() {
        let duration = std::time::Duration::from_secs(86400 + 3600 + 60 + 1);
        let result = pretty_format(duration);
        assert_eq!(result, "1 day, 1 hour, 1 minute and 1 second");
    }

    #[test]
    fn test_pretty_format_chrono_duration_seconds() {
        let duration = chrono::Duration::seconds(30);
        let result = pretty_format(duration);
        assert_eq!(result, "30 seconds");
    }

    #[test]
    fn test_pretty_format_chrono_duration_minutes() {
        let duration = chrono::Duration::minutes(5);
        let result = pretty_format(duration);
        assert_eq!(result, "5 minutes");
    }

    #[test]
    fn test_pretty_format_chrono_duration_hours() {
        let duration = chrono::Duration::hours(2);
        let result = pretty_format(duration);
        assert_eq!(result, "2 hours");
    }

    #[test]
    fn test_pretty_format_chrono_duration_days() {
        let duration = chrono::Duration::days(3);
        let result = pretty_format(duration);
        assert_eq!(result, "3 days");
    }

    #[test]
    fn test_pretty_format_chrono_duration_mixed() {
        let duration = chrono::Duration::days(1) + chrono::Duration::hours(2) + chrono::Duration::minutes(30);
        let result = pretty_format(duration);
        assert_eq!(result, "1 day, 2 hours and 30 minutes");
    }

    #[test]
    fn test_pretty_format_zero_duration() {
        let duration = std::time::Duration::from_secs(0);
        let result = pretty_format(duration);
        assert_eq!(result, "0 seconds");
    }
}
