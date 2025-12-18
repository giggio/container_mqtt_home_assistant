use async_trait::async_trait;
use cmha_proc_macros::EntityDetailsGetter;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::fmt::Debug;

use crate::{
    cancellation_token::CancellationToken,
    devices::{Entity, EntityDetails, EntityDetailsGetter, Error, device::Device},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(EntityDetailsGetter, Debug)]
pub struct Text {
    pub details: EntityDetails,
    pub command_topic: String,
    pub mode: TextMode,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TextMode {
    Text,
    Password,
}

impl Text {
    pub fn new(device_identifier: String, name: String, icon: String) -> Self {
        Self::new_with_details(EntityDetails::new(device_identifier, name, icon))
    }
    pub fn new_with_details(details: EntityDetails) -> Self {
        let command_topic = details.get_topic_for_command(None);
        Text {
            details: details.add_command(command_topic.clone()),
            mode: TextMode::Text,
            command_topic,
        }
    }
    #[allow(dead_code)]
    pub fn with_password(mut self) -> Self {
        self.mode = TextMode::Password;
        self
    }
}

#[async_trait]
impl Entity for Text {
    async fn json_for_discovery<'a>(
        &'a self,
        device: &'a Device,
        _cancellation_token: CancellationToken,
    ) -> Result<Value> {
        let json = json!({
            "platform": "text",
            "command_topic": self.command_topic,
            "mode": self.mode,
        });
        let mut entity_details_json = self.details.json_for_discovery(device).await?;
        if let (Value::Object(entity_details_map), Value::Object(sensor_map)) = (&mut entity_details_json, json) {
            entity_details_map.extend(sensor_map);
        } else {
            return Err(Error::IncorrectJsonStructure);
        }
        let mut map: Map<String, Value> = Map::new();
        map.insert(self.details.id.clone(), entity_details_json);
        Ok(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::test_helpers::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[tokio::test]
    async fn test_text_plain() {
        let text = Text::new("dev1".to_string(), "Some text".to_string(), "mdi:x".to_string());
        assert_eq!(text.mode, TextMode::Text);
    }

    #[tokio::test]
    async fn test_text_password() {
        let text = Text::new("dev1".to_string(), "Some text".to_string(), "mdi:x".to_string()).with_password();
        assert_eq!(text.mode, TextMode::Password);
    }

    #[tokio::test]
    async fn test_text_json_for_discovery() {
        let device = make_device();
        let text = Text::new("dev1".to_string(), "Some text".to_string(), "mdi:x".to_string());
        let json = text
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "some_text": {
                "command_topic": "dev1/some_text/command",
                "icon": "mdi:x",
                "mode": "text",
                "name": "Some text",
                "platform": "text",
                "state_topic": "dev1/some_text/state",
                "unique_id": "test_device_some_text",
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[cfg(feature = "proptests")]
    mod prop_test {
        use super::*;
        use crate::helpers::slugify;
        use pretty_assertions::assert_eq;
        use proptest::prelude::*;
        proptest! {
            #[test]
            fn prop_test_text_json_for_discovery(device_identifier in "\\PC*", name in "\\PC*", icon in "\\PC*") {
                let device = make_device();
                let text = Text::new(device_identifier.clone(), name.clone(), icon.clone());
                let json = futures::executor::block_on(text.json_for_discovery(&device, CancellationToken::default())).unwrap();
                let name_slug = slugify(name.clone());
                let expected_json = json!({
                    name_slug.clone(): {
                        "command_topic": format!("{device_identifier}/{name_slug}/command"),
                        "icon": icon,
                        "mode": "text",
                        "name": name,
                        "platform": "text",
                        "state_topic": format!("{device_identifier}/{name_slug}/state"),
                        "unique_id": format!("test_device_{}", name_slug),
                    }
                });
                assert_eq!(
                    serde_json::to_string_pretty(&json).unwrap(),
                    serde_json::to_string_pretty(&expected_json).unwrap()
                );
            }
        }
    }
}
