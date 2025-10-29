use hashbrown::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tokio::time;

use crate::{
    cancellation_token::CancellationToken,
    devices::{Device, Devices},
};

#[allow(dead_code)]
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Unknown error")]
    Unknown,
    #[error(transparent)]
    Devices(#[from] crate::devices::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

pub struct UpdateEngine<F, Fut, TEvent>
where
    F: FnMut(&CancellationToken) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Option<Vec<TEvent>>>> + Send,
{
    #[allow(dead_code)]
    cancellation_token: CancellationToken,
    events: F,
}

impl<F, Fut, TEvent> Debug for UpdateEngine<F, Fut, TEvent>
where
    F: FnMut(&CancellationToken) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Option<Vec<TEvent>>>> + Send,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.cancellation_token, f)
    }
}

#[allow(dead_code)]
impl<F, Fut, TEvent> UpdateEngine<F, Fut, TEvent>
where
    F: FnMut(&CancellationToken) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Option<Vec<TEvent>>>> + Send,
{
    pub fn new(cancellation_token: CancellationToken, events: F) -> Self {
        Self {
            cancellation_token,
            events,
        }
    }

    pub async fn next_event(&mut self) -> Result<Option<Vec<TEvent>>> {
        (self.events)(&self.cancellation_token).await
    }
}

#[async_trait]
pub trait ProducesEvents<TEvent>: std::fmt::Debug + Send + Sync {
    async fn next_event(&mut self) -> Result<Option<Vec<TEvent>>>;
}

#[async_trait]
impl<F, Fut, TEvent> ProducesEvents<TEvent> for UpdateEngine<F, Fut, TEvent>
where
    F: FnMut(&CancellationToken) -> Fut + Send + Sync,
    Fut: Future<Output = Result<Option<Vec<TEvent>>>> + Send,
{
    async fn next_event(&mut self) -> Result<Option<Vec<TEvent>>> {
        self.next_event().await
    }
}

#[allow(dead_code)] // todo: use DeviceUpdated variant in docker provider
#[derive(Debug, PartialEq, Clone)]
pub enum UpdateEvent {
    Data(HashMap<String, String>),
    DeviceUpdated(HashMap<String, String>),
}

pub struct TimedUpdateEventProvider;
impl TimedUpdateEventProvider {
    pub fn create_event_producer(
        device_arc: Arc<RwLock<Device>>,
        cancellation_token: CancellationToken,
        publish_interval: Duration,
    ) -> Box<dyn ProducesEvents<UpdateEvent>> {
        let last_messages_seed = Arc::new(Mutex::new(HashMap::<String, String>::new()));
        let engine = UpdateEngine::new(cancellation_token, move |cancellation_token| {
            let cancellation_token_clone = cancellation_token.clone();
            let last_messages_mutex = last_messages_seed.clone();
            let device_rwlock = device_arc.clone();
            async move {
                trace!(category = "timed_update_event_provider"; "In the main loop for publishing sensor data, now will wait for next interval tick...");
                let mut interval = time::interval(publish_interval);
                interval.tick().await; // to set the initial instant
                if cancellation_token_clone.wait_on(interval.tick()).await.is_err() {
                    debug!(category = "timed_update_event_provider"; "Cancellation token triggered, stopping periodic sensor data publishing task");
                    return Ok(None);
                }
                trace!(category = "timed_update_event_provider"; "Interval ticked...");
                info!(category = "timed_update_event_provider"; "Publishing sensor data...");
                let mut entities_data = HashMap::<String, String>::new();
                let data = {
                    let device = device_rwlock.read().await;
                    trace!(category = "timed_update_event_provider"; "Getting entities data for device: {}", device.details.name);
                    let result_data = device.get_entities_data().await;
                    if let Err(error) = result_data {
                        error!("Error getting entities data for device {}", device.details.name);
                        return Err(crate::update_engine::Error::Devices(error));
                    }
                    let data = result_data.unwrap();
                    trace!(category = "timed_update_event_provider"; "Got entities data for device: {}: {data:?}", device.details.name);
                    data
                };
                for entity_data in data.into_iter() {
                    let mut last_messages = last_messages_mutex.lock().await;
                    match last_messages.get_mut(&entity_data.0) {
                        Some(last_payload) => {
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
                        }
                        _ => {
                            trace!(
                                category = "timed_update_event_provider";
                                "Entity data for topic {} has changed or is new, will publish",
                                entity_data.0
                            );
                            entities_data.insert(entity_data.0.clone(), entity_data.1.clone());
                            last_messages.insert(entity_data.0, entity_data.1);
                        }
                    }
                }
                Ok(Some(vec![UpdateEvent::Data(entities_data)]))
            }
        });
        Box::new(engine)
    }

    pub fn create_event_producers(
        devices: Devices,
        cancellation_token: CancellationToken,
        publish_interval: Duration,
    ) -> Vec<Box<dyn ProducesEvents<UpdateEvent> + 'static>> {
        let mut event_producers: Vec<Box<dyn ProducesEvents<UpdateEvent>>> = vec![];
        for device in devices.iter() {
            event_producers.push(TimedUpdateEventProvider::create_event_producer(
                device.clone(),
                cancellation_token.clone(),
                publish_interval,
            ));
        }
        event_producers
    }
}

