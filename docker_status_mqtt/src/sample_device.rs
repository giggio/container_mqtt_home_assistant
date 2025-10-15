use std::{collections::HashMap, fmt::Display};

use crate::{
    cancellation_token::CancellationToken,
    device_manager::CommandResult,
    devices::{
        Device, DeviceDetails, DeviceOrigin, DeviceProvider, Devices, Entity, EntityDataProvider, EntityDetails,
        EntityType, Light, Sensor, Switch, Text,
    },
    helpers::*,
};

use async_trait::async_trait;
use chrono::Utc;
use rand::Rng;
use serde_json::json;
use std::result::Result;

pub struct SampleDeviceProvider {
    device_id: String,
    device_name: String,
    dependent_device_id: String,
    dependent_device_name: String,
    is_random: bool,
}

impl SampleDeviceProvider {
    pub fn new(device_name: String, dependent_device_name: String, is_random: bool) -> Self {
        SampleDeviceProvider {
            device_id: slugify(&device_name),
            device_name,
            dependent_device_id: slugify(&dependent_device_name),
            dependent_device_name,
            is_random,
        }
    }
}

#[async_trait]
impl DeviceProvider for SampleDeviceProvider {
    async fn get_devices(&self, availability_topic: String, cancellation_token: CancellationToken) -> Devices {
        let mut main_device = Device::new(
            DeviceDetails {
                name: self.device_name.clone(),
                identifier: self.device_id.clone(),
                manufacturer: "Giovanni Bassi".to_string(),
                sw_version: env!("CARGO_PKG_VERSION").to_string(),
                via_device: None,
            },
            DeviceOrigin {
                name: "docker-status-mqtt".to_string(),
                sw: env!("CARGO_PKG_VERSION").to_string(),
                url: "https://github.com/giggio/docker-status-mqtt".to_string(),
            },
            availability_topic.clone(),
            CancellationToken::default(),
        );
        main_device.entities = vec![
            Box::new(GenericEntity {
                data_provider: Box::new(MemorySensorDataProvider {
                    is_random: self.is_random,
                }),
                device_information: Box::new(Sensor::new_with_details(
                    EntityDetails::new(
                        main_device.details.identifier.clone(),
                        "Memory Usage".to_string(),
                        "mdi:memory".to_string(),
                    )
                    .has_attributes(),
                    "MB".to_string(),
                    Some("data_size".to_string()),
                )),
            }),
            Box::new(GenericEntity {
                data_provider: Box::new(UptimeSensorDataProvider {
                    is_random: self.is_random,
                }),
                device_information: Box::new(Sensor::new(
                    main_device.details.identifier.clone(),
                    "System Uptime".to_string(),
                    "mdi:clock-outline".to_string(),
                    "s".to_string(),
                    Some("duration".to_string()),
                )),
            }),
            Box::new(ServerModeSwitchEntity::new(&main_device, self.is_random).await),
            Box::new(GenericEntity {
                data_provider: Box::new(LivingRoomLightDataProvider {
                    is_random: self.is_random,
                }),
                device_information: Box::new(
                    Light::new(
                        main_device.details.identifier.clone(),
                        "Living Room Light".to_string(),
                        "mdi:lightbulb".to_string(),
                    )
                    .await
                    .support_brightness(100)
                    .await,
                ),
            }),
            Box::new(GenericEntity {
                data_provider: Box::new(LogTextDataProvider {
                    is_random: self.is_random,
                }),
                device_information: Box::new(Text::new(
                    main_device.details.identifier.clone(),
                    "Log text".to_string(),
                    "mdi:script-text-outline".to_string(),
                )),
            }),
        ];
        let mut dependent_device = Device::new(
            DeviceDetails {
                name: self.dependent_device_name.clone(),
                identifier: self.dependent_device_id.clone(),
                manufacturer: "Giovanni Bassi".to_string(),
                sw_version: env!("CARGO_PKG_VERSION").to_string(),
                via_device: Some(main_device.details.identifier.clone()),
            },
            DeviceOrigin {
                name: "docker-status-mqtt".to_string(),
                sw: env!("CARGO_PKG_VERSION").to_string(),
                url: "https://github.com/giggio/docker-status-mqtt".to_string(),
            },
            availability_topic,
            CancellationToken::default(),
        );
        dependent_device.entities = vec![Box::new(GenericEntity {
            data_provider: Box::new(LogTextDataProvider {
                is_random: self.is_random,
            }),
            device_information: Box::new(Text::new(
                dependent_device.details.identifier.clone(),
                "Some text".to_string(),
                "mdi:card-text-outline".to_string(),
            )),
        })];
        Devices::new_from_many_devices(vec![main_device, dependent_device], cancellation_token)
    }
}

