use std::fmt::{Debug, Display};

use async_trait::async_trait;
use docker_status_mqtt_proc_macros::*;
use serde_json::{Map, Value, json};

use crate::device_manager::CommandResult;

#[derive(Debug)]
pub struct Device {
    pub node_id: String,
    pub details: DeviceDetails,
    pub origin: DeviceOrigin,
    pub components: Vec<Box<dyn Entity>>,
}

#[derive(Debug)]
pub struct DeviceDetails {
    pub name: String,
    pub identifier: String,
    pub manufacturer: String,
    pub sw_version: String,
}

#[derive(Debug)]
pub struct DeviceOrigin {
    pub name: String,
    pub sw: String,
    pub url: String,
}

#[async_trait]
pub trait Entity: Send + Sync + Debug {
    fn get_entity_data_provider(&self) -> &dyn EntityDataProvider;
    fn get_data(&self) -> &dyn EntityType;
    async fn handle_command(
        &mut self,
        topic: &str,
        payload: &str,
    ) -> Result<CommandResult, DevicesError>;
}

#[async_trait]
pub trait EntityType: ComponentDetailsGetter + Send + Sync + Debug {
    // todo: needs Send + Sync?
    async fn json_for_discovery(
        &self,
        device_opt: Option<&Device>,
    ) -> Result<serde_json::Value, DevicesError>;
}

#[derive(Debug)]
pub struct ComponentDetails {
    pub device_node_id: String,
    pub device_identifier: String,
    pub id: String,
    pub name: String,
    pub icon: String,
    pub has_attributes: bool,
    pub command_topics: Vec<String>,
}

#[async_trait]
pub trait EntityDataProvider: Send + Sync + Display + Debug {
    // todo: needs Send + Sync?
    async fn get_entity_data(&self, entity: &dyn Entity) -> Result<Vec<EntityData>, DevicesError>;
}

pub trait ComponentDetailsGetter {
    fn details(&self) -> &ComponentDetails;
}

#[derive(ComponentDetailsGetter, Debug)]
pub struct Sensor {
    pub details: ComponentDetails,
    pub unit_of_measurement: String,
    pub device_class: Option<String>,
}

impl ComponentDetails {
    pub fn new(
        device_node_id: String,
        device_identifier: String,
        name: String,
        icon: String,
    ) -> Self {
        ComponentDetails {
            device_node_id,
            device_identifier,
            id: slug::slugify(&name),
            name,
            icon,
            has_attributes: false,
            command_topics: Vec::new(),
        }
    }
    pub fn has_attributes(mut self) -> Self {
        self.has_attributes = true;
        self
    }
    pub fn add_command(mut self, command: String) -> Self {
        self.command_topics.push(command);
        self
    }
}

impl ComponentDetails {
    fn get_topic_for_command(&self, item: Option<&str>) -> String {
        let topic_path = self.get_topic_path(item);
        let command = format!("{topic_path}/command");
        command
    }

    pub fn get_topic_for_state(&self, item: Option<&str>) -> String {
        let topic_path = self.get_topic_path(item);
        format!("{topic_path}/state")
    }

    fn get_topic_path(&self, item: Option<&str>) -> String {
        let basic_path = format!(
            "{}/{}/{}",
            self.device_node_id, self.device_identifier, self.id
        );
        if let Some(item) = item {
            return format!("{basic_path}_{item}");
        }
        basic_path
    }
}

impl Sensor {
    pub fn new(
        device_node_id: String,
        device_identifier: String,
        name: String,
        icon: String,
        unit_of_measurement: String,
        device_class: Option<String>,
    ) -> Self {
        Self::new_with_details(
            ComponentDetails::new(device_node_id, device_identifier, name, icon),
            unit_of_measurement,
            device_class,
        )
    }
    pub fn new_with_details(
        details: ComponentDetails,
        unit_of_measurement: String,
        device_class: Option<String>,
    ) -> Self {
        Sensor {
            details,
            unit_of_measurement,
            device_class,
        }
    }
}

#[derive(ComponentDetailsGetter, Debug)]
pub struct Switch {
    pub details: ComponentDetails,
    pub payload_on: String,
    pub payload_off: String,
    pub device_class: Option<String>,
    pub command_topic: String,
}

