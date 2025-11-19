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
pub struct Switch {
    pub details: EntityDetails,
    pub payload_on: String,
    pub payload_off: String,
    pub device_class: Option<String>,
    pub command_topic: String,
}

impl Switch {
    pub fn new(device_identifier: String, name: String, icon: String, device_class: Option<String>) -> Self {
        Self::new_with_details(EntityDetails::new(device_identifier, name, icon), device_class)
    }
    pub fn new_with_details(details: EntityDetails, device_class: Option<String>) -> Self {
        let command_topic = details.get_topic_for_command(None);
        Switch {
            details: details.add_command(command_topic.clone()),
            payload_on: "ON".to_owned(),
            payload_off: "OFF".to_owned(),
            device_class,
            command_topic,
        }
    }
}

impl From<EntityDetails> for Switch {
    fn from(details: EntityDetails) -> Self {
        let command_topic = details.get_topic_for_command(None);
        Switch {
            details,
            payload_on: "ON".to_owned(),
            payload_off: "OFF".to_owned(),
            device_class: None,
            command_topic,
        }
    }
}

#[async_trait]
impl Entity for Switch {
    async fn json_for_discovery<'a>(
        &'a self,
        device: &'a Device,
        _cancellation_token: CancellationToken,
    ) -> Result<serde_json::Value> {
        let json = json!({
            "device_class": self.device_class,
            "platform": "switch",
            "payload_on": self.payload_on,
            "payload_off": self.payload_off,
            "command_topic": self.command_topic,
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
    use crate::devices::test_helpers::*;

    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[tokio::test]
    async fn test_switch_command_topic() {
        let switch = Switch::new(
            "dev1".to_string(),
            "Test Switch".to_string(),
            "mdi:switch".to_string(),
            Some("switch".to_string()),
        );
        assert_eq!(switch.command_topic, "dev1/test_switch/command");
        assert!(
            switch
                .details
                .command_topics
                .contains(&"dev1/test_switch/command".to_string())
        );
    }

    #[tokio::test]
    async fn test_switch_json_for_discovery() {
        let device = make_device_with_identifier("test_device");
        let switch = Switch::new(
            "test_device".to_string(),
            "Test Switch".to_string(),
            "mdi:switch".to_string(),
            None,
        );
        let json = switch
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
          "test_switch": {
            "command_topic": "test_device/test_switch/command",
            "device_class": null,
            "icon": "mdi:switch",
            "name": "Test Switch",
            "payload_off": "OFF",
            "payload_on": "ON",
            "platform": "switch",
            "state_topic": "test_device/test_switch/state",
            "unique_id": "test_device_test_switch"
          }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_switch_json_for_discovery_with_device_class() {
        let device = make_device_with_identifier("test_device");
        let switch = Switch::new(
            "test_device".to_string(),
            "Outlet Switch".to_string(),
            "mdi:power-socket".to_string(),
            Some("outlet".to_string()),
        );
        let json = switch
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "outlet_switch": {
                "command_topic": "test_device/outlet_switch/command",
                "device_class": "outlet",
                "icon": "mdi:power-socket",
                "name": "Outlet Switch",
                "payload_off": "OFF",
                "payload_on": "ON",
                "platform": "switch",
                "state_topic": "test_device/outlet_switch/state",
                "unique_id": "test_device_outlet_switch"
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }
}
