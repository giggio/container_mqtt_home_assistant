use hashbrown::HashMap;
use std::fmt::Debug;

use serde::{Serialize, Serializer, ser::SerializeSeq};
use serde_json::{Map, Value, json};

use crate::{
    cancellation_token::CancellationToken,
    device_manager::CommandResult,
    devices::{Entity, Error},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Device {
    pub details: DeviceDetails,
    pub origin: DeviceOrigin,
    pub entities: Vec<Box<dyn Entity>>,
    pub availability_topic: String,
    cancellation_token: CancellationToken,
}

impl Device {
    pub fn new(
        details: DeviceDetails,
        origin: DeviceOrigin,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            details,
            origin,
            entities: vec![],
            availability_topic,
            cancellation_token,
        }
    }

    pub fn new_with_entities(
        details: DeviceDetails,
        origin: DeviceOrigin,
        availability_topic: String,
        entities: Vec<Box<dyn Entity>>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            details,
            origin,
            entities,
            availability_topic,
            cancellation_token,
        }
    }

    pub fn command_topics(&self) -> Vec<String> {
        let mut all_command_topics = Vec::new();
        for entity in &self.entities {
            let entity_details = entity.get_data().details();
            for command_topic in &entity_details.command_topics {
                all_command_topics.push(command_topic.to_owned());
            }
        }
        all_command_topics
    }

    pub fn availability_topic(&self) -> String {
        self.availability_topic.clone()
    }

    pub fn discovery_topic(&self, discovery_prefix: &str) -> String {
        format!("{discovery_prefix}/device/{}/config", self.details.identifier)
    }

    pub async fn get_entities_data(&self) -> Result<HashMap<String, String>> {
        let mut entities_data = HashMap::<String, String>::with_capacity(self.entities.len());
        for entity in self.entities.iter() {
            let provider_data = entity.get_entity_data(self.cancellation_token.clone()).await?;
            entities_data.extend(provider_data);
        }
        Ok(entities_data)
    }

    pub async fn json_for_discovery(&self) -> Result<serde_json::Value> {
        let mut device_json = json!({
            "device": self.details,
            "origin": self.origin,
            "availability_topic": self.availability_topic(),
        });
        let mut entities_map = Map::new();
        for entity in &self.entities {
            let entity_json = entity
                .get_data()
                .json_for_discovery(self, self.cancellation_token.clone())
                .await?;
            if let Value::Object(entity_obj) = entity_json {
                for (key, value) in entity_obj {
                    entities_map.insert(key, value);
                }
            } else {
                return Err(Error::IncorrectJsonStructure);
            }
        }
        if let Value::Object(device_map) = &mut device_json {
            device_map.insert("components".to_string(), Value::Object(entities_map));
        } else {
            return Err(Error::IncorrectJsonStructure);
        }
        Ok(device_json)
    }

    pub async fn handle_command(&mut self, topic: &str, payload: &str) -> Result<CommandResult> {
        for entity in self.entities.iter_mut() {
            trace!(
                "Checking device handling command '{topic}' for device '{}' and entity: '{}'",
                self.details.name,
                entity.get_data().details().name
            );
            let command_handle_result = entity
                .handle_command(topic, payload, self.cancellation_token.clone())
                .await?;
            if command_handle_result.handled {
                debug!(
                    "Device '{}' handled command '{topic}' for entity: '{}'",
                    self.details.name,
                    entity.get_data().details().name
                );
                return Ok(command_handle_result);
            } else {
                trace!(
                    "Entity '{}' did not handle command '{topic}' for device '{}'",
                    entity.get_data().details().name,
                    self.details.name
                );
            }
        }
        Ok(CommandResult {
            handled: false,
            state_update_topics: None,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct DeviceDetails {
    pub name: String,
    #[serde(serialize_with = "vec_string_from_string", rename = "identifiers")]
    pub identifier: String,
    pub manufacturer: String,
    pub sw_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via_device: Option<String>,
}

fn vec_string_from_string<S>(text: &String, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut seq = serializer.serialize_seq(Some(1))?;
    seq.serialize_element(text)?;
    seq.end()
}

#[derive(Debug, Serialize)]
pub struct DeviceOrigin {
    pub name: String,
    pub sw: String,
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::test_module::test_helpers::*;
    use crate::devices::{EntityDetails, MockAnEntityType, MockEntity};

    use mockall::predicate;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn test_device_command_topics_without_entities_is_empty() {
        let device = create_test_device();
        let topics = device.command_topics();
        assert_eq!(topics.len(), 0);
    }

    #[test]
    fn test_device_command_topics_has_topic_when_has_entity() {
        let mut device = create_test_device();
        let mut mock_entity_type = MockAnEntityType::new();
        mock_entity_type.expect_details().return_const(
            EntityDetails::new("dev1".to_string(), "Test Switch".to_string(), "mdi:switch".to_string())
                .add_command("dev1/test_switch/command".to_string()),
        );
        let mut mock_entity = MockEntity::new();
        mock_entity.expect_get_data().return_const(Box::new(mock_entity_type));
        device.entities.push(Box::new(mock_entity));

        let topics = device.command_topics();
        assert_eq!(topics.len(), 1);
        assert_eq!(topics[0], "dev1/test_switch/command");
    }

    #[test]
    fn test_device_availability_topic() {
        let device = create_test_device();
        assert_eq!(device.availability_topic(), "dev1/availability");
    }

    #[test]
    fn test_device_discovery_topic() {
        let device = create_test_device();
        assert_eq!(
            device.discovery_topic("homeassistant"),
            "homeassistant/device/test_device/config"
        );
    }

    #[tokio::test]
    async fn test_device_json_for_discovery() {
        let mut device = create_test_device();
        let entity = get_mock_entity();
        device.entities.push(Box::new(entity));
        let device_json = device.json_for_discovery().await.unwrap();
        let expected_json = json!({
            "device": {
                "identifiers": ["test_device"],
                "name": "Test Device",
                "manufacturer": "Test Mfg",
                "sw_version": "1.0.0",
            },
            "origin": {
                "name": "test-origin",
                "sw": "1.0.0",
                "url": "http://example.com"
            },
            "availability_topic": "dev1/availability",
            "components": {
                "test": true,
           },
        });
        assert_eq!(
            serde_json::to_string_pretty(&device_json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_device_devices_data() {
        let mut device = create_test_device();
        device.entities.push(Box::new(get_mock_entity()));
        let data = device.get_entities_data().await.unwrap();
        assert_eq!(
            hashmap! { "dev1/test_name/state".to_string() => "test_state".to_string() },
            data
        );
    }

    #[test]
    fn test_device_command_topics_multiple_entities() {
        let mut device = create_test_device();

        let mut mock_entity_type1 = MockAnEntityType::new();
        mock_entity_type1.expect_details().return_const(
            EntityDetails::new("dev1".to_string(), "Switch 1".to_string(), "mdi:switch".to_string())
                .add_command("dev1/switch1/command".to_string()),
        );
        let mut mock_entity1 = MockEntity::new();
        mock_entity1.expect_get_data().return_const(Box::new(mock_entity_type1));
        device.entities.push(Box::new(mock_entity1));

        let mut mock_entity_type2 = MockAnEntityType::new();
        mock_entity_type2.expect_details().return_const(
            EntityDetails::new("dev1".to_string(), "Switch 2".to_string(), "mdi:switch".to_string())
                .add_command("dev1/switch2/command".to_string())
                .add_command("dev1/switch2_brightness/command".to_string()),
        );
        let mut mock_entity2 = MockEntity::new();
        mock_entity2.expect_get_data().return_const(Box::new(mock_entity_type2));
        device.entities.push(Box::new(mock_entity2));

        let topics = device.command_topics();
        assert_eq!(topics.len(), 3);
        assert!(topics.contains(&"dev1/switch1/command".to_string()));
        assert!(topics.contains(&"dev1/switch2/command".to_string()));
        assert!(topics.contains(&"dev1/switch2_brightness/command".to_string()));
    }

    #[tokio::test]
    async fn test_device_handle_command_no_handlers() {
        let mut device = create_test_device();

        let result = device.handle_command("unknown/topic", "payload").await.unwrap();
        assert!(!result.handled);
        assert_eq!(result.state_update_topics, None);
    }

    #[tokio::test]
    async fn test_device_handle_command_with_handler() {
        let mut device = create_test_device();

        let mut mock_entity_type = MockAnEntityType::new();
        mock_entity_type.expect_details().return_const(EntityDetails::new(
            "dev1".to_string(),
            "Test Entity".to_string(),
            "mdi:test".to_string(),
        ));
        let mut mock_entity = MockEntity::new();
        mock_entity.expect_get_data().return_const(Box::new(mock_entity_type));
        mock_entity
            .expect_handle_command()
            .with(
                predicate::eq("test/topic"),
                predicate::eq("test_payload"),
                predicate::always(),
            )
            .returning(|_, _, _| {
                Box::pin(async move {
                    Ok(CommandResult {
                        handled: true,
                        state_update_topics: Some(hashmap! {"test/state".to_string() => "1".to_string()}),
                    })
                })
            });
        device.entities.push(Box::new(mock_entity));

        let result = device.handle_command("test/topic", "test_payload").await.unwrap();
        assert!(result.handled);
        assert_eq!(
            result.state_update_topics,
            Some(hashmap! {"test/state".to_string() => "1".to_string()})
        );
    }

    #[tokio::test]
    async fn test_device_json_for_discovery_empty_entities() {
        let device = create_test_device();

        let json = device.json_for_discovery().await.unwrap();
        let expected_json = json!({
            "device": {
                "identifiers": ["test_device"],
                "name": "Test Device",
                "manufacturer": "Test Mfg",
                "sw_version": "1.0.0",
            },
            "origin": {
                "name": "test-origin",
                "sw": "1.0.0",
                "url": "http://example.com",
            },
            "availability_topic": "dev1/availability",
            "components": {}
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[test]
    fn test_entity_details_with_special_characters() {
        let details = EntityDetails::new(
            "device_with_underscores".to_string(),
            "Entity with Spaces & Special!".to_string(),
            "mdi:special-icon".to_string(),
        );
        assert_eq!(details.device_identifier, "device_with_underscores");
        assert_eq!(details.id, "entity_with_spaces_special");
        assert_eq!(details.name, "Entity with Spaces & Special!");
        assert_eq!(details.icon, "mdi:special-icon");
    }

    #[tokio::test]
    async fn test_device_discovery_topic_different_prefixes() {
        let device = Device::new(
            create_device_details(),
            create_device_origin(),
            "test_device/availability".to_string(),
            CancellationToken::default(),
        );

        assert_eq!(
            device.discovery_topic("homeassistant"),
            "homeassistant/device/test_device/config"
        );
        assert_eq!(device.discovery_topic("custom"), "custom/device/test_device/config");
        assert_eq!(device.discovery_topic(""), "/device/test_device/config");
    }
}
