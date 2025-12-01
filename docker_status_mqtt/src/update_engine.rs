use async_stream::stream;
use futures::{Stream, StreamExt};
use hashbrown::{HashMap, HashSet};
use std::{fmt::Debug, sync::Arc, time::Duration};

use tokio::{
    sync::{Mutex, RwLock},
    time,
};

use crate::{
    cancellation_token::CancellationToken,
    devices::{Device, DeviceProvider, Devices},
    helpers::AsyncMap,
};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Unknown error")]
    Unknown,
    #[error(transparent)]
    Devices(#[from] crate::devices::Error),
    #[error(transparent)]
    Cancellation(#[from] crate::cancellation_token::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum UpdateEvent {
    Data(HashMap<String, String>),
    DevicesRemoved(Vec<Arc<RwLock<Device>>>),
    DevicesCreated(HashSet<String>),
}

pub struct TimedUpdateEventProvider;
impl TimedUpdateEventProvider {
    pub fn create_entity_update_event_producer(
        devices: &Devices,
        cancellation_token: CancellationToken,
        publish_interval: Duration,
        last_messages: Arc<Mutex<HashMap<String, String>>>,
    ) -> impl Stream<Item = Result<Vec<UpdateEvent>>> {
        stream! {
            let mut interval = time::interval(publish_interval);
            interval.tick().await; // to set the initial instant
            loop {
                trace!(category = "timed_update_event_provider"; "In the loop for publishing sensor data, now will wait for next interval tick...");
                if cancellation_token.wait_on(interval.tick()).await.is_err() {
                    trace!(category = "timed_update_event_provider"; "In the loop for publishing sensor data, cancellation requested, breaking...");
                    break;
                }
                trace!(category = "timed_update_event_provider"; "In the loop for publishing sensor data, interval ticked...");
                info!(category = "timed_update_event_provider"; "Publishing sensor data...");
                match get_events_from_devices(devices, cancellation_token.clone(), last_messages.clone()).await {
                    Ok(events) => {
                        trace!(category = "timed_update_event_provider"; "Got sensor data, sending data...");
                        yield Ok(vec![UpdateEvent::Data(events)]);
                    },
                    Err(err) => {
                        yield Err(err);
                    },
                }
            }
        }
    }

    pub fn create_devices_created_and_removed_event_producer(
        device_providers: Arc<Vec<Box<dyn DeviceProvider>>>,
        devices: &Devices,
        availability_topic: String,
        cancellation_token: CancellationToken,
        publish_interval: Duration,
        last_messages: Arc<Mutex<HashMap<String, String>>>,
    ) -> impl Stream<Item = Result<Vec<UpdateEvent>>> {
        stream! {
            let mut interval = time::interval(publish_interval);
            interval.tick().await; // to set the initial instant
            loop {
                trace!(category = "timed_update_event_provider"; "In the loop for publishing new and removed devices, now will wait for next interval tick...");
                if cancellation_token.wait_on(interval.tick()).await.is_err() {
                    trace!(category = "timed_update_event_provider"; "In the loop for publishing new and removed devices, cancellation requested, breaking...");
                    break;
                }
                trace!(category = "timed_update_event_provider"; "In the loop for publishing new and removed devices, interval ticked...");
                match get_new_and_removed_devices_events_from_device_managers(
                    device_providers.clone(),
                    devices,
                    availability_topic.clone(),
                    cancellation_token.clone(),
                    last_messages.clone(),
                ).await {
                    Ok(events) => {
                        if !events.is_empty() {
                            info!(category = "timed_update_event_provider"; "Publishing new and removed devices...");
                            yield Ok(events);
                        }
                    },
                    err => {
                        error!(category = "timed_update_event_provider"; "Error getting new and removed devices: {:?}", &err);
                        yield err;
                    },
                }
            }
        }
    }
}

async fn get_new_and_removed_devices_events_from_device_managers(
    device_providers: Arc<Vec<Box<dyn DeviceProvider>>>,
    devices: &Devices,
    availability_topic: String,
    cancellation_token: CancellationToken,
    last_messages: Arc<Mutex<HashMap<String, String>>>,
) -> Result<Vec<UpdateEvent>> {
    let (added, removed) = device_providers
        .iter()
        .async_map(async |device_provider| {
            Ok((
                device_provider
                    .add_discovered_devices(devices, availability_topic.clone(), cancellation_token.clone())
                    .await?,
                device_provider
                    .remove_missing_devices(devices, cancellation_token.clone())
                    .await?,
            ))
        })
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .fold(
            (HashSet::new(), Vec::new()),
            |(mut added, mut removed), (mut to_add, to_remove)| {
                added.extend(to_add.drain());
                removed.extend(to_remove);
                (added, removed)
            },
        );

    let removed_topic_prefixes = removed
        .iter()
        .async_map(async |device| device.read().await)
        .await
        .iter()
        .flat_map(|removed_device| {
            removed_device
                .entities
                .iter()
                .map(|entity| entity.details().get_topic_path(None))
        })
        .collect::<HashSet<_>>();
    last_messages.lock().await.retain(|key, _| {
        removed_topic_prefixes
            .iter()
            .all(|topic_prefix| !key.starts_with(topic_prefix))
    });

    let mut events = vec![];
    if !added.is_empty() {
        events.push(UpdateEvent::DevicesCreated(added));
    }
    if !removed.is_empty() {
        events.push(UpdateEvent::DevicesRemoved(removed));
    }
    Ok(events)
}

async fn get_events_from_devices(
    devices: &Devices,
    cancellation_token: CancellationToken,
    last_messages: Arc<Mutex<HashMap<String, String>>>,
) -> Result<HashMap<String, String>> {
    Ok(devices.devices_vec().await.into_iter().async_map(async |device_arc| {
            info!(category = "timed_update_event_provider"; "Publishing sensor data...");
            let device = cancellation_token.wait_on(device_arc.read()).await?;
            trace!(category = "timed_update_event_provider"; "Getting entities data for device: {}", device.details.name);
            let data = device.get_entities_data().await?;
            trace!(category = "timed_update_event_provider"; "Got entities data for device: {}: {data:?}", device.details.name);
            Ok(data)
        })
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .async_map(async |data| // filter later so that previous errors are not added to last_messages
                filter_data(data, last_messages.clone(), cancellation_token.clone()).await
            )
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .reduce(|mut events_acc, events| {
                events_acc.extend(events);
                events_acc
            })
            .unwrap_or_default())
}

async fn filter_data(
    data: HashMap<String, String>,
    last_messages_mutex: Arc<Mutex<HashMap<String, String>>>,
    cancellation_token: CancellationToken,
) -> Result<HashMap<String, String>> {
    let mut entities_data = HashMap::<String, String>::new();
    for entity_data in data {
        let mut last_messages = cancellation_token.wait_on(last_messages_mutex.lock()).await?;
        if let Some(last_payload) = last_messages.get_mut(&entity_data.0) {
            if *last_payload == entity_data.1 {
                trace!(
                    category = "timed_update_event_provider";
                    "Entity data for topic {} payload is identical, skipping publish",
                    entity_data.0
                );
            } else {
                trace!(
                    category = "timed_update_event_provider";
                    "Entity data for topic {} payload has changed, will publish",
                    entity_data.0
                );
                entities_data.insert(entity_data.0.clone(), entity_data.1.clone());
                *last_payload = entity_data.1;
            }
        } else {
            trace!(
                category = "timed_update_event_provider";
                "Entity data for topic {} has changed or is new, will publish",
                entity_data.0
            );
            entities_data.insert(entity_data.0.clone(), entity_data.1.clone());
            last_messages.insert(entity_data.0, entity_data.1);
        }
    }
    Ok(entities_data)
}

pub async fn create_event_producers(
    devices: &Devices,
    device_providers: Arc<Vec<Box<dyn DeviceProvider>>>,
    availability_topic: String,
    cancellation_token: CancellationToken,
    publish_interval: Duration,
) -> impl Stream<Item = Result<Vec<UpdateEvent>>> {
    let last_messages = Arc::new(Mutex::new(HashMap::<String, String>::new()));
    futures::stream::select_all([
        TimedUpdateEventProvider::create_devices_created_and_removed_event_producer(
            device_providers.clone(),
            devices,
            availability_topic.clone(),
            cancellation_token.clone(),
            publish_interval,
            last_messages.clone(),
        )
        .boxed(),
        TimedUpdateEventProvider::create_entity_update_event_producer(
            devices,
            cancellation_token,
            publish_interval,
            last_messages,
        )
        .boxed(),
    ])
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use serial_test::serial;
    use std::{future, pin::pin, sync::Arc};
    use tokio::sync::Mutex;

    use super::*;
    use crate::devices::test_helpers::*;
    use pretty_assertions::assert_eq;

    impl PartialEq for UpdateEvent {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (UpdateEvent::Data(a), UpdateEvent::Data(b)) => a == b,
                (UpdateEvent::DevicesRemoved(a), UpdateEvent::DevicesRemoved(b)) => {
                    let a_ids: HashSet<String> = a
                        .iter()
                        .map(|device_lock| {
                            let device = futures::executor::block_on(device_lock.read());
                            device.details.identifier.clone()
                        })
                        .collect();
                    let b_ids: HashSet<String> = b
                        .iter()
                        .map(|device_lock| {
                            let device = futures::executor::block_on(device_lock.read());
                            device.details.identifier.clone()
                        })
                        .collect();
                    a_ids == b_ids
                }
                (UpdateEvent::DevicesCreated(a), UpdateEvent::DevicesCreated(b)) => a == b,
                _ => false,
            }
        }
    }

    #[tokio::test]
    #[serial]
    async fn test_publish_sensor_data_for_all_devices_skips_unchanged_data() {
        let mut device = make_empty_device();
        device.data_handlers.push(Box::new(make_mock_data_handler()));
        let devices = Devices::new_from_single_device(device);
        let event_producer = TimedUpdateEventProvider::create_entity_update_event_producer(
            &devices,
            CancellationToken::default(),
            Duration::from_millis(1),
            Arc::new(Mutex::new(HashMap::<String, String>::new())),
        );
        let mut event_producer = pin!(event_producer);
        let first_event = event_producer.next().await.unwrap().unwrap(); // First call to populate last_messages
        assert_eq!(
            vec![UpdateEvent::Data(hashmap! {
                "dev1/test_name/state".to_string() => "test_state".to_string()
            })],
            first_event
        );
        let second_event = event_producer.next().await.unwrap().unwrap(); // Second call should skip unchanged data
        assert_eq!(vec![UpdateEvent::Data(hashmap! {})], second_event);
    }

    #[tokio::test]
    async fn test_create_devices_created_and_removed_event_producer_adds_devices() {
        let mut mock_provider = crate::devices::MockDeviceProvider::new();
        mock_provider
            .expect_add_discovered_devices()
            .returning(|_, _, _| Box::pin(future::ready(Ok(["new_device".to_string()].into_iter().collect()))));
        mock_provider
            .expect_remove_missing_devices()
            .returning(|_, _| Box::pin(future::ready(Ok(vec![]))));

        let devices = make_empty_devices();
        let event_producer = TimedUpdateEventProvider::create_devices_created_and_removed_event_producer(
            Arc::new(vec![Box::new(mock_provider)]),
            &devices,
            "availability_topic".to_string(),
            CancellationToken::default(),
            Duration::from_millis(1),
            Arc::new(Mutex::new(HashMap::new())),
        );

        let mut event_producer = pin!(event_producer);
        let event = event_producer.next().await.unwrap().unwrap();
        assert_eq!(
            event,
            vec![UpdateEvent::DevicesCreated(
                ["new_device".to_string()].into_iter().collect()
            )]
        );
    }

    #[tokio::test]
    async fn test_create_devices_created_and_removed_event_producer_removes_devices() {
        let mut mock_provider = crate::devices::MockDeviceProvider::new();
        mock_provider
            .expect_add_discovered_devices()
            .returning(|_, _, _| Box::pin(future::ready(Ok(HashSet::new()))));
        mock_provider.expect_remove_missing_devices().returning(|_, _| {
            let device = make_device_with_identifier("removed_device");
            Box::pin(future::ready(Ok(vec![Arc::new(RwLock::new(device))])))
        });

        let devices = make_empty_devices();
        let event_producer = TimedUpdateEventProvider::create_devices_created_and_removed_event_producer(
            Arc::new(vec![Box::new(mock_provider)]),
            &devices,
            "availability_topic".to_string(),
            CancellationToken::default(),
            Duration::from_millis(1),
            Arc::new(Mutex::new(HashMap::new())),
        );

        let mut event_producer = pin!(event_producer);
        let event = event_producer.next().await.unwrap().unwrap();

        assert_eq!(event.len(), 1);
        match &event[0] {
            UpdateEvent::DevicesRemoved(removed) => {
                assert_eq!(removed.len(), 1);
                let device = removed[0].read().await;
                assert_eq!(device.details.identifier, "removed_device");
            }
            _ => panic!("Expected DevicesRemoved event"),
        }
    }

    #[tokio::test]
    async fn test_create_devices_created_and_removed_event_producer_handles_errors() {
        let mut mock_provider = crate::devices::MockDeviceProvider::new();
        mock_provider
            .expect_add_discovered_devices()
            .returning(|_, _, _| Box::pin(future::ready(Err(crate::devices::Error::Unknown))));
        mock_provider
            .expect_remove_missing_devices()
            .returning(|_, _| Box::pin(future::ready(Ok(vec![]))));

        let devices = make_empty_devices();
        let event_producer = TimedUpdateEventProvider::create_devices_created_and_removed_event_producer(
            Arc::new(vec![Box::new(mock_provider)]),
            &devices,
            "availability_topic".to_string(),
            CancellationToken::default(),
            Duration::from_millis(1),
            Arc::new(Mutex::new(HashMap::new())),
        );

        let mut event_producer = pin!(event_producer);
        let result = event_producer.next().await.unwrap();
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn test_create_devices_created_and_removed_event_producer_purges_last_messages() {
        let devices = make_empty_devices();
        let device = devices.devices_vec().await.into_iter().next().unwrap();
        let entity = make_mock_entity_with_device_id_and_name("removed_device", "Test Device");
        {
            device.write().await.entities.push(Box::new(entity));
        }
        let mut mock_provider = crate::devices::MockDeviceProvider::new();
        mock_provider
            .expect_add_discovered_devices()
            .returning(|_, _, _| Box::pin(future::ready(Ok(HashSet::new()))));
        mock_provider
            .expect_remove_missing_devices()
            .returning(move |_, _| Box::pin(future::ready(Ok(vec![device.clone()]))));

        let last_messages = Arc::new(Mutex::new(hashmap! {
            "removed_device/test_device/state".to_string() => "some_value".to_string(),
            "other_device/test_device/state".to_string() => "other_value".to_string()
        }));

        let event_producer = TimedUpdateEventProvider::create_devices_created_and_removed_event_producer(
            Arc::new(vec![Box::new(mock_provider)]),
            &devices,
            "availability_topic".to_string(),
            CancellationToken::default(),
            Duration::from_millis(1),
            last_messages.clone(),
        );

        let mut event_producer = pin!(event_producer);
        let _ = event_producer.next().await.unwrap().unwrap();

        let messages = last_messages.lock().await;
        assert!(!messages.contains_key("removed_device/test_device/state"));
        assert!(messages.contains_key("other_device/test_device/state"));
    }
}