#[derive(Debug)]
struct MemorySensorDataProvider {
    is_random: bool,
}
impl Display for MemorySensorDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MemorySensorDataProvider{{is_random: {}}}", self.is_random)
    }
}
#[async_trait]
impl EntityDataProvider for MemorySensorDataProvider {
    async fn get_entity_data(
        &self,
        entity: &dyn Entity,
        _cancellation_token: CancellationToken,
    ) -> Result<HashMap<String, String>, crate::devices::Error> {
        let memory_usage = if self.is_random {
            1024 + (rand::random::<u32>() % 512)
        } else {
            4096
        };
        let details = entity.get_data().details();
        Ok(hashmap! {
                details.get_topic_for_state(Some("attributes")) => json!({
                    "total": 16384,
                    "used": memory_usage,
                    "free": 16384 - memory_usage,
                    "is_cached": if self.is_random { rand::random::<bool>() } else { true },
                    "manufacturer": "Toshiba",
                    "manufacture_date": "2024-02-07T08:21:56-03:00",
                })
                .to_string(),
                details.get_topic_for_state(None) => memory_usage.to_string()
        })
    }
}

#[derive(Debug)]
struct UptimeSensorDataProvider {
    is_random: bool,
}
impl Display for UptimeSensorDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UptimeSensorDataProvider{{is_random: {}}}", self.is_random)
    }
}
#[async_trait]
impl EntityDataProvider for UptimeSensorDataProvider {
    async fn get_entity_data(
        &self,
        entity: &dyn Entity,
        _cancellation_token: CancellationToken,
    ) -> Result<HashMap<String, String>, crate::devices::Error> {
        let uptime = if self.is_random {
            Utc::now().timestamp() % 86400
        } else {
            2726
        };
        Ok(hashmap! {entity.get_data().details().get_topic_for_state(None) => uptime.to_string()})
    }
}

#[derive(Debug)]
struct LogTextDataProvider {
    is_random: bool,
}
impl Display for LogTextDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LogTextDataProvider {{is_random: {}}}", self.is_random)
    }
}
#[async_trait]
impl EntityDataProvider for LogTextDataProvider {
    async fn get_entity_data(
        &self,
        entity: &dyn Entity,
        _cancellation_token: CancellationToken,
    ) -> Result<HashMap<String, String>, crate::devices::Error> {
        let log_text = if self.is_random {
            (&mut rand::rng())
                .sample_iter(rand::distr::Alphanumeric)
                .take((rand::random::<u32>() % 50) as usize + 1)
                .map(char::from)
                .collect()
        } else {
            "This is a log text".to_string()
        };
        Ok(hashmap! {entity.get_data().details().get_topic_for_state(None) => log_text.to_string()})
    }
}

#[derive(Debug)]
struct ServerModeSwitchDataProvider {
    is_random: bool,
    is_on: bool,
}

impl Display for ServerModeSwitchDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ServerModeSwitchDataProvider{{is_random: {}, is_on: {}}}",
            self.is_random, self.is_on
        )
    }
}
#[async_trait]
impl EntityDataProvider for ServerModeSwitchDataProvider {
    async fn get_entity_data(
        &self,
        entity: &dyn Entity,
        _cancellation_token: CancellationToken,
    ) -> Result<HashMap<String, String>, crate::devices::Error> {
        let is_on = if self.is_random {
            if rand::random::<bool>() { "ON" } else { "OFF" }
        } else if self.is_on {
            "ON"
        } else {
            "OFF"
        };
        Ok(hashmap! {entity.get_data().details().get_topic_for_state(None) => is_on.to_string()})
    }
}

#[derive(Debug)]
struct LivingRoomLightDataProvider {
    is_random: bool,
}
impl Display for LivingRoomLightDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LivingRoomLightDataProvider{{is_random: {}}}", self.is_random)
    }
}
#[async_trait]
impl EntityDataProvider for LivingRoomLightDataProvider {
    async fn get_entity_data(
        &self,
        entity: &dyn Entity,
        _cancellation_token: CancellationToken,
    ) -> Result<HashMap<String, String>, crate::devices::Error> {
        let light_state = if self.is_random {
            if rand::random::<bool>() { "ON" } else { "OFF" }
        } else {
            "ON"
        };
        Ok(hashmap! {entity.get_data().details().get_topic_for_state(None) => light_state.to_string()})
    }
}

