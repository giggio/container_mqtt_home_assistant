pub use crate::devices::{
    button::{Button, ButtonDeviceClass},
    device::*,
    devices::Devices,
    light::Light,
    sensor::Sensor,
    switch::Switch,
    text::Text,
};
use crate::{cancellation_token::CancellationToken, device_manager::CommandResult, helpers::*};
use async_trait::async_trait;
use hashbrown::{HashMap, HashSet};
use serde_json::{Value, json};
use std::{fmt::Debug, sync::Arc};
use tokio::sync::RwLock;

mod button;
mod device;
#[allow(clippy::module_inception)]
mod devices;
mod light;
mod number;
mod sensor;
mod switch;
mod text;

#[cfg(test)]
pub mod test_module;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Cancelled(#[from] crate::cancellation_token::Error),
    #[error("Device provider error: {0}")]
    DeviceProvider(String),
    #[error("Incorrect JSON structure")]
    IncorrectJsonStructure,
    #[error("Unknown error")]
    Unknown,
}

impl PartialEq<dyn Entity> for Box<dyn Entity + '_> {
    fn eq(&self, other: &dyn Entity) -> bool {
        self.get_data().details() == other.get_data().details()
    }
}

impl PartialEq for dyn Entity + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.get_data().details() == other.get_data().details()
    }
}

#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait Entity: Send + Sync + Debug {
    async fn get_entity_data(&self, _cancellation_token: CancellationToken) -> Result<HashMap<String, String>> {
        Ok(HashMap::new())
    }
    fn get_data(&self) -> &dyn EntityType;
    #[allow(unused_variables)]
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        warn!(
            "Entity {} received command for topic {topic} but has no handler implemented",
            self.get_data().details().name
        );
        Ok(CommandResult {
            handled: false,
            state_update_topics: None,
        })
    }
    async fn handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        if !self.get_data().details().command_topics.contains(&topic.to_owned()) {
            trace!(
                "Entity {} received event for topic {topic} and CANNOT handle it",
                self.get_data().details().name
            );
            Ok(CommandResult {
                handled: false,
                state_update_topics: None,
            })
        } else {
            trace!(
                "Entity {} received event for topic {topic} and CAN handle it",
                self.get_data().details().name
            );
            self.do_handle_command(topic, payload, cancellation_token).await
        }
    }
}

#[async_trait]
pub trait EntityType: EntityDetailsGetter + Send + Sync + Debug {
    async fn json_for_discovery<'a>(
        &'a self,
        device: &'a Device,
        cancellation_token: CancellationToken,
    ) -> Result<serde_json::Value>;
}

#[cfg(test)]
mockall::mock! {
    #[derive(Debug)]
    pub AnEntityType { }
    #[async_trait]
    impl EntityType for AnEntityType {
        async fn json_for_discovery<'a>(&'a self, device: &'a Device, cancellation_token: CancellationToken) -> Result<serde_json::Value>;
    }
    impl EntityDetailsGetter for AnEntityType {
        fn details(&self) -> &EntityDetails;
    }
}

#[derive(Debug, PartialEq)]
pub struct EntityDetails {
    pub device_identifier: String,
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub has_attributes: bool,
    pub command_topics: Vec<String>,
}

