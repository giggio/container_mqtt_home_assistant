use std::sync::Arc;

use crate::{
    cancellation_token::CancellationToken,
    device_manager::CommandResult,
    devices::{
        Button, ButtonDeviceClass, Device, DeviceDetails, DeviceOrigin, DeviceProvider, Devices, EntityDetails,
        EntityDetailsGetter, HandlesData, Light, Number, Result, Sensor, Switch, Text,
    },
    helpers::slugify,
};

use async_trait::async_trait;
use chrono::Utc;
use hashbrown::{HashMap, HashSet};
use rand::Rng;
use serde_json::json;
use tokio::sync::RwLock;

pub struct SampleDeviceProvider {
    device_id: String,
    device_name: String,
    dependent_device_id: String,
    dependent_device_name: String,
    is_random: bool,
}

impl SampleDeviceProvider {
    pub fn new(device_name: impl Into<String>) -> Self {
        let device_name = device_name.into();
        let dependent_device_name = "Dependent device".to_string();
        SampleDeviceProvider {
            device_id: slugify(&device_name),
            device_name,
            dependent_device_id: slugify(&dependent_device_name),
            dependent_device_name,
            is_random: true,
        }
    }

    #[cfg(test)]
    pub fn not_random(self) -> Self {
        Self {
            is_random: false,
            ..self
        }
    }
}

#[async_trait]
impl DeviceProvider for SampleDeviceProvider {
    fn id(&self) -> String {
        "sample_device_provider".to_string()
    }
    #[allow(clippy::too_many_lines)]
    async fn get_devices(&self, availability_topic: String, cancellation_token: CancellationToken) -> Result<Devices> {
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
            self.id(),
            cancellation_token.clone(),
        );
        let memory_sensor = Box::new(Sensor::new_with_details(
            EntityDetails::new(
                main_device.details.identifier.clone(),
                "Memory Usage".to_string(),
                "mdi:memory".to_string(),
            )
            .has_attributes(),
            Some("MB".to_string()),
            Some("data_size".to_string()),
        ));
        let memory_sensor_data = Box::new(MemorySensor {
            is_random: self.is_random,
            state_topic: memory_sensor.details().get_topic_for_state(None),
            attributes_state_topic: memory_sensor.details().get_topic_for_state(Some("attributes")),
        });
        let uptime_sensor = Box::new(Sensor::new(
            main_device.details.identifier.clone(),
            "System Uptime".to_string(),
            "mdi:clock-outline".to_string(),
            "s".to_string(),
            "duration".to_string(),
        ));
        let uptime_sensor_data = Box::new(UptimeSensor {
            state_topic: uptime_sensor.details().get_topic_for_state(None),
            is_random: self.is_random,
        });
        let living_room_light = Box::new(
            Light::new(
                main_device.details.identifier.clone(),
                "Living Room Light".to_string(),
                "mdi:lightbulb".to_string(),
            )
            .support_brightness(100),
        );
        let living_room_light_data = Box::new(LivingRoomLight {
            is_random: self.is_random,
            is_on: false,
            state_topic: living_room_light.details().get_topic_for_state(None),
        });
        let log_text = Box::new(Text::new(
            main_device.details.identifier.clone(),
            "Log text".to_string(),
            "mdi:script-text-outline".to_string(),
        ));
        let log_text_data = Box::new(LogText {
            is_random: self.is_random,
            state_topic: log_text.details().get_topic_for_state(None),
        });
        let server_mode_switch = Box::new(Switch::new(
            main_device.details.identifier.clone(),
            "Server Mode".to_owned(),
            "mdi:server".to_owned(),
            None,
        ));
        let server_mode_switch_data = Box::new(ServerModeSwitch {
            is_random: self.is_random,
            is_on: false,
            state_topic: server_mode_switch.details().get_topic_for_state(None),
            command_topic: server_mode_switch.details().get_topic_for_command(None),
        });
        let some_number = Box::new(Number::new(
            main_device.details.identifier.clone(),
            "Some number".to_owned(),
            "mdi:numeric".to_owned(),
        ));
        let some_number_data = Box::new(SomeNumber {
            is_random: self.is_random,
            state_topic: some_number.details().get_topic_for_state(None),
            command_topic: some_number.details().get_topic_for_command(None),
            value: None,
        });
        let some_button = Box::new(
            Button::new(
                main_device.details.identifier.clone(),
                "Some Button".to_string(),
                "mdi:button-pointer".to_string(),
            )
            .with_device_class(ButtonDeviceClass::Identify),
        );
        let some_button_data = Box::new(SomeButton {
            command_topic: some_button.details().get_topic_for_command(None),
        });
        main_device.entities = vec![
            living_room_light,
            log_text,
            memory_sensor,
            server_mode_switch,
            uptime_sensor,
            some_button,
            some_number,
        ];
        main_device.data_handlers = vec![
            living_room_light_data,
            log_text_data,
            memory_sensor_data,
            server_mode_switch_data,
            uptime_sensor_data,
            some_button_data,
            some_number_data,
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
            self.id(),
            cancellation_token.clone(),
        );