#[derive(Debug)]
struct GenericEntity {
    device_information: Box<dyn EntityType>,
    data_provider: Box<dyn EntityDataProvider>,
}

#[async_trait]
impl Entity for GenericEntity {
    fn get_entity_data_provider(&self) -> &dyn EntityDataProvider {
        self.data_provider.as_ref()
    }

    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }

    async fn handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<CommandResult, crate::devices::Error> {
        if self
            .device_information
            .details()
            .command_topics
            .contains(&topic.to_owned())
        {
            error!(
                "MyEntity received entity {} event for topic {topic} and payload {payload} and HANDLED it",
                self.device_information.details().name
            );
            return Ok(CommandResult {
                handled: true,
                state_update: None,
            });
        }
        trace!(
            "MyEntity received entity {} event for topic {topic} and payload {payload} and DID NOT HANDLE it",
            self.device_information.details().name
        );
        Ok(CommandResult {
            handled: false,
            state_update: None,
        })
    }
}

#[derive(Debug)]
struct ServerModeSwitchEntity {
    device_information: Box<dyn EntityType>,
    data_provider: Box<dyn EntityDataProvider>,
}
impl ServerModeSwitchEntity {
    pub async fn new(device: &Device, is_random: bool) -> Self {
        ServerModeSwitchEntity {
            device_information: Box::new(
                Switch::new(
                    device.details.identifier.clone(),
                    "Server Mode".to_owned(),
                    "mdi:server".to_owned(),
                    None,
                )
                .await,
            ),
            data_provider: Box::new(ServerModeSwitchDataProvider {
                is_random,
                is_on: false,
            }),
        }
    }
}
#[async_trait]
impl Entity for ServerModeSwitchEntity {
    fn get_entity_data_provider(&self) -> &dyn EntityDataProvider {
        self.data_provider.as_ref()
    }

    fn get_data(&self) -> &dyn EntityType {
        self.device_information.as_ref()
    }

