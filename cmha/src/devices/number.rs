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
pub struct Number {
    pub details: EntityDetails,
    pub command_topic: String,
    pub mode: NumberMode,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NumberMode {
    Auto,
    Box,
    Slider,
}

impl Number {
    #[allow(dead_code)]
    pub fn new(device_identifier: String, name: String, icon: String) -> Self {
        Self::new_with_details(EntityDetails::new(device_identifier, name, icon))
    }
    pub fn new_with_details(details: EntityDetails) -> Self {
        let command_topic = details.get_topic_for_command(None);
        Number {
            details: details.add_command(command_topic.clone()),
            mode: NumberMode::Auto,
            command_topic,
        }
    }
    #[allow(dead_code)]
    pub fn with_mode_slider(mut self) -> Self {
        self.mode = NumberMode::Slider;
        self
    }
    #[allow(dead_code)]
    pub fn with_mode_box(mut self) -> Self {
        self.mode = NumberMode::Box;
        self
    }
}

#[async_trait]
impl Entity for Number {
    async fn json_for_discovery<'a>(
        &'a self,
        device: &'a Device,
        _cancellation_token: CancellationToken,
    ) -> Result<Value> {
        let json = json!({
            "platform": "number",
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
    async fn test_number_auto() {
        let number = Number::new("dev1".to_string(), "Some number".to_string(), "mdi:x".to_string());
        assert_eq!(number.mode, NumberMode::Auto);
    }

    #[tokio::test]
    async fn test_number_slider() {
        let number = Number::new("dev1".to_string(), "Some number".to_string(), "mdi:x".to_string()).with_mode_slider();
        assert_eq!(number.mode, NumberMode::Slider);
    }

    #[tokio::test]
    async fn test_number_box() {
        let number = Number::new("dev1".to_string(), "Some number".to_string(), "mdi:x".to_string()).with_mode_box();
        assert_eq!(number.mode, NumberMode::Box);
    }

    #[tokio::test]
    async fn test_number_json_for_discovery() {
        let device = make_device();
        let number = Number::new("dev1".to_string(), "Some number".to_string(), "mdi:x".to_string());
        let json = number
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "some_number": {
                "command_topic": "dev1/some_number/command",
                "icon": "mdi:x",
                "mode": "auto",
                "name": "Some number",
                "platform": "number",
                "state_topic": "dev1/some_number/state",
                "unique_id": "test_device_some_number",
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }
}