        let log_text = Box::new(Text::new(
            dependent_device.details.identifier.clone(),
            "Some text".to_string(),
            "mdi:card-text-outline".to_string(),
        ));
        let log_text_data = Box::new(LogText {
            is_random: self.is_random,
            state_topic: log_text.details().get_topic_for_state(None),
        });
        dependent_device.entities = vec![log_text];
        dependent_device.data_handlers = vec![log_text_data];
        Ok(Devices::new_from_many_devices(
            vec![main_device, dependent_device],
            cancellation_token,
        ))
    }

    async fn remove_missing_devices(
        &self,
        _devices: &Devices,
        _cancellation_token: CancellationToken,
    ) -> Result<Vec<Arc<RwLock<Device>>>> {
        Ok(vec![])
    }

    async fn add_discovered_devices(
        &self,
        _devices: &Devices,
        _availability_topic: String,
        _cancellation_token: CancellationToken,
    ) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }
}

#[derive(Debug)]
struct SomeNumber {
    is_random: bool,
    value: Option<u32>,
    state_topic: String,
    command_topic: String,
}
#[async_trait]
impl HandlesData for SomeNumber {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        error!("MemorySensor received event for topic {topic} and payload {payload}");
        self.value = payload.parse().ok();
        return Ok(CommandResult {
            handled: true,
            state_update_topics: Some(hashmap! {self.state_topic.clone() => self.value.unwrap_or(100).to_string()}),
        });
    }
    async fn get_entity_data(&self, _cancellation_token: CancellationToken) -> Result<HashMap<String, String>> {
        let number = if self.is_random {
            rand::random::<u32>() % 100
        } else {
            self.value.unwrap_or(100)
        };
        Ok(hashmap! { self.state_topic.clone() => number.to_string() })
    }
}

#[derive(Debug)]
struct MemorySensor {
    is_random: bool,
    state_topic: String,
    attributes_state_topic: String,
}
#[async_trait]
impl HandlesData for MemorySensor {
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        error!("MemorySensor received event for topic {topic} and payload {payload}");
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
    async fn get_entity_data(&self, _cancellation_token: CancellationToken) -> Result<HashMap<String, String>> {
        let memory_usage = if self.is_random {
            1024 + (rand::random::<u32>() % 512)
        } else {
            4096
        };

        Ok(hashmap! {
            self.attributes_state_topic.clone() => json!({
                "total": 16384,
                "used": memory_usage,
                "free": 16384 - memory_usage,
                "is_cached": if self.is_random { rand::random::<bool>() } else { true },
                "manufacturer": "Toshiba",
                "manufacture_date": "2024-02-07T08:21:56-03:00",
            }).to_string(),
            self.state_topic.clone() => memory_usage.to_string()
        })
    }
}

#[derive(Debug)]
struct UptimeSensor {
    is_random: bool,
    state_topic: String,
}
#[async_trait]
impl HandlesData for UptimeSensor {
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        error!("UptimeSensor received event for topic {topic} and payload {payload}",);
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
    async fn get_entity_data(&self, _cancellation_token: CancellationToken) -> Result<HashMap<String, String>> {
        let uptime = if self.is_random {
            Utc::now().timestamp() % 86400
        } else {
            2726
        };
        Ok(hashmap! {self.state_topic.clone() => uptime.to_string()})
    }
}

