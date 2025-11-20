use crate::{
    cancellation_token::CancellationToken,
    device_manager::PublishResult,
    devices::{DeviceProvider, Result, device::Device},
    helpers::*,
};
use hashbrown::{HashMap, HashSet};
use std::{fmt::Debug, sync::Arc};
use tokio::sync::RwLock;

#[derive(Clone, Debug)]
pub struct Devices {
    devices: Arc<RwLock<HashMap<String, Arc<RwLock<Device>>>>>,
    #[allow(dead_code)] // todo: use cancellation_token somewhere or remove it?
    cancellation_token: CancellationToken,
}

impl Devices {
    pub fn new_from_many_devices(devices: Vec<Device>, cancellation_token: CancellationToken) -> Self {
        let devices = Arc::new(RwLock::new(HashMap::<_, _>::from_iter(
            devices
                .into_iter()
                .map(|d| (d.details.identifier.clone(), Arc::new(RwLock::new(d)))),
        )));
        Self {
            devices,
            cancellation_token,
        }
    }

    pub async fn new_from_many_shared_devices(
        devices: Vec<Arc<RwLock<Device>>>,
        cancellation_token: CancellationToken,
    ) -> Self {
        let devices = Arc::new(RwLock::new(HashMap::<_, _>::from_iter(
            devices
                .into_iter()
                .async_map(async |device_lock| {
                    let id = {
                        let device = device_lock.read().await;
                        device.details.identifier.clone()
                    };
                    (id, device_lock)
                })
                .await,
        )));
        Self {
            devices,
            cancellation_token,
        }
    }

    #[cfg(test)]
    pub fn new_from_single_device(device: Device, cancellation_token: CancellationToken) -> Self {
        Self {
            devices: Arc::new(RwLock::new(
                hashmap! {device.details.identifier.clone() => Arc::new(RwLock::new(device))},
            )),
            cancellation_token,
        }
    }

    pub async fn from_device_providers(
        providers: Arc<Vec<Box<dyn DeviceProvider>>>,
        availability_topic: String,
        cancellation_token: CancellationToken,
    ) -> Result<Self> {
        let mut all_devices = HashMap::new();
        for provider in providers.iter() {
            let Devices { mut devices, .. } = provider
                .get_devices(availability_topic.clone(), CancellationToken::default())
                .await?;
            all_devices.extend(Arc::get_mut(&mut devices).unwrap().write().await.drain());
        }
        Ok(Devices {
            devices: Arc::new(RwLock::new(all_devices)),
            cancellation_token,
        })
    }

    pub async fn iter(&self) -> Vec<Arc<RwLock<Device>>> {
        self.devices.read().await.values().cloned().collect()
    }

    pub fn into_iter(self) -> Option<hashbrown::hash_map::IntoValues<String, Arc<RwLock<Device>>>> {
        Arc::<_>::into_inner(self.devices).map(|rwlock| rwlock.into_inner().into_values())
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
        self.iter()
            .await
            .async_map(
                |device_lock| async move { device_lock.read().await.create_discovery_info(discovery_prefix).await },
            )
            .await
            .into_iter()
            .collect::<Result<HashMap<_, _>>>()
    }

    pub async fn discovery_topics(&self, discovery_prefix: &str) -> Vec<String> {
        let mut topics = Vec::new();
        for device_lock in self.iter().await {
            let device = device_lock.read().await;
            topics.push(device.discovery_topic(discovery_prefix));
        }
        topics
    }

    pub async fn command_topics(&self) -> Vec<String> {
        let mut topics = Vec::new();
        for device_lock in self.iter().await {
            let device = device_lock.read().await;
            topics.extend(device.command_topics());
        }
        topics
    }

    pub async fn get_entities_data(&self) -> Result<HashMap<String, String>> {
        let mut entities_data = HashMap::<String, String>::new();
        for device_lock in self.iter().await {
            let device = device_lock.read().await;
            trace!("Getting entities data for device: {}", device.details.name);
            let data = device.get_entities_data().await?;
            trace!("Got entities data for device: {}: {data:?}", device.details.name);
            entities_data.extend(data);
        }
        Ok(entities_data)
    }