#[cfg(test)]
mod tests {
    use serial_test::serial;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    use super::*;
    use crate::{cancellation_token::CancellationTokenSource, devices::test_module::test_helpers::*};
    use pretty_assertions::assert_eq;

    #[tokio::test]
    async fn single_event() {
        let mut engine = UpdateEngine::new(CancellationToken::default(), |_| async {
            Ok(Some(vec![UpdateEvent::Data(
                hashmap! {"x".to_string() => "y".to_string()},
            )]))
        });

        let events = engine.next_event().await.unwrap();
        assert_eq!(
            events,
            Some(vec![UpdateEvent::Data(hashmap! {"x".to_string() => "y".to_string()})])
        );
    }

    #[tokio::test]
    async fn no_event() {
        let mut engine = UpdateEngine::<_, _, UpdateEvent>::new(CancellationToken::default(), |_| async { Ok(None) });
        let first_events = engine.next_event().await.unwrap();
        assert_eq!(first_events, None);
        let second_events = engine.next_event().await.unwrap();
        assert_eq!(second_events, None);
    }

    struct SomeState {
        call_count: usize,
        current_char: char,
        max_calls: u8,
    }

    impl SomeState {
        fn new(max_calls: u8) -> Self {
            Self {
                call_count: 0,
                current_char: 'a',
                max_calls,
            }
        }

        async fn get_events(&mut self, cancellation_token: CancellationToken) -> Option<Vec<UpdateEvent>> {
            tokio::task::yield_now().await; // simulating some async work and context switch
            if self.call_count >= self.max_calls as usize || cancellation_token.is_cancelled().await {
                return None;
            }
            self.call_count += 1;
            let first_char = self.current_char;
            let second_char = ((first_char as u8) + 1) as char;
            self.current_char = ((second_char as u8) + 1) as char;
            Some(vec![UpdateEvent::Data(hashmap! {
                first_char.to_string() => second_char.to_string()
            })])
        }
    }

    #[tokio::test]
    async fn one_event_two_polls() {
        let some_state = Arc::new(Mutex::new(SomeState::new(1)));
        let mut engine = UpdateEngine::new(CancellationToken::default(), move |cancellation_token| {
            let state = some_state.clone();
            let cancellation_token_clone = cancellation_token.clone();
            async move { Ok(state.lock().await.get_events(cancellation_token_clone).await) }
        });
        let first_events = engine.next_event().await.unwrap();
        assert_eq!(
            first_events,
            Some(vec![UpdateEvent::Data(hashmap! {"a".to_string() => "b".to_string()})])
        );
        let second_events = engine.next_event().await.unwrap();
        assert_eq!(second_events, None);
    }

    #[tokio::test]
    async fn two_events_three_polls() {
        let some_state = Arc::new(Mutex::new(SomeState::new(2)));
        let mut engine = UpdateEngine::new(CancellationToken::default(), move |cancellation_token| {
            let state = some_state.clone();
            let cancellation_token_clone = cancellation_token.clone();
            async move { Ok(state.lock().await.get_events(cancellation_token_clone).await) }
        });
        let first_events = engine.next_event().await.unwrap();
        assert_eq!(
            first_events,
            Some(vec![UpdateEvent::Data(hashmap! {"a".to_string() => "b".to_string()})])
        );
        let second_events = engine.next_event().await.unwrap();
        assert_eq!(
            second_events,
            Some(vec![UpdateEvent::Data(hashmap! {"c".to_string() => "d".to_string()})])
        );
        let third_events = engine.next_event().await.unwrap();
        assert_eq!(third_events, None);
    }

    #[tokio::test]
    async fn two_events_cancelled() {
        let mut cancellation_token_source = CancellationTokenSource::new();
        let some_state = Arc::new(Mutex::new(SomeState::new(2)));
        let mut engine = UpdateEngine::new(
            cancellation_token_source.create_token().await,
            move |cancellation_token| {
                let state = some_state.clone();
                let cancellation_token_clone = cancellation_token.clone();
                async move { Ok(state.lock().await.get_events(cancellation_token_clone).await) }
            },
        );
        let first_events = engine.next_event().await.unwrap();
        assert_eq!(
            first_events,
            Some(vec![UpdateEvent::Data(hashmap! {"a".to_string() => "b".to_string()})])
        );
        cancellation_token_source.cancel().await;
        let second_events = engine.next_event().await.unwrap();
        assert_eq!(second_events, None);
    }

    #[tokio::test]
    #[serial]
    async fn test_publish_sensor_data_for_all_devices_skips_unchanged_data() {
        let mut device = make_empty_device();
        device.entities.push(Box::new(get_mock_entity()));
        let mut event_producer = TimedUpdateEventProvider::create_event_producer(
            Arc::new(RwLock::new(device)),
            CancellationToken::default(),
            Duration::from_millis(1),
        );
        let first_event = event_producer.next_event().await.unwrap().unwrap(); // First call to populate last_messages
        assert_eq!(
            vec![UpdateEvent::Data(hashmap! {
                "dev1/test_name/state".to_string() => "test_state".to_string()
            })],
            first_event
        );
        let second_event = event_producer.next_event().await.unwrap(); // Second call should skip unchanged data
        assert_eq!(Some(vec![UpdateEvent::Data(hashmap! {})]), second_event);
    }
}