#[derive(Debug)]
struct LogText {
    is_random: bool,
    state_topic: String,
}
#[async_trait]
impl HandlesData for LogText {
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        error!("LogText received event for topic {topic} and payload {payload}");
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
    async fn get_entity_data(&self, _cancellation_token: CancellationToken) -> Result<HashMap<String, String>> {
        let log_text = if self.is_random {
            (&mut rand::rng())
                .sample_iter(rand::distr::Alphanumeric)
                .take((rand::random::<u32>() % 50) as usize + 1)
                .map(char::from)
                .collect()
        } else {
            "This is a log text".to_string()
        };
        Ok(hashmap! {self.state_topic.clone() => log_text.clone()})
    }
}

#[derive(Debug)]
struct ServerModeSwitch {
    is_random: bool,
    is_on: bool,
    state_topic: String,
    command_topic: String,
}
#[async_trait]
impl HandlesData for ServerModeSwitch {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        self.is_random = false;
        self.is_on = "ON" == payload;
        error!("ServerModeSwitchEntity received event for topic {topic} and payload {payload}");
        Ok(CommandResult {
            handled: true,
            state_update_topics: Some(self.get_entity_data(cancellation_token).await?),
        })
    }
    async fn get_entity_data(&self, _cancellation_token: CancellationToken) -> Result<HashMap<String, String>> {
        let is_on = if self.is_random {
            if rand::random::<bool>() { "ON" } else { "OFF" }
        } else if self.is_on {
            "ON"
        } else {
            "OFF"
        };
        Ok(hashmap! {self.state_topic.clone() => is_on.to_string()})
    }
}

#[derive(Debug)]
struct LivingRoomLight {
    is_random: bool,
    is_on: bool,
    state_topic: String,
}
#[async_trait]
impl HandlesData for LivingRoomLight {
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        self.is_random = false;
        self.is_on = "ON" == payload;
        error!("LivingRoomLight received event for topic {topic} and payload {payload}");
        return Ok(CommandResult {
            handled: true,
            state_update_topics: Some(self.get_entity_data(cancellation_token).await?),
        });
    }
    async fn get_entity_data(&self, _cancellation_token: CancellationToken) -> Result<HashMap<String, String>> {
        let light_state = if self.is_random {
            if rand::random::<bool>() { "ON" } else { "OFF" }
        } else if self.is_on {
            "ON"
        } else {
            "OFF"
        };
        Ok(hashmap! {self.state_topic.clone() => light_state.to_string()})
    }
}