    pub async fn handle_command(&self, publish_result: &PublishResult) -> Result<HashMap<String, String>> {
        let mut state_updates = HashMap::new();
        let mut handled = false;
        for device_lock in self.iter().await {
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
    pub async fn len(&self) -> usize {
        self.devices.read().await.len()
    }

    #[allow(dead_code)] // todo: keep? possibly useful
    pub async fn get(&self, identifier: &str) -> Option<Arc<RwLock<Device>>> {
        self.devices.read().await.get(identifier).cloned()
    }

    pub async fn filter(&self, identifiers: HashSet<String>) -> Devices {
        let filtered_devices = self
            .devices
            .read()
            .await
            .iter()
            .filter(|(id, _)| identifiers.contains(*id))
            .map(|(id, device)| (id.clone(), device.clone()))
            .collect::<HashMap<_, _>>();
        Devices {
            devices: Arc::new(RwLock::new(filtered_devices)),
            cancellation_token: self.cancellation_token.clone(),
        }
    }

    pub async fn identifiers(&self) -> HashSet<String> {
        self.devices.read().await.keys().cloned().collect()
    }

    pub async fn add_devices(&self, new_devices: Vec<Device>) -> Result<()> {
        self.cancellation_token.wait_on(self.devices.write()).await?.extend(
            new_devices
                .into_iter()
                .map(|d| (d.details.identifier.clone(), Arc::new(RwLock::new(d)))),
        );
        Ok(())
    }

    pub async fn remove_devices(&self, identifiers: &HashSet<String>) -> Result<Vec<Arc<RwLock<Device>>>> {
        let mut devices = self.cancellation_token.wait_on(self.devices.write()).await?;
        Ok(identifiers
            .iter()
            .filter_map(|identifier| devices.remove(identifier))
            .collect::<Vec<_>>())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::MockHandlesData;
    use crate::devices::test_helpers::*;
    use pretty_assertions::assert_eq;
    use std::future;

    #[tokio::test]
    async fn test_new_from_many_devices() {
        let device1 = make_device_with_identifier("device1");
        let device2 = make_device_with_identifier("device2");
        let devices = Devices::new_from_many_devices(vec![device1, device2], CancellationToken::default());

        assert_eq!(devices.len().await, 2);
        assert!(devices.get("device1").await.is_some());
        assert!(devices.get("device2").await.is_some());
    }

    #[tokio::test]
    async fn test_new_from_many_shared_devices() {
        let device1 = Arc::new(RwLock::new(make_device_with_identifier("device1")));
        let device2 = Arc::new(RwLock::new(make_device_with_identifier("device2")));
        let devices = Devices::new_from_many_shared_devices(vec![device1, device2], CancellationToken::default()).await;

        assert_eq!(devices.len().await, 2);
        assert!(devices.get("device1").await.is_some());
        assert!(devices.get("device2").await.is_some());
    }

    #[tokio::test]
    async fn test_new_from_single_device() {
        let device = make_device_with_identifier("device1");
        let devices = Devices::new_from_single_device(device, CancellationToken::default());

        assert_eq!(devices.len().await, 1);
        assert!(devices.get("device1").await.is_some());
    }

    #[tokio::test]
    async fn test_iter() {
        let device1 = make_device_with_identifier("device1");
        let device2 = make_device_with_identifier("device2");
        let devices = Devices::new_from_many_devices(vec![device1, device2], CancellationToken::default());

        let iterated_devices = devices.iter().await;
        assert_eq!(iterated_devices.len(), 2);
    }

    #[tokio::test]
    async fn test_into_iter() {
        let device1 = make_device_with_identifier("device1");
        let device2 = make_device_with_identifier("device2");
        let devices = Devices::new_from_many_devices(vec![device1, device2], CancellationToken::default());

        let iterated_devices: Vec<_> = devices.into_iter().unwrap().collect();
        assert_eq!(iterated_devices.len(), 2);
    }

    #[tokio::test]
    async fn test_into_vec() {
        let device1 = make_device_with_identifier("device1");
        let device2 = make_device_with_identifier("device2");
        let devices = Devices::new_from_many_devices(vec![device1, device2], CancellationToken::default());

        let vec_devices = devices.into_vec().await.unwrap();
        assert_eq!(vec_devices.len(), 2);
        assert!(vec_devices.iter().any(|d| d.details.identifier == "device1"));
        assert!(vec_devices.iter().any(|d| d.details.identifier == "device2"));
    }

    #[tokio::test]
    async fn test_identifiers() {
        let device1 = make_device_with_identifier("device1");
        let device2 = make_device_with_identifier("device2");
        let devices = Devices::new_from_many_devices(vec![device1, device2], CancellationToken::default());

        let identifiers = devices.identifiers().await;
        assert_eq!(identifiers.len(), 2);
        assert!(identifiers.contains("device1"));
        assert!(identifiers.contains("device2"));
    }

    #[tokio::test]
    async fn test_create_discovery_info() {
        let mut device = make_device_with_identifier("device1");
        device.entities.push(Box::new(make_mock_entity()));
        let devices = Devices::new_from_single_device(device, CancellationToken::default());

        let discovery_info = devices.create_discovery_info("homeassistant").await.unwrap();
        assert_eq!(discovery_info.len(), 1);

        let payload = discovery_info.get("homeassistant/device/device1/config").unwrap();
        let json: serde_json::Value = serde_json::from_str(payload).unwrap();

        assert_eq!(json["device"]["identifiers"][0], "device1");
        assert_eq!(json["components"]["test"], true);
    }

    #[tokio::test]
    async fn test_discovery_topics() {
        let device = make_device_with_identifier("device1");
        let devices = Devices::new_from_single_device(device, CancellationToken::default());

        let topics = devices.discovery_topics("homeassistant").await;
        assert_eq!(topics.len(), 1);
        assert!(topics[0].contains("device1"));
    }

    #[tokio::test]
    async fn test_command_topics() {
        let mut device = make_device_with_identifier("device1");
        device.entities.push(Box::new(make_mock_entity()));
        let devices = Devices::new_from_single_device(device, CancellationToken::default());

        let topics = devices.command_topics().await;
        assert!(!topics.is_empty());
    }

    #[tokio::test]
    async fn test_get_entities_data() {
        let mut device = make_device_with_identifier("device1");
        device.data_handlers.push(Box::new(make_mock_data_handler()));
        let devices = Devices::new_from_single_device(device, CancellationToken::default());

        let data = devices.get_entities_data().await.unwrap();
        assert_eq!(1, data.len());
        assert!(data.contains_key("dev1/test_name/state"));
    }

    #[tokio::test]
    async fn test_handle_command() {
        let mut device = make_device_with_identifier("device1");
        let mut data_handler = MockHandlesData::new();
        data_handler.expect_handle_command().returning(|_, _, _| {
            Box::pin(future::ready(Ok(crate::device_manager::CommandResult {
                handled: true,
                state_update_topics: Some(hashmap! {
                    "state_topic".to_string() => "new_state".to_string()
                }),
            })))
        });
        device.data_handlers.push(Box::new(data_handler));
        let devices = Devices::new_from_single_device(device, CancellationToken::default());

        let result = devices
            .handle_command(&PublishResult {
                topic: "device1/test_name/command".to_string(),
                payload: "payload".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result.get("state_topic").unwrap(), "new_state");
    }

    #[tokio::test]
    async fn test_filter() {
        let device1 = make_device_with_identifier("device1");
        let device2 = make_device_with_identifier("device2");
        let devices = Devices::new_from_many_devices(vec![device1, device2], CancellationToken::default());

        let filtered = devices.filter(HashSet::from(["device1".to_string()])).await;
        assert_eq!(filtered.len().await, 1);
        assert!(filtered.get("device1").await.is_some());
        assert!(filtered.get("device2").await.is_none());
    }

    #[tokio::test]
    async fn test_add_devices() {
        let device1 = make_device_with_identifier("device1");
        let devices = Devices::new_from_single_device(device1, CancellationToken::default());

        let device2 = make_device_with_identifier("device2");
        devices.add_devices(vec![device2]).await.unwrap();

        assert_eq!(devices.len().await, 2);
        assert!(devices.get("device1").await.is_some());
        assert!(devices.get("device2").await.is_some());
    }

    #[tokio::test]
    async fn test_remove_devices() {
        let device1 = make_device_with_identifier("device1");
        let device2 = make_device_with_identifier("device2");
        let devices = Devices::new_from_many_devices(vec![device1, device2], CancellationToken::default());

        let removed = devices
            .remove_devices(&vec!["device1".to_string()].into_iter().collect())
            .await
            .unwrap();

        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].read().await.details.identifier, "device1");
        assert_eq!(devices.len().await, 1);
        assert!(devices.get("device1").await.is_none());
        assert!(devices.get("device2").await.is_some());
    }
}
