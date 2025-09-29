use std::fmt::Display;

use crate::{device_manager::CommandResult, ha_devices::*};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;
use slug::slugify;

pub struct SampleDeviceProvider {
    device_id: String,
    device_name: String,
    is_random: bool,
}

impl SampleDeviceProvider {
    pub fn new(device_name: String, is_random: bool) -> Self {
        SampleDeviceProvider {
            device_id: slugify(&device_name),
            device_name,
            is_random,
        }
    }
}

#[derive(Debug)]
struct MemorySensorDataProvider {
    is_random: bool,
}
impl Display for MemorySensorDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MemorySensorDataProvider{{is_random: {}}}",
            self.is_random
        )
    }
}
#[async_trait]
impl EntityDataProvider for MemorySensorDataProvider {
    async fn get_entity_data(&self, entity: &dyn Entity) -> Result<Vec<EntityData>, DevicesError> {
        let memory_usage = if self.is_random {
            1024 + (rand::random::<u32>() % 512)
        } else {
            4096
        };
        let details = entity.get_data().details();
        Ok(vec![
            EntityData {
                topic: details.get_topic_for_state(Some("attributes")),
                payload: json!({
                    "total": 16384,
                    "used": memory_usage,
                    "free": 16384 - memory_usage,
                    "is_cached": if self.is_random { rand::random::<bool>() } else { true },
                    "manufacturer": "Toshiba",
                    "manufacture_date": "2024-02-07T08:21:56-03:00",
                })
                .to_string(),
            },
            EntityData {
                topic: details.get_topic_for_state(None),
                payload: memory_usage.to_string(),
            },
        ])
    }
}

#[derive(Debug)]
struct UptimeSensorDataProvider {
    is_random: bool,
}
impl Display for UptimeSensorDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "UptimeSensorDataProvider{{is_random: {}}}",
            self.is_random
        )
    }
}
#[async_trait]
impl EntityDataProvider for UptimeSensorDataProvider {
    async fn get_entity_data(&self, entity: &dyn Entity) -> Result<Vec<EntityData>, DevicesError> {
        let uptime = if self.is_random {
            Utc::now().timestamp() % 86400
        } else {
            2726
        };
        Ok(vec![EntityData {
            topic: entity.get_data().details().get_topic_for_state(None),
            payload: uptime.to_string(),
        }])
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
    async fn get_entity_data(&self, entity: &dyn Entity) -> Result<Vec<EntityData>, DevicesError> {
        let is_on = if self.is_random {
            if rand::random::<bool>() { "ON" } else { "OFF" }
        } else if self.is_on {
            "ON"
        } else {
            "OFF"
        };
        Ok(vec![EntityData {
            topic: entity.get_data().details().get_topic_for_state(None),
            payload: is_on.to_string(),
        }])
    }
}

#[derive(Debug)]
struct LivingRoomLightDataProvider {
    is_random: bool,
}
impl Display for LivingRoomLightDataProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LivingRoomLightDataProvider{{is_random: {}}}",
            self.is_random
        )
    }
}
#[async_trait]
impl EntityDataProvider for LivingRoomLightDataProvider {
    async fn get_entity_data(&self, entity: &dyn Entity) -> Result<Vec<EntityData>, DevicesError> {
        let light_state = if self.is_random {
            if rand::random::<bool>() { "ON" } else { "OFF" }
        } else {
            "ON"
        };
        Ok(vec![EntityData {
            topic: entity.get_data().details().get_topic_for_state(None),
            payload: light_state.to_string(),
        }])
    }
}

