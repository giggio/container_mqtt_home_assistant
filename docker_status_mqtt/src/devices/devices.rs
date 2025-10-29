use crate::{
    cancellation_token::CancellationToken,
    device_manager::PublishResult,
    devices::{DeviceProvider, Result, device::Device},
};
use hashbrown::HashMap;
use std::{fmt::Debug, sync::Arc};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct Devices {
    devices: Arc<HashMap<String, Arc<RwLock<Device>>>>,
    #[allow(dead_code)] // todo: use cancellation_token somewhere or remove it?
    cancellation_token: CancellationToken,
}

impl Devices {
    pub fn new_from_many_devices(devices: Vec<Device>, cancellation_token: CancellationToken) -> Self {
        let devices = Arc::new(HashMap::<_, _>::from_iter(
            devices
                .into_iter()
                .map(|d| (d.details.identifier.clone(), Arc::new(RwLock::new(d)))),
        ));
        Devices {
            devices,
            cancellation_token,
        }
    }

    #[cfg(test)]
    pub fn new_from_single_device(device: Device, cancellation_token: CancellationToken) -> Self {
        Devices {
            devices: Arc::new(hashmap! {device.details.identifier.clone() => Arc::new(RwLock::new(device))}),
            cancellation_token,
        }
    }

    pub async fn from_device_providers(
        providers: Vec<Box<dyn DeviceProvider>>,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> Result<Self> {
        let mut all_devices = HashMap::new();
        for provider in providers {
            let Devices { mut devices, .. } = provider
                .get_devices(availability_topic.clone(), CancellationToken::default())
                .await?;
            all_devices.extend(Arc::get_mut(&mut devices).unwrap().drain());
        }
        Ok(Devices {
            devices: Arc::new(all_devices),
            cancellation_token,
        })
    }

    pub fn iter(&self) -> hashbrown::hash_map::Values<'_, String, Arc<RwLock<Device>>> {
        self.devices.values()
    }

    pub fn into_iter(self) -> Option<hashbrown::hash_map::IntoValues<String, Arc<RwLock<Device>>>> {
        Arc::<_>::into_inner(self.devices).map(|devices| devices.into_values())
    }

    #[allow(dead_code)] // used in tests and possibly useful elsewhere
    pub async fn into_vec(self) -> Option<Vec<Device>> {
        let locked_devices = self.into_iter().unwrap().collect::<Vec<_>>();
        let mut devices = vec![];
        for device_lock in locked_devices {
            match Arc::<_>::into_inner(device_lock) {
                Some(rwlock) => {
                    let device = rwlock.into_inner();
                    devices.push(device);
                }
                None => {
                    return None;
                }
            }
        }
        Some(devices)
    }

    pub async fn create_discovery_info(&self, discovery_prefix: &str) -> Result<HashMap<String, String>> {
        let mut map = HashMap::new();
        for device_lock in self.iter() {
            let device = device_lock.read().await;
            let discovery_json = device.json_for_discovery().await?.to_string();
            let discovery_topic = device.discovery_topic(discovery_prefix);
            map.insert(discovery_topic, discovery_json);
        }
        Ok(map)
    }

    pub async fn discovery_topics(&self, discovery_prefix: &str) -> Vec<String> {
        let mut topics = Vec::new();
        for device_lock in self.iter() {
            let device = device_lock.read().await;
            topics.push(device.discovery_topic(discovery_prefix));
        }
        topics
    }

    pub async fn command_topics(&self) -> Vec<String> {
        let mut topics = Vec::new();
        for device_lock in self.iter() {
            let device = device_lock.read().await;
            topics.extend(device.command_topics());
        }
        topics
    }

    pub async fn get_entities_data(&self) -> Result<HashMap<String, String>> {
        let mut entities_data = HashMap::<String, String>::new();
        for device_lock in self.iter() {
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

    pub async fn handle_command(&self, publish_result: &PublishResult) -> Result<HashMap<String, String>> {
        let mut state_updates = HashMap::new();
        let mut handled = false;
        for device_lock in self.iter() {
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
                handled = true;
                trace!(
                    "Device {device_name} handled command on topic: {}, payload: {}",
                    publish_result.topic, publish_result.payload,
                );
                if let Some(state_update_topics) = command_handle_result.state_update_topics {
                    state_updates.extend(state_update_topics);
                }
                break;
            } else {
                trace!(
                    "Device {device_name} did not handle command on topic: {}, payload: {}",
                    publish_result.topic, publish_result.payload,
                );
            }
            trace!(
                "Device {device_name} finished checking command and did not handle it, will now go to next device..."
            );
        }
        if !handled {
            warn!(
                "Received message on unknown topic: {} and payload:\n{}",
                publish_result.topic, publish_result.payload,
            );
        }
        Ok(state_updates)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.devices.len()
    }
}
