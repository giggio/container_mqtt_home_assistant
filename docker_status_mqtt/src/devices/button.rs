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
pub struct Button {
    pub details: EntityDetails,
    pub device_class: Option<String>,
    pub command_topic: String,
    pub can_be_made_unavailable: bool,
}

pub enum ButtonDeviceClass {
    Identify,
    #[allow(dead_code)]
    Restart,
    #[allow(dead_code)]
    Update,
}

impl Button {
    pub fn new(device_identifier: impl Into<String>, name: impl Into<String>, icon: impl Into<String>) -> Self {
        Self::new_with_details(EntityDetails::new(device_identifier.into(), name, icon))
    }
    pub fn new_with_details(details: EntityDetails) -> Self {
        let command_topic = details.get_topic_for_command(None);
        Button {
            details: details.add_command(command_topic.clone()),
            device_class: None,
            command_topic,
            can_be_made_unavailable: false,
        }
    }

    pub fn with_device_class(mut self, device_class: ButtonDeviceClass) -> Self {
        self.device_class = Some(
            match device_class {
                ButtonDeviceClass::Identify => "identify",
                ButtonDeviceClass::Restart => "restart",
                ButtonDeviceClass::Update => "update",
            }
            .to_string(),
        );
        self
    }
    pub fn can_be_made_unavailable(mut self) -> Self {
        self.can_be_made_unavailable = true;
        self
    }
}

impl From<EntityDetails> for Box<Button> {
    fn from(val: EntityDetails) -> Self {
        Box::new(Button::new_with_details(val))
    }
}

#[async_trait]
impl Entity for Button {
    async fn json_for_discovery<'a>(
        &'a self,
        device: &'a Device,
        _cancellation_token: CancellationToken,
    ) -> Result<serde_json::Value> {
        let json = json!({
            "device_class": self.device_class,
            "platform": "button",
            "command_topic": self.command_topic,
        });
        let mut entity_details_json = self.details.json_for_discovery(device).await?;
        if let (Value::Object(entity_details_map), Value::Object(sensor_map)) = (&mut entity_details_json, json) {
            entity_details_map.extend(sensor_map);
            if self.can_be_made_unavailable {
                let availability_topic = self.details.get_topic_for_availability(None);
                entity_details_map.insert(
                    "availability".to_string(),
                    json!([
                        { "topic": device.availability_topic() },
                        { "topic": availability_topic },
                    ]),
                );
                entity_details_map.insert("availability_mode".to_string(), Value::String("all".to_string()));
            }
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
    use crate::devices::test_helpers::*;

    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[tokio::test]
    async fn test_button_command_topic() {
        let button = Button::new("dev1".to_string(), "Test Button".to_string(), "mdi:button".to_string());
        assert_eq!(button.command_topic, "dev1/test_button/command");
        assert!(
            button
                .details
                .command_topics
                .contains(&"dev1/test_button/command".to_string())
        );
    }

    #[tokio::test]
    async fn test_button_json_for_discovery() {
        let device = create_test_device_with_identifier("test_device");
        let button = Button::new(
            "test_device".to_string(),
            "Test Button".to_string(),
            "mdi:button".to_string(),
        );
        let json = button
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
          "test_button": {
            "command_topic": "test_device/test_button/command",
            "device_class": null,
            "icon": "mdi:button",
            "name": "Test Button",
            "platform": "button",
            "state_topic": "test_device/test_button/state",
            "unique_id": "test_device_test_button"
          }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_button_json_for_discovery_with_device_class() {
        let device = create_test_device_with_identifier("test_device");
        let button = Button::new(
            "test_device".to_string(),
            "Outlet Button".to_string(),
            "mdi:power-socket".to_string(),
        )
        .with_device_class(ButtonDeviceClass::Identify);
        let json = button
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "outlet_button": {
                "command_topic": "test_device/outlet_button/command",
                "device_class": "identify",
                "icon": "mdi:power-socket",
                "name": "Outlet Button",
                "platform": "button",
                "state_topic": "test_device/outlet_button/state",
                "unique_id": "test_device_outlet_button"
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_button_json_for_discovery_can_be_made_unavailable() {
        let device = create_test_device_with_identifier("test_device");
        let button = Button::new(
            "test_device".to_string(),
            "Test Button".to_string(),
            "mdi:button".to_string(),
        )
        .can_be_made_unavailable();
        let json = button
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
          "test_button": {
            "command_topic": "test_device/test_button/command",
            "device_class": null,
            "icon": "mdi:button",
            "name": "Test Button",
            "platform": "button",
            "state_topic": "test_device/test_button/state",
            "unique_id": "test_device_test_button",
            "availability": [
                {
                    "topic": "test_device/availability",
                },
                {
                    "topic": "test_device/test_button/availability",
                },
            ],
            "availability_mode": "all",
          }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }
}