    async fn handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        cancellation_token: CancellationToken,
    ) -> Result<CommandResult, crate::devices::Error> {
        if !self
            .device_information
            .details()
            .command_topics
            .contains(&topic.to_owned())
        {
            return Ok(CommandResult {
                handled: false,
                state_update: None,
            });
        }
        self.data_provider = Box::new(ServerModeSwitchDataProvider {
            is_random: false,
            is_on: "ON" == payload,
        });
        error!(
            "ServerModeSwitchEntity received entity {} event for topic {topic} and payload {payload}, data provider now is {}",
            self.device_information.details().name,
            self.data_provider
        );
        Ok(CommandResult {
            handled: true,
            state_update: Some(self.data_provider.get_entity_data(self, cancellation_token).await?),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::devices::Entity;

    use super::*;
    use pretty_assertions::assert_eq;

    fn create_device() -> Device {
        Device::new(
            DeviceDetails {
                name: "Device 1".to_string(),
                identifier: "device_1".to_string(),
                manufacturer: "Test Manufacturer".to_string(),
                sw_version: "1.0.0".to_string(),
                via_device: None,
            },
            DeviceOrigin {
                name: "test-origin".to_string(),
                sw: "1.0.0".to_string(),
                url: "http://example.com".to_string(),
            },
            "dev1/availability_topic".to_string(),
            CancellationToken::default(),
        )
    }

    #[tokio::test]
    async fn entity_server_mode_switch_data_provider_update() {
        let device = create_device();
        let mut server_mode_switch = ServerModeSwitchEntity::new(&device, false).await;
        let data = server_mode_switch
            .get_entity_data_provider()
            .get_entity_data(&server_mode_switch as &dyn Entity, CancellationToken::default())
            .await
            .unwrap();
        let entity_data = data.iter().last().unwrap();
        assert_eq!(entity_data.0, "device_1/server_mode/state");
        assert_eq!(entity_data.1, "OFF");
        server_mode_switch.data_provider = Box::new(ServerModeSwitchDataProvider {
            is_random: false,
            is_on: true,
        });
        let new_data = server_mode_switch
            .get_entity_data_provider()
            .get_entity_data(&server_mode_switch as &dyn Entity, CancellationToken::default())
            .await
            .unwrap();
        let new_entity_data = new_data.iter().last().unwrap();
        assert_eq!(new_entity_data.0, "device_1/server_mode/state");
        assert_eq!(new_entity_data.1, "ON");
    }

    #[tokio::test]
    async fn entity_server_mode_switch_handle_command() {
        let mut device = create_device();
        device
            .entities
            .push(Box::new(ServerModeSwitchEntity::new(&device, true).await));
        let server_mode_switch = device.entities[0].as_mut();
        let result = server_mode_switch
            .handle_command("device_1/server_mode/command", "ON", CancellationToken::default())
            .await
            .unwrap();
        assert!(result.handled);
        assert_eq!(
            result.state_update.unwrap(),
            hashmap! {"device_1/server_mode/state".to_string() => "ON".to_string()}
        );
    }

    #[tokio::test]
    async fn entity_server_mode_switch_toggle() {
        let mut device = create_device();
        device
            .entities
            .push(Box::new(ServerModeSwitchEntity::new(&device, false).await));
        let server_mode_switch = device.entities[0].as_mut();

        // Turn ON
        let result_on = server_mode_switch
            .handle_command("device_1/server_mode/command", "ON", CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(
            result_on,
            CommandResult {
                handled: true,
                state_update: Some(hashmap! {"device_1/server_mode/state".to_string() => "ON".to_string()})
            }
        );

        // Turn OFF
        let result_off = server_mode_switch
            .handle_command("device_1/server_mode/command", "OFF", CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(
            result_off,
            CommandResult {
                handled: true,
                state_update: Some(hashmap! {"device_1/server_mode/state".to_string() => "OFF".to_string()})
            }
        );
    }

    #[tokio::test]
    async fn entity_does_not_handle_wrong_topic() {
        let mut device = create_device();
        device
            .entities
            .push(Box::new(ServerModeSwitchEntity::new(&device, false).await));
        let server_mode_switch = device.entities[0].as_mut();

        let result = server_mode_switch
            .handle_command("wrong/topic", "ON", CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(
            result,
            CommandResult {
                handled: false,
                state_update: None
            }
        );
    }

    #[tokio::test]
    async fn test_sample_device_provider_creates_devices() {
        let provider = SampleDeviceProvider::new("Test Device".to_string(), "Dependent Device".to_string(), false);
        let devices = provider
            .get_devices("node_id/availability".to_string(), CancellationToken::default())
            .await;

        assert_eq!(devices.len(), 2);
        let device = &devices.iter().next().unwrap().read().await;
        assert_eq!(device.details.name, "Test Device");
        assert_eq!(device.details.identifier, "test_device");
        assert_eq!(device.availability_topic, "node_id/availability");
        assert!(!device.entities.is_empty());
    }

    #[tokio::test]
    async fn test_device_json_for_discovery_sample_device() {
        let provider = crate::sample_device::SampleDeviceProvider::new(
            "Test Device".to_string(),
            "Dependent Device".to_string(),
            false,
        );
        let devices = provider
            .get_devices(
                "device_manager_id/availability".to_owned(),
                CancellationToken::default(),
            )
            .await;
        let discovery_info = devices.create_discovery_info("homeassistant").await.unwrap();
        assert_eq!(discovery_info.len(), 2);
        let main_device_json = serde_json::from_str::<serde_json::Value>(
            discovery_info.get("homeassistant/device/test_device/config").unwrap(),
        )
        .unwrap();
        let expected_main_device_json = json!({
            "device": {
                "identifiers": ["test_device"],
                "name": "Test Device",
                "manufacturer": "Giovanni Bassi",
                "sw_version": env!("CARGO_PKG_VERSION"),
            },
            "origin": {
                "name": "docker-status-mqtt",
                "sw": env!("CARGO_PKG_VERSION"),
                "url": "https://github.com/giggio/docker-status-mqtt",
            },
            "availability_topic": "device_manager_id/availability",
            "components": {
                "memory_usage": {
                    "name": "Memory Usage",
                    "device_class": "data_size",
                    "unique_id": "test_device_memory_usage",
                    "state_topic": "test_device/memory_usage/state",
                    "icon": "mdi:memory",
                    "json_attributes_topic": "test_device/memory_usage_attributes/state",
                    "json_attributes_template": "{{ value_json | tojson }}",
                    "platform": "sensor",
                    "unit_of_measurement": "MB"
                },
                "system_uptime": {
                    "name": "System Uptime",
                    "device_class": "duration",
                    "unique_id": "test_device_system_uptime",
                    "state_topic": "test_device/system_uptime/state",
                    "icon": "mdi:clock-outline",
                    "platform": "sensor",
                    "unit_of_measurement": "s"
                },
                "server_mode": {
                    "name": "Server Mode",
                    "device_class": null,
                    "unique_id": "test_device_server_mode",
                    "state_topic": "test_device/server_mode/state",
                    "icon": "mdi:server",
                    "platform": "switch",
                    "payload_on": "ON",
                    "payload_off": "OFF",
                    "command_topic": "test_device/server_mode/command"
                },
                "living_room_light": {
                    "name": "Living Room Light",
                    "unique_id": "test_device_living_room_light",
                    "state_topic": "test_device/living_room_light/state",
                    "icon": "mdi:lightbulb",
                    "platform": "light",
                    "payload_on": "ON",
                    "payload_off": "OFF",
                    "command_topic": "test_device/living_room_light/command",
                    "brightness_scale": 100,
                    "brightness_command_topic": "test_device/living_room_light_brightness/command",
                    "brightness_state_topic": "test_device/living_room_light_brightness/state",
                },
               "log_text": {
                   "command_topic": "test_device/log_text/command",
                   "icon": "mdi:script-text-outline",
                   "mode": "text",
                   "name": "Log text",
                   "platform": "text",
                   "state_topic": "test_device/log_text/state",
                   "unique_id": "test_device_log_text"
                },
           },
        });
        assert_eq!(
            serde_json::to_string_pretty(&main_device_json).unwrap(),
            serde_json::to_string_pretty(&expected_main_device_json).unwrap()
        );
        let dependent_device_json = serde_json::from_str::<serde_json::Value>(
            discovery_info
                .get("homeassistant/device/dependent_device/config")
                .unwrap(),
        )
        .unwrap();
        let expected_dependent_device_json = json!({
            "device": {
                "identifiers": ["dependent_device"],
                "name": "Dependent Device",
                "manufacturer": "Giovanni Bassi",
                "sw_version": env!("CARGO_PKG_VERSION"),
                "via_device": "test_device",
            },
            "origin": {
                "name": "docker-status-mqtt",
                "sw": env!("CARGO_PKG_VERSION"),
                "url": "https://github.com/giggio/docker-status-mqtt",
            },
            "availability_topic": "device_manager_id/availability",
            "components": {
               "some_text": {
                   "command_topic": "dependent_device/some_text/command",
                   "icon": "mdi:card-text-outline",
                   "mode": "text",
                   "name": "Some text",
                   "platform": "text",
                   "state_topic": "dependent_device/some_text/state",
                   "unique_id": "dependent_device_some_text"
                },
           },
        });
        assert_eq!(
            serde_json::to_string_pretty(&dependent_device_json).unwrap(),
            serde_json::to_string_pretty(&expected_dependent_device_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_device_get_entities_data() {
        let provider = crate::sample_device::SampleDeviceProvider::new(
            "Test Device".to_string(),
            "Dependent Device".to_string(),
            false,
        );
        let devices = provider
            .get_devices("test_device/availability".to_owned(), CancellationToken::default())
            .await;
        let device = devices.iter().next().unwrap().read().await;
        let data = device.get_entities_data().await.unwrap();
        assert_eq!(
            data,
            hashmap! {
                "test_device/memory_usage/state".to_string() => "4096".to_string(),
                "test_device/memory_usage_attributes/state".to_string() => json!({
                    "total": 16384,
                    "used": 4096,
                    "free": 12288,
                    "is_cached": true,
                    "manufacturer": "Toshiba",
                    "manufacture_date": "2024-02-07T08:21:56-03:00",
                })
                .to_string(),
                "test_device/system_uptime/state".to_string() => "2726".to_string(),
                "test_device/server_mode/state".to_string() => "OFF".to_string(),
                "test_device/living_room_light/state".to_string() => "ON".to_string(),
                "test_device/log_text/state".to_string() => "This is a log text".to_string(),
            }
        );
    }
}