#[async_trait]
impl DeviceProvider for SampleDeviceProvider {
    async fn get_devices(&self) -> Vec<Device> {
        let node_id = "mqtt-docker".to_string(); // todo: where should this come from?
        let mut device = Device {
            node_id,
            details: DeviceDetails {
                name: self.device_name.clone(),
                identifier: self.device_id.clone(),
                manufacturer: "Giovanni Bassi".to_string(),
                sw_version: env!("CARGO_PKG_VERSION").to_string(),
            },
            origin: DeviceOrigin {
                name: "docker-status-mqtt".to_string(),
                sw: env!("CARGO_PKG_VERSION").to_string(),
                url: "https://github.com/giggio/docker-status-mqtt".to_string(),
            },
            components: vec![],
        };

        device.components = vec![
            Box::new(GenericEntity {
                data_provider: Box::new(MemorySensorDataProvider {
                    is_random: self.is_random,
                }),
                device_information: Box::new(Sensor::new_with_details(
                    ComponentDetails::new(
                        device.node_id.clone(),
                        device.details.identifier.clone(),
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
                    device.node_id.clone(),
                    device.details.identifier.clone(),
                    "System Uptime".to_string(),
                    "mdi:clock-outline".to_string(),
                    "s".to_string(),
                    Some("duration".to_string()),
                )),
            }),
            Box::new(ServerModeSwitchEntity::new(&device, self.is_random).await),
            Box::new(GenericEntity {
                data_provider: Box::new(LivingRoomLightDataProvider {
                    is_random: self.is_random,
                }),
                device_information: Box::new(
                    Light::new(
                        device.node_id.clone(),
                        device.details.identifier.clone(),
                        "Living Room Light".to_string(),
                        "mdi:lightbulb".to_string(),
                    )
                    .await
                    .support_brightness(100)
                    .await,
                ),
            }),
        ];
        vec![device]
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
    ) -> Result<CommandResult, DevicesError> {
        if self
            .device_information
            .details()
            .command_topics
            .contains(&topic.to_owned())
        {
            error!(
                "MyEntity received component {} event for topic {topic} and payload {payload} and HANDLED it",
                self.device_information.details().name
            );
            return Ok(CommandResult {
                handled: true,
                state_update: None,
            });
        }
        trace!(
            "MyEntity received component {} event for topic {topic} and payload {payload} and DID NOT HANDLE it",
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
                    device.node_id.clone(),
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
    ) -> Result<CommandResult, DevicesError> {
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
            "ServerModeSwitchEntity received component {} event for topic {topic} and payload {payload}, data provider now is {}",
            self.device_information.details().name,
            self.data_provider
        );
        Ok(CommandResult {
            handled: true,
            state_update: Some(self.data_provider.get_entity_data(self).await?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn create_device() -> Device {
        Device {
            node_id: "node1".to_string(),
            details: DeviceDetails {
                name: "Device 1".to_string(),
                identifier: "device-1".to_string(),
                manufacturer: "Test Manufacturer".to_string(),
                sw_version: "1.0.0".to_string(),
            },
            origin: DeviceOrigin {
                name: "test-origin".to_string(),
                sw: "1.0.0".to_string(),
                url: "http://example.com".to_string(),
            },
            components: vec![],
        }
    }

    #[tokio::test]
    async fn entity_server_mode_switch_data_provider_update() {
        let device = create_device();
        let mut server_mode_switch = ServerModeSwitchEntity::new(&device, false).await;
        let data = server_mode_switch
            .get_entity_data_provider()
            .get_entity_data(&server_mode_switch as &dyn Entity)
            .await
            .unwrap();
        let entity_data = &data[0];
        assert_eq!(entity_data.topic, "node1/device-1/server-mode/state");
        assert_eq!(entity_data.payload, "OFF");
        server_mode_switch.data_provider = Box::new(ServerModeSwitchDataProvider {
            is_random: false,
            is_on: true,
        });
        let new_data = server_mode_switch
            .get_entity_data_provider()
            .get_entity_data(&server_mode_switch as &dyn Entity)
            .await
            .unwrap();
        let new_entity_data = &new_data[0];
        assert_eq!(new_entity_data.topic, "node1/device-1/server-mode/state");
        assert_eq!(new_entity_data.payload, "ON");
    }

    #[tokio::test]
    async fn entity_server_mode_switch_handle_command() {
        let mut device = create_device();
        device
            .components
            .push(Box::new(ServerModeSwitchEntity::new(&device, true).await));
        let server_mode_switch = device.components[0].as_mut();
        let result = server_mode_switch
            .handle_command("node1/device-1/server-mode/command", "ON")
            .await
            .unwrap();
        assert!(result.handled);
        assert_eq!(
            result.state_update.unwrap(),
            vec![EntityData {
                topic: "node1/device-1/server-mode/state".to_string(),
                payload: "ON".to_string(),
            }]
        );
    }
}
