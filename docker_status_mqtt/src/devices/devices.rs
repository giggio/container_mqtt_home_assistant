use crate::{
    cancellation_token::CancellationToken,
    device_manager::{CommandResult, PublishResult},
    devices::{DeviceProvider, Result, device::Device},
};
use std::{collections::HashMap, fmt::Debug, sync::Arc};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct Devices {
    devices: Arc<Vec<Arc<RwLock<Device>>>>,
    #[allow(dead_code)] // todo: use cancellation_token somewhere or remove it?
    cancellation_token: CancellationToken,
}

impl Devices {
    pub fn new_from_many_devices(devices: Vec<Device>, cancellation_token: CancellationToken) -> Self {
        Devices {
            devices: Arc::new(devices.into_iter().map(|d| Arc::new(RwLock::new(d))).collect()),
            cancellation_token,
        }
    }

    #[cfg(test)]
    pub fn new_from_single_device(device: Device, cancellation_token: CancellationToken) -> Self {
        Devices {
            devices: Arc::new(vec![Arc::new(RwLock::new(device))]),
            cancellation_token,
        }
    }

    pub async fn from_device_providers(
        providers: Vec<Box<dyn DeviceProvider>>,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> Self {
        let mut all_devices = Vec::new();
        for provider in providers {
            let Devices { mut devices, .. } = provider
                .get_devices(availability_topic.clone(), CancellationToken::default())
                .await;
            all_devices.append(Arc::get_mut(&mut devices).unwrap());
        }
        Devices {
            devices: Arc::new(all_devices),
            cancellation_token,
        }
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Arc<RwLock<Device>>> {
        self.devices.iter()
    }

    pub async fn create_discovery_info(&self, discovery_prefix: &str) -> Result<HashMap<String, String>> {
        let mut map = HashMap::new();
        for device_lock in self.devices.iter() {
            let device = device_lock.read().await;
            let discovery_json = device.json_for_discovery().await?.to_string();
            let discovery_topic = device.discovery_topic(discovery_prefix);
            map.insert(discovery_topic, discovery_json);
        }
        Ok(map)
    }

    pub async fn discovery_topics(&self, discovery_prefix: &str) -> Vec<String> {
        let mut topics = Vec::new();
        for device_lock in self.devices.iter() {
            let device = device_lock.read().await;
            topics.push(device.discovery_topic(discovery_prefix));
        }
        topics
    }

    pub async fn command_topics(&self) -> Vec<String> {
        let mut topics = Vec::new();
        for device_lock in self.devices.iter() {
            let device = device_lock.read().await;
            topics.extend(device.command_topics());
        }
        topics
    }

    pub async fn get_entities_data(&self) -> Result<HashMap<String, String>> {
        let mut entities_data = HashMap::<String, String>::new();
        for device_lock in self.devices.iter() {
            let device = device_lock.read().await;
            trace!("Getting entities data for device: {}", device.details.name);
            let data = device.get_entities_data().await?;
            trace!("Got entities data for device: {}: {data:?}", device.details.name);
            for entity_data in data.into_iter() {
                entities_data.insert(entity_data.0.clone(), entity_data.1.clone());
            }
        }
        Ok(entities_data)
    }

    pub async fn deal_with_command(&self, publish_result: &PublishResult) -> Result<Vec<CommandResult>> {
        let mut command_handle_results = Vec::new();
        for device_lock in self.devices.iter() {
            let (command_handle_result, device_name) = {
                let mut device = device_lock.write().await;
                trace!("Checking if device {} can handle command...", device.details.name);
                (
                    device
                        .handle_command(&publish_result.topic, &publish_result.payload)
                        .await?,
                    device.details.name.clone(),
                )
            };
            if command_handle_result.handled {
                trace!(
                    "Device {device_name} handled command on topic: {}, payload: {}",
                    publish_result.topic, publish_result.payload,
                );
            } else {
                trace!(
                    "Device {device_name} did not handle command on topic: {}, payload: {}",
                    publish_result.topic, publish_result.payload,
                );
            }
            command_handle_results.push(command_handle_result);
            trace!("Device {device_name} finished checking command, will now go to next device...");
        }
        Ok(command_handle_results)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.devices.len()
    }
}
