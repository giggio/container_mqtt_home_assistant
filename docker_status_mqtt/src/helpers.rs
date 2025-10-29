use std::time::Duration;

use chrono_humanize::{Accuracy, HumanTime, Tense};

pub fn slugify(name: impl Into<String>) -> String {
    guls::slugify(name.into()).replace('-', "_")
}

pub fn pretty_format(duration: Duration) -> String {
    let chrono_duration = chrono::Duration::from_std(duration).unwrap();
    HumanTime::from(chrono_duration).to_text_en(Accuracy::Precise, Tense::Present)
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
}