impl EntityDetails {
    pub fn new(device_identifier: impl Into<String>, name: impl Into<String>, icon: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            device_identifier: device_identifier.into(),
            id: slugify(&name),
            name,
            icon: Some(icon.into()),
            has_attributes: false,
            command_topics: Vec::new(),
        }
    }

    pub fn new_without_icon(device_identifier: impl Into<String>, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            device_identifier: device_identifier.into(),
            id: slugify(&name),
            name,
            icon: None,
            has_attributes: false,
            command_topics: Vec::new(),
        }
    }

    pub fn set_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn has_attributes(mut self) -> Self {
        self.has_attributes = true;
        self
    }

    pub fn add_command(mut self, command: String) -> Self {
        self.command_topics.push(command);
        self
    }

    pub fn get_topic_for_command(&self, item: Option<&str>) -> String {
        let topic_path = self.get_topic_path(item);
        let command = format!("{topic_path}/command");
        command
    }

    pub fn get_topic_for_state(&self, item: Option<&str>) -> String {
        let topic_path = self.get_topic_path(item);
        format!("{topic_path}/state")
    }

    pub fn get_topic_path(&self, item: Option<&str>) -> String {
        let basic_path = format!("{}/{}", self.device_identifier, self.id);
        if let Some(item) = item {
            return format!("{basic_path}_{item}");
        }
        basic_path
    }

    pub async fn json_for_discovery(&self, device: &Device) -> Result<serde_json::Value> {
        let json = json!({
            "name": self.name,
            "unique_id": format!("{}_{}", device.details.identifier, self.id),
            "state_topic": self.get_topic_for_state(None),
        });
        let mut json_map = cast!(json, Value::Object);
        if let Some(icon) = &self.icon {
            json_map.insert("icon".to_owned(), Value::String(icon.clone()));
        }
        if self.has_attributes {
            json_map.insert(
                "json_attributes_topic".to_owned(),
                Value::String(self.get_topic_for_state(Some("attributes"))),
            );
            json_map.insert(
                "json_attributes_template".to_owned(),
                Value::String("{{ value_json | tojson }}".to_string()),
            );
        }
        Ok(Value::Object(json_map))
    }
}

pub trait EntityDetailsGetter {
    fn details(&self) -> &EntityDetails;
}

#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait DeviceProvider: Send + Sync {
    fn id(&self) -> String;
    async fn get_devices(&self, availability_topic: String, cancellation_token: CancellationToken) -> Result<Devices>;
    // Removes the devices that are no longer present on the physical device. Returns the identifiers of the removed devices.
    async fn remove_missing_devices(
        &self,
        devices: &Devices,
        cancellation_token: CancellationToken,
    ) -> Result<Vec<Arc<RwLock<Device>>>>;
    // Adds new devices that are present on the physical device but not in the provided Devices collection. Returns the
    // identifiers of the added devices.
    async fn add_discovered_devices(
        &self,
        devices: &Devices,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> Result<HashSet<String>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn create_test_entity_details() -> EntityDetails {
        EntityDetails::new("dev1".to_string(), "Test Entity".to_string(), "mdi:test".to_string())
    }

    #[test]
    fn test_entity_details_has_attributes() {
        let details = create_test_entity_details().has_attributes();
        assert!(details.has_attributes);
    }

    #[test]
    fn test_entity_details_add_multiple_commands() {
        let details = create_test_entity_details()
            .add_command("command1".to_string())
            .add_command("command2".to_string());
        assert_eq!(details.command_topics.len(), 2);
        assert_eq!(details.command_topics[0], "command1");
        assert_eq!(details.command_topics[1], "command2");
    }

    #[test]
    fn test_entity_details_get_topic_path() {
        let details = create_test_entity_details();
        assert_eq!(details.get_topic_path(None), "dev1/test_entity");
        assert_eq!(
            details.get_topic_path(Some("brightness")),
            "dev1/test_entity_brightness"
        );
    }

    #[test]
    fn test_entity_details_get_topic_for_command() {
        let details = create_test_entity_details();
        assert_eq!(details.get_topic_for_command(None), "dev1/test_entity/command");
        assert_eq!(
            details.get_topic_for_command(Some("brightness")),
            "dev1/test_entity_brightness/command"
        );
    }

    #[test]
    fn test_entity_details_add_command() {
        let details = EntityDetails::new("dev1".to_string(), "Test Switch".to_string(), "mdi:switch".to_string())
            .add_command("dev1/test_switch/command".to_string());
        assert_eq!(details.command_topics.len(), 1);
        assert_eq!(details.command_topics[0], "dev1/test_switch/command");
    }

    #[test]
    fn test_entity_details_get_topics_for_state() {
        let details = EntityDetails::new("dev1".to_string(), "Test Sensor".to_string(), "mdi:test".to_string());
        assert_eq!(details.get_topic_for_state(None), "dev1/test_sensor/state");
        assert_eq!(
            details.get_topic_for_state(Some("brightness")),
            "dev1/test_sensor_brightness/state"
        );
    }
}