impl Switch {
    pub async fn new(
        device_node_id: String,
        device_identifier: String,
        name: String,
        icon: String,
        device_class: Option<String>,
    ) -> Self {
        Self::new_with_details(
            ComponentDetails::new(device_node_id, device_identifier, name, icon),
            device_class,
        )
        .await
    }
    pub async fn new_with_details(details: ComponentDetails, device_class: Option<String>) -> Self {
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

#[derive(ComponentDetailsGetter, Debug)]
pub struct Light {
    pub details: ComponentDetails,
    pub payload_on: String,
    pub payload_off: String,
    pub supports_brightness: bool,
    pub brightness_scale: u8,
    pub command_topic: String,
    pub brightness_command_topic: Option<String>,
}

impl Light {
    pub async fn new(
        device_node_id: String,
        device_identifier: String,
        name: String,
        icon: String,
    ) -> Self {
        Self::new_with_details(ComponentDetails::new(
            device_node_id,
            device_identifier,
            name,
            icon,
        ))
        .await
    }
    pub async fn new_with_details(details: ComponentDetails) -> Self {
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
pub trait DeviceProvider {
    async fn get_devices(&self) -> Vec<Device>;
}

#[derive(Debug, PartialEq)]
pub struct EntityData {
    pub topic: String,
    pub payload: String,
}

impl Device {
    pub fn command_topics(&self) -> Vec<String> {
        let mut all_command_topics = Vec::new();
        for component in &self.components {
            let component_details = component.get_data().details();
            for command_topic in &component_details.command_topics {
                all_command_topics.push(command_topic.to_owned());
            }
        }
        all_command_topics
    }

    pub fn discovery_topic(&self, discovery_prefix: &str) -> String {
        format!("{}/device/{}/config", discovery_prefix, self.node_id)
    }

    pub fn availability_topic(&self) -> String {
        format!("{}/{}/availability", self.node_id, self.details.identifier)
    }

    pub async fn get_entities_data(&self) -> Result<Vec<EntityData>, DevicesError> {
        let mut entities_data = Vec::<EntityData>::with_capacity(self.components.len() * 2); // rough estimate
        for component in self.components.iter() {
            let entity_data = component
                .get_entity_data_provider()
                .get_entity_data(component.as_ref())
                .await?;
            entities_data.extend(entity_data);
        }
        Ok(entities_data)
    }

    pub async fn json_for_discovery(&self) -> Result<serde_json::Value, DevicesError> {
        let mut device_json = json!({
            "device": {
                "identifiers": [self.details.identifier],
                "name": self.details.name,
                "manufacturer": self.details.manufacturer,
                "sw_version": self.details.sw_version,
            },
            "origin": {
                "name": self.origin.name,
                "sw": self.origin.sw,
                "url": self.origin.url,
            },
            "availability_topic": format!("{}/{}/availability", self.node_id, self.details.identifier),
        });
        let mut components_map = Map::new();
        for component in &self.components {
            let component_json = component.get_data().json_for_discovery(Some(self)).await?;
            if let Value::Object(component_obj) = component_json {
                for (key, value) in component_obj {
                    components_map.insert(key, value);
                }
            } else {
                return Err(DevicesError::IncorrectJsonStructure);
            }
        }
        if let Value::Object(device_map) = &mut device_json {
            device_map.insert("components".to_string(), Value::Object(components_map));
        } else {
            return Err(DevicesError::IncorrectJsonStructure);
        }
        Ok(device_json)
    }

    pub async fn handle_command(
        &mut self,
        topic: &str,
        payload: &str,
    ) -> Result<CommandResult, DevicesError> {
        for component in self.components.iter_mut() {
            trace!(
                "Checking device handling command '{topic}' for device '{}' and component: '{}'",
                self.details.name,
                component.get_data().details().name
            );
            let command_handle_result = component.handle_command(topic, payload).await?;
            if command_handle_result.handled {
                debug!(
                    "Device '{}' handled command '{topic}' for component: '{}'",
                    self.details.name,
                    component.get_data().details().name
                );
                return Ok(command_handle_result);
            } else {
                trace!(
                    "Component '{}' did not handle command '{topic}' for device '{}'",
                    component.get_data().details().name,
                    self.details.name
                );
            }
        }
        Ok(CommandResult {
            handled: false,
            state_update: None,
        })
    }
}

impl ComponentDetails {
    async fn to_json(
        &self,
        device_opt: Option<&Device>,
    ) -> Result<serde_json::Value, DevicesError> {
        let device = device_opt.ok_or(DevicesError::MissingDevice)?;
        let json = json!({
            "name": self.name,
            "unique_id": format!("{}_{}", device.details.identifier, self.id),
            "state_topic": self.get_topic_for_state(None),
            "icon": self.icon,
        });
        Ok(if self.has_attributes {
            let mut json_map = cast!(json, Value::Object);
            json_map.insert(
                "json_attributes_topic".to_owned(),
                Value::String(self.get_topic_for_state(Some("attributes"))),
            );
            json_map.insert(
                "json_attributes_template".to_owned(),
                Value::String("{{ value_json | tojson }}".to_string()),
            );
            Value::Object(json_map)
        } else {
            json
        })
    }
}

#[async_trait]
impl EntityType for Sensor {
    async fn json_for_discovery(
        &self,
        device_opt: Option<&Device>,
    ) -> Result<serde_json::Value, DevicesError> {
        let json = json!({
            "device_class": self.device_class,
            "platform": "sensor",
            "unit_of_measurement": self.unit_of_measurement,
        });
        let mut component_details_json = self.details.to_json(device_opt).await?;
        if let (Value::Object(component_details_map), Value::Object(sensor_map)) =
            (&mut component_details_json, json)
        {
            component_details_map.extend(sensor_map);
        } else {
            return Err(DevicesError::IncorrectJsonStructure);
        }
        let mut map: Map<String, Value> = Map::new();
        map.insert(self.details.id.clone(), component_details_json);
        Ok(Value::Object(map))
    }
}

#[async_trait]
impl EntityType for Switch {
    async fn json_for_discovery(
        &self,
        device_opt: Option<&Device>,
    ) -> Result<serde_json::Value, DevicesError> {
        let json = json!({
            "device_class": self.device_class,
            "platform": "switch",
            "payload_on": self.payload_on,
            "payload_off": self.payload_off,
            "command_topic": self.command_topic,
        });
        let mut component_details_json = self.details.to_json(device_opt).await?;
        if let (Value::Object(component_details_map), Value::Object(sensor_map)) =
            (&mut component_details_json, json)
        {
            component_details_map.extend(sensor_map);
        } else {
            return Err(DevicesError::IncorrectJsonStructure);
        }
        let mut map: Map<String, Value> = Map::new();
        map.insert(self.details.id.clone(), component_details_json);
        Ok(Value::Object(map))
    }
}

#[async_trait]
impl EntityType for Light {
    async fn json_for_discovery(
        &self,
        device_opt: Option<&Device>,
    ) -> Result<serde_json::Value, DevicesError> {
        let json = json!({
            "platform": "light",
            "payload_on": self.payload_on,
            "payload_off": self.payload_off,
            "command_topic": self.command_topic,
            "brightness_scale": (if self.supports_brightness { self.brightness_scale } else { 255 }),
            "brightness_command_topic": (if self.supports_brightness { self.brightness_command_topic.clone() } else { None }),
            "brightness_state_topic": (if self.supports_brightness { Some(self.details.get_topic_for_state(Some("brightness"))) } else { None }),
        });
        let mut component_details_json = self.details.to_json(device_opt).await?;
        if let (Value::Object(component_details_map), Value::Object(sensor_map)) =
            (&mut component_details_json, json)
        {
            component_details_map.extend(sensor_map);
        } else {
            return Err(DevicesError::IncorrectJsonStructure);
        }
        let mut map: Map<String, Value> = Map::new();
        map.insert(self.details.id.clone(), component_details_json);
        Ok(Value::Object(map))
    }
}

#[derive(thiserror::Error, Debug)]
pub enum DevicesError {
    #[error("Missing device")]
    MissingDevice,
    #[error("Incorrect JSON structure")]
    IncorrectJsonStructure,
    #[allow(dead_code)]
    #[error("Unknown device error")]
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    #[tokio::test]
    async fn test_device_to_json() {
        let provider =
            crate::sample_device::SampleDeviceProvider::new("Test Device".to_string(), false);
        let mut devices = provider.get_devices().await;
        let device = devices.pop().unwrap();
        let device_json = device.json_for_discovery().await.unwrap();
        let expected_json = json!({
            "device": {
                "identifiers": ["test-device"],
                "name": "Test Device",
                "manufacturer": "Giovanni Bassi",
                "sw_version": env!("CARGO_PKG_VERSION"),
            },
            "origin": {
                "name": "docker-status-mqtt",
                "sw": env!("CARGO_PKG_VERSION"),
                "url": "https://github.com/giggio/docker-status-mqtt",
            },
            "availability_topic": "mqtt-docker/test-device/availability",
            "components": {
                "memory-usage": {
                    "name": "Memory Usage",
                    "device_class": "data_size",
                    "unique_id": "test-device_memory-usage",
                    "state_topic": "mqtt-docker/test-device/memory-usage/state",
                    "icon": "mdi:memory",
                    "json_attributes_topic": "mqtt-docker/test-device/memory-usage_attributes/state",
                    "json_attributes_template": "{{ value_json | tojson }}",
                    "platform": "sensor",
                    "unit_of_measurement": "MB"
                },
                "system-uptime": {
                    "name": "System Uptime",
                    "device_class": "duration",
                    "unique_id": "test-device_system-uptime",
                    "state_topic": "mqtt-docker/test-device/system-uptime/state",
                    "icon": "mdi:clock-outline",
                    "platform": "sensor",
                    "unit_of_measurement": "s"
                },
                "server-mode": {
                    "name": "Server Mode",
                    "device_class": null,
                    "unique_id": "test-device_server-mode",
                    "state_topic": "mqtt-docker/test-device/server-mode/state",
                    "icon": "mdi:server",
                    "platform": "switch",
                    "payload_on": "ON",
                    "payload_off": "OFF",
                    "command_topic": "mqtt-docker/test-device/server-mode/command"
                },
                "living-room-light": {
                    "name": "Living Room Light",
                    "unique_id": "test-device_living-room-light",
                    "state_topic": "mqtt-docker/test-device/living-room-light/state",
                    "icon": "mdi:lightbulb",
                    "platform": "light",
                    "payload_on": "ON",
                    "payload_off": "OFF",
                    "command_topic": "mqtt-docker/test-device/living-room-light/command",
                    "brightness_scale": 100,
                    "brightness_command_topic": "mqtt-docker/test-device/living-room-light_brightness/command",
                    "brightness_state_topic": "mqtt-docker/test-device/living-room-light_brightness/state",
                }
            },
        });
        assert_eq!(
            serde_json::to_string_pretty(&device_json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_state_update_json() -> Result<(), Box<dyn std::error::Error>> {
        let provider =
            crate::sample_device::SampleDeviceProvider::new("Test Device".to_string(), false);
        let mut devices = provider.get_devices().await;
        let device = devices.pop().unwrap();
        let data = device.get_entities_data().await?;
        let data_json = Value::Array(data.into_iter().map(|item|
            json!({
                "topic": item.topic,
                "payload": serde_json::from_str(&item.payload).unwrap_or(Value::String(item.payload.clone())),
            })
        ).collect::<Vec<Value>>());
        let expected_json = json!([
            {
                "payload": {
                    "free": 12288,
                    "is_cached": true,
                    "manufacture_date": "2024-02-07T08:21:56-03:00",
                    "manufacturer": "Toshiba",
                    "total": 16384,
                    "used": 4096
                },
                "topic": "mqtt-docker/test-device/memory-usage_attributes/state"
            },
            {
                "payload": 4096,
                "topic": "mqtt-docker/test-device/memory-usage/state"
            },
            {
                "payload": 2726,
                "topic": "mqtt-docker/test-device/system-uptime/state"
            },
            {
                "payload": "OFF",
                "topic": "mqtt-docker/test-device/server-mode/state",
            },
            {
                "payload": "ON",
                "topic": "mqtt-docker/test-device/living-room-light/state",
            }
        ]);
        assert_eq!(
            serde_json::to_string_pretty(&data_json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
        Ok(())
    }
}
