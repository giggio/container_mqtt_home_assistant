use async_trait::async_trait;
use docker_status_mqtt_proc_macros::*;
use serde_json::{Map, Value, json};
use std::fmt::Debug;

use crate::{
    cancellation_token::CancellationToken,
    devices::{Entity, EntityDetails, EntityDetailsGetter, Error, device::Device},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(EntityDetailsGetter, Debug)]
pub struct Light {
    pub details: EntityDetails,
    pub payload_on: String,
    pub payload_off: String,
    pub supports_brightness: bool,
    pub brightness_scale: u8,
    pub command_topic: String,
    pub brightness_command_topic: Option<String>,
}

impl Light {
    pub async fn new(device_identifier: String, name: String, icon: String) -> Self {
        Self::new_with_details(EntityDetails::new(device_identifier, name, icon)).await
    }
    pub async fn new_with_details(details: EntityDetails) -> Self {
        let command_topic = details.get_topic_for_command(None);
        Light {
            details: details.add_command(command_topic.clone()),
            payload_on: "ON".to_owned(),
            payload_off: "OFF".to_owned(),
            supports_brightness: false,
            brightness_scale: 255,
            command_topic,
            brightness_command_topic: None,
        }
    }
    pub async fn support_brightness(mut self, scale: u8) -> Self {
        self.supports_brightness = true;
        self.brightness_scale = scale;
        let brightness_command_topic = self.details.get_topic_for_command(Some("brightness"));
        self.details = self.details.add_command(brightness_command_topic.clone());
        self.brightness_command_topic = Some(brightness_command_topic);
        self
    }
}

#[async_trait]
impl Entity for Light {
    async fn json_for_discovery<'a>(
        &'a self,
        device: &'a Device,
        _cancellation_token: CancellationToken,
    ) -> Result<Value> {
        let json = json!({
            "platform": "light",
            "payload_on": self.payload_on,
            "payload_off": self.payload_off,
            "command_topic": self.command_topic,
            "brightness_scale": (if self.supports_brightness { self.brightness_scale } else { 255 }),
            "brightness_command_topic": (if self.supports_brightness { self.brightness_command_topic.clone() } else { None }),
            "brightness_state_topic": (if self.supports_brightness { Some(self.details.get_topic_for_state(Some("brightness"))) } else { None }),
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
    async fn test_light_creation_basic() {
        let light = Light::new("dev1".to_string(), "Test light".to_string(), "mdi:x".to_string()).await;
        assert_eq!(light.payload_on, "ON");
        assert_eq!(light.payload_off, "OFF");
        assert!(!light.supports_brightness);
        assert_eq!(light.brightness_scale, 255);
        assert_eq!(light.command_topic, "dev1/test_light/command");
    }

    #[tokio::test]
    async fn test_light_with_brightness() {
        let light = Light::new(
            "dev1".to_string(),
            "Dimmable Light".to_string(),
            "mdi:lightbulb".to_string(),
        )
        .await
        .support_brightness(100)
        .await;
        assert!(light.supports_brightness);
        assert_eq!(light.brightness_scale, 100);
        assert_eq!(
            light.brightness_command_topic.unwrap(),
            "dev1/dimmable_light_brightness/command"
        );
    }

    #[tokio::test]
    async fn test_light_support_brightness_chaining() {
        let light = Light::new(
            "dev1".to_string(),
            "Dimmable Light".to_string(),
            "mdi:lightbulb".to_string(),
        )
        .await
        .support_brightness(200)
        .await;
        assert!(light.supports_brightness);
        assert_eq!(light.brightness_scale, 200);
        assert_eq!(
            light.brightness_command_topic.unwrap(),
            "dev1/dimmable_light_brightness/command"
        );
        assert!(
            light
                .details
                .command_topics
                .contains(&"dev1/dimmable_light/command".to_string())
        );
        assert!(
            light
                .details
                .command_topics
                .contains(&"dev1/dimmable_light_brightness/command".to_string())
        );
    }

    #[tokio::test]
    async fn test_light_json_for_discovery_without_brightness() {
        let device = make_device_with_identifier("test_device");
        let light = Light::new(
            "test_device".to_string(),
            "Simple Light".to_string(),
            "mdi:lightbulb".to_string(),
        )
        .await;
        let json = light
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "simple_light": {
                "brightness_command_topic": null,
                "brightness_scale": 255,
                "brightness_state_topic": null,
                "command_topic": "test_device/simple_light/command",
                "icon": "mdi:lightbulb",
                "name": "Simple Light",
                "payload_off": "OFF",
                "payload_on": "ON",
                "platform": "light",
                "state_topic": "test_device/simple_light/state",
                "unique_id": "test_device_simple_light"
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_light_json_for_discovery_with_brightness() {
        let device = make_device_with_identifier("test_device");
        let light = Light::new(
            "test_device".to_string(),
            "Dimmable Light".to_string(),
            "mdi:lightbulb".to_string(),
        )
        .await
        .support_brightness(100)
        .await;
        let json = light
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "dimmable_light": {
                "brightness_command_topic": "test_device/dimmable_light_brightness/command",
                "brightness_scale": 100,
                "brightness_state_topic": "test_device/dimmable_light_brightness/state",
                "command_topic": "test_device/dimmable_light/command",
                "icon": "mdi:lightbulb",
                "name": "Dimmable Light",
                "payload_off": "OFF",
                "payload_on": "ON",
                "platform": "light",
                "state_topic": "test_device/dimmable_light/state",
                "unique_id": "test_device_dimmable_light"
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_light_brightness_scale_edge_cases() {
        let light1 = Light::new(
            "dev1".to_string(),
            "Min Brightness".to_string(),
            "mdi:lightbulb".to_string(),
        )
        .await
        .support_brightness(0)
        .await;
        assert_eq!(light1.brightness_scale, 0);

        let light2 = Light::new(
            "dev1".to_string(),
            "Max Brightness".to_string(),
            "mdi:lightbulb".to_string(),
        )
        .await
        .support_brightness(255)
        .await;
        assert_eq!(light2.brightness_scale, 255);
    }
}