#[derive(Debug)]
struct SomeButton {
    command_topic: String,
}
#[async_trait]
impl HandlesData for SomeButton {
    fn get_command_topics(&self) -> Vec<&str> {
        vec![&self.command_topic]
    }
    async fn do_handle_command(
        &mut self,
        topic: &str,
        payload: &str,
        _cancellation_token: CancellationToken,
    ) -> Result<CommandResult> {
        error!("SomeButton received event for topic {topic} and payload {payload}");
        return Ok(CommandResult {
            handled: true,
            state_update_topics: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn entity_server_mode_switch_update() {
        let mut server_mode_switch = ServerModeSwitch {
            state_topic: "device_1/server_mode/state".to_string(),
            command_topic: "device_1/server_mode/command".to_string(),
            is_random: false,
            is_on: false,
        };
        let data = server_mode_switch
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();
        let entity_data = data.iter().last().unwrap();
        assert_eq!(entity_data.0, "device_1/server_mode/state");
        assert_eq!(entity_data.1, "OFF");
        let command_result = server_mode_switch
            .do_handle_command("device_1/server_mode/command", "ON", CancellationToken::default())
            .await
            .unwrap();
        assert!(command_result.handled);
        let state_updates = command_result.state_update_topics.unwrap();
        let updated_state = state_updates.get("device_1/server_mode/state").unwrap();
        assert_eq!(updated_state, "ON");
        let data = server_mode_switch
            .get_entity_data(CancellationToken::default())
            .await
            .unwrap();
        let entity_data = data.iter().last().unwrap();
        assert_eq!(entity_data.0, "device_1/server_mode/state");
        assert_eq!(entity_data.1, "ON");
    }

    #[tokio::test]
    async fn entity_server_mode_switch_handle_command() {
        let mut server_mode_switch = ServerModeSwitch {
            state_topic: "device_1/server_mode/state".to_string(),
            command_topic: "device_1/server_mode/command".to_string(),
            is_random: false,
            is_on: false,
        };
        let result = server_mode_switch
            .handle_command("device_1/server_mode/command", "ON", CancellationToken::default())
            .await
            .unwrap();
        assert!(result.handled);
        assert_eq!(
            result.state_update_topics.unwrap(),
            hashmap! {"device_1/server_mode/state".to_string() => "ON".to_string() }
        );
    }

    #[tokio::test]
    async fn entity_server_mode_switch_toggle() {
        let mut server_mode_switch = ServerModeSwitch {
            state_topic: "device_1/server_mode/state".to_string(),
            command_topic: "device_1/server_mode/command".to_string(),
            is_random: false,
            is_on: false,
        };
        let result_on = server_mode_switch
            .handle_command("device_1/server_mode/command", "ON", CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(
            result_on,
            CommandResult {
                handled: true,
                state_update_topics: Some(hashmap! {"device_1/server_mode/state".to_string() => "ON".to_string() }),
            }
        );

        let result_off = server_mode_switch
            .handle_command("device_1/server_mode/command", "OFF", CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(
            result_off,
            CommandResult {
                handled: true,
                state_update_topics: Some(hashmap! {"device_1/server_mode/state".to_string() => "OFF".to_string() }),
            }
        );
    }

    #[tokio::test]
    async fn entity_does_not_handle_wrong_topic() {
        let mut server_mode_switch = ServerModeSwitch {
            state_topic: "device_1/server_mode/state".to_string(),
            command_topic: "device_1/server_mode/command".to_string(),
            is_random: false,
            is_on: false,
        };
        let result = server_mode_switch
            .handle_command("wrong/topic", "ON", CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(
            result,
            CommandResult {
                handled: false,
                state_update_topics: None
            }
        );
    }

    #[tokio::test]
    async fn test_sample_device_provider_creates_devices() {
        let provider = SampleDeviceProvider::new("Test Device").not_random();
        let devices = provider
            .get_devices("node_id/availability".to_string(), CancellationToken::default())
            .await
            .unwrap();
        assert_eq!(devices.len().await, 2);
        let mut devices_vec = devices.into_vec().unwrap();
        devices_vec.sort_by_key(|d| d.details.identifier.clone());
        let device = devices_vec.last().unwrap();
        assert_eq!(device.details.name, "Test Device");
        assert_eq!(device.details.identifier, "test_device");
        assert_eq!(device.availability_topic, "node_id/availability");
        assert!(!device.entities.is_empty());
    }

    #[tokio::test]
    async fn test_device_json_for_discovery_sample_device() {
        let provider = SampleDeviceProvider::new("Test Device").not_random();
        let devices = provider
            .get_devices(
                "device_manager_id/availability".to_owned(),
                CancellationToken::default(),
            )
            .await
            .unwrap();
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
            "availability": [{ "topic": "device_manager_id/availability" }],
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
                "some_button": {
                  "command_topic": "test_device/some_button/command",
                  "device_class": "identify",
                  "icon": "mdi:button-pointer",
                  "name": "Some Button",
                  "platform": "button",
                  "state_topic": "test_device/some_button/state",
                  "unique_id": "test_device_some_button",
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
                "name": "Dependent device",
                "manufacturer": "Giovanni Bassi",
                "sw_version": env!("CARGO_PKG_VERSION"),
                "via_device": "test_device",
            },
            "origin": {
                "name": "docker-status-mqtt",
                "sw": env!("CARGO_PKG_VERSION"),
                "url": "https://github.com/giggio/docker-status-mqtt",
            },
            "availability": [{ "topic": "device_manager_id/availability" }],
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
        let provider = SampleDeviceProvider::new("Test Device").not_random();
        let devices = provider
            .get_devices("test_device/availability".to_owned(), CancellationToken::default())
            .await
            .unwrap();
        let data = devices.get_entities_data().await.unwrap();
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
                "test_device/living_room_light/state".to_string() => "OFF".to_string(),
                "test_device/log_text/state".to_string() => "This is a log text".to_string(),
                "dependent_device/some_text/state".to_string() => "This is a log text".to_string(),
            }
        );
    }
}
