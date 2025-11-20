use chrono::Utc;
use futures::StreamExt;
use hashbrown::HashMap;
use rumqttc::{
    Transport,
    v5::{
        ClientError,
        ConnectionError::{self, *},
        Event, MqttOptions,
        mqttbytes::{
            QoS,
            v5::{LastWill, Packet},
        },
    },
};
use rustls_platform_verifier::ConfigVerifierExt;
use std::{
    sync::{Arc, atomic},
    time::Duration,
};
use tokio::{
    sync::watch::{Receiver, Sender},
    task::JoinHandle,
    time::{self, timeout},
};
use tokio_rustls::rustls::ClientConfig;

use crate::{
    cancellation_token::{CancellationToken, CancellationTokenSource},
    devices::{DeviceProvider, Devices},
    helpers::{AsyncMap, pretty_format},
    update_engine::{UpdateEvent, create_event_producers},
};

const DURATION_KEEPALIVE: Duration = Duration::from_secs(45);
const DURATION_DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const DURATION_AVAILABILITY_AFTER_DISCOVERY: Duration = Duration::from_secs(5);
const DURATION_UNTIL_SHUTDOWN: Duration = Duration::from_secs(5);
const DURATION_ZERO: Duration = Duration::from_secs(0);
const DURATION_MAX: Duration = Duration::from_secs(u64::MAX);
const DURATION_QUICK_CYCLE: Duration = Duration::from_millis(100);
const DISCOVERY_PREFIX: &str = "homeassistant";

#[cfg(not(test))]
pub mod mqtt_client {
    pub use rumqttc::v5::AsyncClient;
    pub use rumqttc::v5::EventLoop;
}

#[cfg_attr(test, mockall_double::double)]
use mqtt_client::{AsyncClient, EventLoop};

pub type Result<T> = std::result::Result<T, Error>;

struct EventHandled {
    should_wait: bool,
    should_break: bool,
}

#[derive(Clone, Debug)]
pub enum ChannelMessages {
    Stopping,
    Connected(bool),
    Message(PublishResult),
}

#[derive(Clone)]
pub struct DeviceManager {
    client: AsyncClient,
    discovery_prefix: String,
    connected: Arc<atomic::AtomicU8>,
    tx: Sender<ChannelMessages>,
    default_timeout: Duration,
    publish_interval: Duration,
    availability_topic: String,
    availability_after_discovery: Duration,
    cancellation_token: CancellationToken,
}

impl DeviceManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        broker_host: String,
        broker_port: u16,
        device_manager_id: String,
        username: String,
        password: String,
        disable_tls: bool,
        publish_interval: Duration,
        cancellation_token: CancellationToken,
    ) -> Result<(Self, EventLoop, Receiver<ChannelMessages>)> {
        info!(
            "Connecting to MQTT broker at {broker_host}:{broker_port} with device manager id: {device_manager_id} and user: {username}"
        );
        let availability_topic = format!("{device_manager_id}/availability");
        let mut mqtt_options = MqttOptions::new(device_manager_id, broker_host, broker_port);
        mqtt_options
            .set_last_will(LastWill {
                topic: availability_topic.clone().into(),
                message: "offline".into(),
                qos: QoS::AtLeastOnce,
                retain: false,
                properties: None,
            })
            .set_keep_alive(DURATION_KEEPALIVE)
            .set_clean_start(false)
            .set_credentials(username, password);
        if disable_tls {
            warn!("WARNING: TLS is disabled, connection will be unencrypted!");
        } else {
            mqtt_options.set_transport(Transport::tls_with_config(
                ClientConfig::with_platform_verifier()?.into(),
            ));
        }
        let (client, eventloop) = AsyncClient::new(mqtt_options, 10);
        let (tx, rx) = tokio::sync::watch::channel(ChannelMessages::Connected(false));
        Ok((
            Self {
                client,
                discovery_prefix: DISCOVERY_PREFIX.to_string(),
                connected: Arc::new(atomic::AtomicU8::new(0)), // None
                tx,
                default_timeout: DURATION_DEFAULT_TIMEOUT,
                publish_interval,
                availability_topic,
                availability_after_discovery: DURATION_AVAILABILITY_AFTER_DISCOVERY,
                cancellation_token,
            },
            eventloop,
            rx,
        ))
    }

    fn is_connected(&self) -> Option<bool> {
        match self.connected.load(atomic::Ordering::SeqCst) {
            0 => None,
            1 => Some(false),
            _ => Some(true),
        }
    }

    fn set_connected(&self, is_connected: bool) {
        let int_value = if is_connected { 2 } else { 1 };
        self.connected.store(int_value, atomic::Ordering::SeqCst);
    }

    pub fn availability_topic(&self) -> String {
        self.availability_topic.clone()
    }

    async fn publish_entities_discovery(&self, devices: Devices) -> Result<()> {
        let discovery_info = devices.create_discovery_info(&self.discovery_prefix).await?;
        for (discovery_topic, discovery_json) in discovery_info.into_iter() {
            trace!(
                "Publishing discovery, topic: {}, payload: {}",
                &discovery_topic, &discovery_json,
            );
            self.publish_to_client(discovery_topic.clone(), discovery_json).await?;
            debug!("Published device discoveries for topic: {}", discovery_topic);
        }
        Ok(())
    }

    async fn subscribe_to_commands(&self, devices: Devices) -> Result<()> {
        debug!("Subscribing to Home Assistant status topic...");
        self.subscribe_to_client(format!("{}/status", &self.discovery_prefix))
            .await?;
        trace!("Subscribed to Home Assistant status topic");
        debug!("Subscribing to command topics...");
        for command_topic in devices.command_topics().await.iter() {
            self.subscribe_to_client(command_topic.to_owned()).await?;
            trace!("Subscribed to command topic {command_topic}");
        }
        trace!("Subscribed to command topics");
        Ok(())
    }

    async fn publish_removed_entities_discovery(&self, devices: Devices) -> Result<()> {
        trace!("Removing devices and entities from Home Assistant...");
        for discovery_topic in devices.discovery_topics(&self.discovery_prefix).await.into_iter() {
            trace!("Removing device for topic: {}", &discovery_topic);
            self.publish_to_client(discovery_topic, "".to_string()).await?;
        }
        info!("Removed devices and entities from Home Assistant");
        Ok(())
    }

    async fn publish_sensor_data_for_all_devices(&self, devices: Devices) -> Result<()> {
        trace!("Publishing sensor data to Home Assistant...");
        if self.is_connected() != Some(true) {
            trace!("Not connected to MQTT broker, skipping sensor data publishing");
            return Ok(());
        }
        debug!("Publishing sensor data...");
        let entities_data = devices.get_entities_data().await?;
        self.publish_sensor_data(entities_data).await?;
        trace!("Published all sensor data to Home Assistant");
        Ok(())
    }

    async fn publish_sensor_data(&self, entities_data: HashMap<String, String>) -> Result<()> {
        for entity_data in entities_data {
            if log_enabled!(log::Level::Trace) {
                trace!(
                    "Publishing sensor data to topic: {}, payload: {}",
                    entity_data.0, entity_data.1
                );
            } else if log_enabled!(log::Level::Info) {
                info!("Publishing sensor data to topic: {}", entity_data.0);
            }
            self.publish_to_client(entity_data.0, entity_data.1).await?;
            trace!("Published sensor data to topic");
        }
        Ok(())
    }

    async fn deal_with_command(&self, publish_result: PublishResult, devices: Devices) -> Result<()> {
        trace!(
            "Dealing with command on topic: {}, payload: {}",
            publish_result.topic, publish_result.payload
        );
        if publish_result.topic == format!("{}/status", self.discovery_prefix) {
            if publish_result.payload == "online" {
                info!("Broker is online, initializing in 5 seconds...");
                let other_mqtt_device = self.clone();
                tokio::spawn(async move {
                    time::sleep(other_mqtt_device.default_timeout).await;
                    if let Err(err) = other_mqtt_device.initialize(devices).await {
                        error!("Error initializing after broker came online: {err}");
                    }
                });
            } else {
                info!("Broker is going offline...");
            }
            return Ok(());
        }
        let state_updates = devices.handle_command(&publish_result).await?;
        if state_updates.is_empty() {
            trace!(
                "No state updated for topic: {} and payload:\n{}",
                publish_result.topic, publish_result.payload
            );
            return Ok(());
        }
        match self.publish_sensor_data(state_updates).await {
            Ok(_) => trace!("Sensor data published after command handling"),
            Err(e) => {
                error!(category = "deal_with_command"; "Error publishing sensor data after command handling: {e}")
            }
        }
        trace!("Completed dealing with command");
        Ok(())
    }

    async fn disconnect(&self) -> Result<()> {
        trace!("Disconnecting from MQTT broker if connected...");
        if self.is_connected() == Some(true) {
            info!("Disconnecting from MQTT broker...");
            self.client.try_disconnect()?;
        } else {
            trace!("Not connected to MQTT broker, skipping disconnect");
        }
        Ok(())
    }

    async fn make_available(&self) -> Result<()> {
        self.set_availability(true).await?;
        Ok(())
    }

    fn make_unavailable(&self) -> impl Future<Output = Result<()>> {
        self.set_availability(false)
    }

    async fn set_availability(&self, available: bool) -> Result<()> {
        let availability_text = if available { "online" } else { "offline" }.to_string();
        trace!(
            "Publishing availability in topic {} as {availability_text} for all devices...",
            self.availability_topic
        );
        if self.is_connected() == Some(true) {
            info!("Publishing availability as {availability_text}...");
            self.publish_to_client(self.availability_topic.clone(), availability_text)
                .await?;
            trace!("Published availability to topic");
        } else {
            trace!("Not connected to MQTT broker, will not set availability");
            return Ok(());
        }
        trace!("Published availability for all devices");
        Ok(())
    }

    async fn got_disconnected(
        &self,
        message: &str,
        event: &str,
    ) -> std::result::Result<(), tokio::sync::watch::error::SendError<ChannelMessages>> {
        let is_connected = { self.is_connected() };
        match is_connected {
            Some(true) => {
                error!("Disconnected from MQTT broker after event {event}: Message: {message}");
                self.set_connected(false);
                self.tx.send(ChannelMessages::Connected(false))?;
                debug!("Sent disconnected message to channel after event {event}");
            }
            Some(false) => trace!("Not sending disconnected message to channel because was already disconnected"),
            None => {
                error!("Initial connection failed with event: {event}");
                self.set_connected(false);
            }
        }
        Ok(())
    }

    async fn deal_with_event(
        &mut self,
        event_result: std::result::Result<Event, ConnectionError>,
        should_stop: bool,
    ) -> std::result::Result<EventHandled, EventHandlingError> {
        trace!("Dealing with event: {event_result:?}, should_stop: {should_stop}");
        let mut should_wait = false;
        let mut should_break = false;
        match event_result {
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                trace!("Received publish packet: {publish:?}");
                let publish_result = PublishResult {
                    topic: String::from_utf8_lossy(publish.topic.as_ref()).to_string(),
                    payload: String::from_utf8_lossy(&publish.payload).to_string(),
                };
                self.tx.send(ChannelMessages::Message(publish_result))?;
            }
            Ok(Event::Incoming(Packet::ConnAck(conn_ack))) => {
                if conn_ack.session_present {
                    debug!("Reconnected to MQTT broker with existing session");
                } else {
                    debug!("Connected to MQTT broker with new session");
                }
                self.set_connected(true);
                self.tx.send(ChannelMessages::Connected(true))?;
            }
            Ok(Event::Incoming(Packet::Disconnect(_))) => {
                trace!("Received disconnect packet from broker, will wait...");
                self.got_disconnected("Unexpected disconnection, will retry...", "disconnect")
                    .await?;
                should_wait = true;
            }
            Ok(Event::Outgoing(o)) => {
                trace!("Outgoing event: {o:?}");
            }
            Ok(event) => {
                trace!("Event loop received other event: {event:?}");
            }
            Err(e @ ConnectionRefused(_)) => {
                error!("Connection refused: {e}");
                return Err(EventHandlingError::Connection(format!("Connection refused: {e}")));
            }
            Err(Io(x)) => {
                trace!("Received I/O error: {x}, will wait...");
                self.got_disconnected(&format!("MQTT I/O connection error ({x}), will retry..."), "I/O Error")
                    .await?;
                should_wait = true;
            }
            Err(Tls(e)) => {
                error!("MQTT TLS error ({e})");
                return Err(EventHandlingError::Connection("TLS Error".to_string()));
            }
            Err(MqttState(error)) => {
                if should_stop {
                    debug!("Got MQTT state error after stop request ({error}), exiting loop...");
                    should_break = true;
                } else {
                    trace!("Received MQTT state error ({error}), will wait...");
                    self.got_disconnected("MQTT state error, retrying in 5 seconds...", "I/O Error")
                        .await?;
                    should_wait = true;
                }
            }
            Err(e) => {
                trace!("Received other MQTT connection error: {e}, will wait...");
                self.got_disconnected(
                    &format!("MQTT connection error: {e}, retrying in 5 seconds..."),
                    "I/O Error",
                )
                .await?;
                should_wait = true;
            }
        };
        Ok(EventHandled {
            should_wait,
            should_break,
        })
    }

    /// This will kick off the connection with the broker, publish discovery messages and subscribe to command topics.
    /// After the connection is established, the event loop should be started to handle incoming messages and events.
    /// Subscriptions can be initialized together with the discovery boostraping messages because they do not need to
    /// wait for the connection to be established.
    pub async fn initialize(&self, devices: Devices) -> Result<()> {
        debug!("Initializing DeviceManager...");
        trace!("Publishing entities discovery on initialization...");
        self.publish_entities_discovery(devices.clone())
            .await
            .unwrap_or_else(|e| error!("Error publishing entities discovery: {e}"));
        trace!("Subscribing to command topics on initialization...");
        self.subscribe_to_commands(devices.clone()).await?;
        trace!("Publishing sensor data for all devices on initialization...");
        self.publish_sensor_data_for_all_devices(devices.clone()).await?;
        let other_mqtt_device = self.clone();
        tokio::spawn(async move {
            trace!("Waiting before making available on initialization after discovery...");
            match other_mqtt_device
                .cancellation_token
                .wait_on(time::sleep(other_mqtt_device.availability_after_discovery))
                .await
            {
                Ok(_) => {
                    // this wait is necessary because if it is the first time the device is connected,
                    // registration will take a few seconds and if we make it available before that,
                    // Home Assistant will not register it and the device and entities will not become
                    // available.
                    trace!("Making devices available on initialization after discovery...");
                    if other_mqtt_device.make_available().await.is_err() {
                        error!("Error making devices available on initialization after discovery.");
                    }
                }
                Err(_) => {
                    info!("Received Ctrl+C, not making available after discovery.");
                }
            };
        });
        trace!("Initialization done...");
        Ok(())
    }

    /// This runs synchronously the event loop, handling incoming messages and events,
    /// on the main thread.
    pub async fn deal_with_event_loop(&mut self, mut eventloop: EventLoop) -> Result<()> {
        let mut should_stop = false;
        let mut stop_at = None;
        let mut should_wait = false;
        loop {
            if should_wait {
                trace!(category = "[event_loop]"; "Sleeping in the beginning of event loop...");
                should_wait = false;

                if self
                    .cancellation_token
                    .wait_on(time::sleep(self.publish_interval))
                    .await
                    .is_err()
                {
                    info!(category = "[event_loop]"; "Received Ctrl+C, shutting down...");
                    should_stop = true;
                    stop_at = Some(Utc::now() + DURATION_UNTIL_SHUTDOWN);
                    if self.is_connected() == Some(true) {
                        trace!(category = "[event_loop]"; "Was sleeping on the event loop and connected, will make unavailable and disconnect after stop request...");
                        self.tx.send(ChannelMessages::Stopping)?;
                    } else {
                        trace!(category = "[event_loop]"; "Was sleeping on the event loop and not connected, no need to wait for event loop and will skip make unavailable and disconnecting...");
                        break;
                    }
                }
            }
            let wait_duration = if should_stop {
                if self.is_connected() != Some(true) {
                    trace!(category = "[event_loop]"; "Not connected and stop requested, exiting event loop...");
                    break;
                }
                let difference = stop_at.unwrap_or(chrono::DateTime::<Utc>::MIN_UTC) - Utc::now();
                if difference.num_seconds() <= 0 {
                    info!(category = "[event_loop]"; "Graceful shutdown period elapsed, forcing disconnection...");
                    break;
                }
                let duration = difference.to_std().unwrap_or(DURATION_ZERO);
                trace!(category = "[event_loop]"; "Stop requested, will wait for {} before forcing disconnection...", pretty_format(duration));
                duration
            } else {
                DURATION_MAX
            };
            if log_enabled!(log::Level::Trace) {
                if wait_duration.as_secs() == u64::MAX {
                    trace!(category = "[event_loop]"; "Waiting indefinitely for events...");
                } else {
                    trace!(category = "[event_loop]"; "Waiting for events for {}...", pretty_format(wait_duration));
                }
            }
            tokio::select! {
                sleep_result = self
                    .cancellation_token
                    .wait_on(time::sleep(wait_duration)) => {
                    match sleep_result {
                        Ok(_) => {
                            error!(category = "[event_loop]"; "Forcing disconnection...");
                            break;
                        }
                        Err(_) => {
                            info!(category = "[event_loop]"; "Received Ctrl+C, shutting down...");
                            should_stop = true;
                            stop_at = Some(Utc::now() + DURATION_UNTIL_SHUTDOWN);
                            if self.is_connected() == Some(true) {
                                trace!(category = "[event_loop]"; "Was waiting for event loop and connected, will make unavailable and disconnect after stop request...");
                                self.tx.send(ChannelMessages::Stopping)?;
                            } else {
                                trace!(category = "[event_loop]"; "Was waiting for event loop and not connected, no need to wait for event loop and will skip make unavailable and disconnecting...");
                                break;
                            }
                        }
                    }
                }
                loop_result = eventloop.poll() => {
                    trace!(category = "[event_loop]"; "Event loop polled, result: {loop_result:?}");
                    match self.deal_with_event(loop_result, should_stop).await {
                        Ok(event_handled) => {
                            should_wait = event_handled.should_wait;
                            if event_handled.should_break {
                                trace!(category = "[event_loop]"; "Event handled, breaking as requested...");
                                break;
                            }
                            trace!(category = "[event_loop]"; "Event handled, continuing event loop pool...");
                        }
                        Err(EventHandlingError::Connection(msg)) => {
                            error!(category = "[event_loop]"; "Connection error ({msg}), exiting event loop...");
                            return Err(Error::Connection(format!("{msg} in the event loop.")));
                        }
                        Err(e) => {
                            error!(category = "[event_loop]"; "Error dealing with event: {e}, will retry in 5 seconds...");
                            should_wait = true;
                        }
                    }
                }
            }
        }
        debug!(category = "[event_loop]"; "Exiting event loop");
        Ok(())
    }

    pub fn publish_sensor_data_periodically(
        &self,
        rx: Receiver<ChannelMessages>,
        devices: Devices,
        device_providers: Arc<Vec<Box<dyn DeviceProvider>>>,
    ) -> Result<()> {
        trace!("Starting periodic sensor data publishing task...");
        let other_mqtt_device = self.clone();
        tokio::spawn(async move {
            match other_mqtt_device
                .maintain_message_traffic(rx, devices, device_providers)
                .await
            {
                Ok(_) => trace!("Periodic sensor data publishing task exited normally"),
                Err(e) => error!("Error in periodic sensor data publishing task: {e}"),
            }
        });
        Ok(())
    }

    /// this runs in a separate thread, handling messages from the channel
    pub async fn maintain_message_traffic(
        self,
        mut rx: Receiver<ChannelMessages>,
        devices: Devices,
        device_providers: Arc<Vec<Box<dyn DeviceProvider>>>,
    ) -> Result<()> {
        let mut connection_manager = ConnectionManager::new(devices.clone());
        loop {
            trace!(category = "[message_traffic]"; "Before waiting for next message...");
            match self.cancellation_token.wait_on(rx.changed()).await {
                Err(_) => {
                    trace!(category = "[message_traffic]"; "Cancellation requested, stopping connection manager and update publishing loop...");
                    connection_manager
                        .deal_with_connection_status_change_and_manage_periodic_publishing(
                            &self,
                            device_providers.clone(),
                            false,
                        )
                        .await?;
                    continue;
                }
                Ok(Err(_)) => {
                    trace!(category = "[message_traffic]"; "Channel closed, exiting message sync loop");
                    return Err(Error::ChannelClosed);
                }
                Ok(Ok(_)) => {
                    trace!(category = "[message_traffic]"; "Channel changed, processing message...");
                }
            }
            let message = rx.borrow_and_update().clone();
            trace!(category = "[message_traffic]"; "Got next channel message: {:?}", message);
            match message {
                ChannelMessages::Connected(is_connected_message) => {
                    connection_manager
                        .deal_with_connection_status_change_and_manage_periodic_publishing(
                            &self,
                            device_providers.clone(),
                            is_connected_message,
                        )
                        .await?
                }
                ChannelMessages::Message(publish_result) => {
                    self.deal_with_command(publish_result.clone(), devices.clone()).await?
                }
                ChannelMessages::Stopping => {
                    trace!(category = "[message_traffic]"; "Received stop message, will stop state publishing...");
                    connection_manager
                        .deal_with_connection_status_change_and_manage_periodic_publishing(
                            &self,
                            device_providers.clone(),
                            false,
                        )
                        .await?;
                    trace!(category = "[message_traffic]"; "Processing stop message, will make devices unavailable...");
                    self.make_unavailable().await?;
                    trace!(category = "[message_traffic]"; "Processing stop message, will disconnect...");
                    self.disconnect().await?;
                    trace!(category = "[message_traffic]"; "Processing stop message, exiting message sync loop...");
                    connection_manager.stop().await;
                    trace!(category = "[message_traffic]"; "Processed stop message, done.");
                    break;
                }
            }
            trace!(category = "[message_traffic]"; "Channel loop iteration complete");
        }
        trace!(category = "[message_traffic]"; "Exiting message traffic handling function");
        Ok(())
    }

    pub async fn publish_to_client(&self, topic: String, payload: String) -> Result<()> {
        Ok(self
            .run_with_cancellation_and_timeout(self.client.publish(topic, QoS::AtLeastOnce, false, payload))
            .await??)
    }

    pub async fn subscribe_to_client(&self, topic: String) -> Result<()> {
        Ok(self
            .run_with_cancellation_and_timeout(self.client.subscribe(topic, QoS::AtLeastOnce))
            .await??)
    }

    async fn run_with_cancellation_and_timeout<F>(&self, future: F) -> Result<<F as IntoFuture>::Output>
    where
        F: IntoFuture,
    {
        let result = self
            .cancellation_token
            .wait_on(timeout(self.default_timeout, future.into_future()))
            .await??;
        Ok(result)
    }
}

#[derive(Debug, PartialEq)]
pub struct CommandResult {
    pub handled: bool,
    pub state_update_topics: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct PublishResult {
    pub topic: String,
    pub payload: String,
}

struct ConnectionManager {
    connected: bool,
    join_handle: Option<JoinHandle<()>>,
    cancellation_token_source: Option<CancellationTokenSource>,
    devices: Devices,
}

impl ConnectionManager {
    fn new(devices: Devices) -> Self {
        Self {
            connected: false,
            join_handle: None,
            cancellation_token_source: None,
            devices,
        }
    }

    async fn stop(mut self) {
        trace!("Stopping ConnectionManager, cancelling periodic publishing task...");
        if let Some(cancellation_token_source) = &mut self.cancellation_token_source {
            cancellation_token_source.cancel().await;
        }
    }

    pub async fn deal_with_connection_status_change_and_manage_periodic_publishing(
        &mut self,
        device_manager: &DeviceManager,
        device_providers: Arc<Vec<Box<dyn DeviceProvider>>>,
        is_connected_message: bool,
    ) -> Result<()> {
        trace!(
            "Dealing with connection status change, is_connected_message: {is_connected_message}, current connected state: {}",
            self.connected
        );
        if self.join_handle.is_some() {
            if self.connected && is_connected_message {
                trace!("Periodic sensor data publishing task is already running, skipping restart");
                return Ok(());
            } else if !self.connected && !is_connected_message {
                trace!("Periodic sensor data publishing task is not running, skipping abort");
                return Ok(());
            } else if self.connected {
                trace!("Was connected, now is disconnected, will abort periodic sensor data publishing task");
            } else {
                trace!("Was not connected, now is connected, will start periodic sensor data publishing task below");
            }
            debug!("Aborting periodic sensor data publishing task...");
            self.cancellation_token_source
                .take()
                .unwrap_or_else(|| {
                    error!("Cancellation token source missing when aborting periodic sensor data publishing task, creating new one for cancellation...");
                    CancellationTokenSource::new()
                })
                .cancel()
                .await;
            let timeout_duration = DURATION_UNTIL_SHUTDOWN;
            let wait_until = Utc::now() + timeout_duration;
            trace!(
                "Waiting for periodic sensor data publishing task to finish, will timeout in {}...",
                pretty_format(timeout_duration)
            );
            if let Some(join_handle) = self.join_handle.take() {
                while !join_handle.is_finished() && Utc::now() < wait_until {
                    trace!("Waiting for periodic sensor data publishing task to finish...");
                    time::sleep(DURATION_QUICK_CYCLE).await;
                }
                if join_handle.is_finished() {
                    trace!("Periodic sensor data publishing task finished.");
                } else {
                    error!("Periodic sensor data publishing task did not finish in time, aborting...");
                    join_handle.abort();
                }
            } else {
                error!(
                    "Join handle missing when aborting periodic sensor data publishing task, cannot wait for it to finish."
                );
            }
        }
        self.connected = is_connected_message;
        if is_connected_message {
            debug!("Channel received Connected message");
            trace!("Initializing devices immediately after connection...");
            device_manager.initialize(self.devices.clone()).await?;
            trace!("Device manager initialized, starting periodic sensor data publishing task...");
            let devices_clone = self.devices.clone();
            let mut cancellation_token_source = CancellationTokenSource::new();
            let cancellation_token = cancellation_token_source.create_token().await;
            self.cancellation_token_source = Some(cancellation_token_source);
            let other_mqtt_device = device_manager.clone();
            let other_device_provider = device_providers.clone();
            self.join_handle = Some(tokio::spawn(async move {
                let _publish_sensor_data_result = Self::publish_sensor_data_in_a_loop(
                    other_mqtt_device,
                    other_device_provider,
                    devices_clone.clone(),
                    cancellation_token,
                )
                .await; // todo: handle result
            }));
        } else {
            trace!("Channel received Disconnected message");
        }
        Ok(())
    }

    /// This runs in a separate thread, publishing sensor data periodically until cancellation is requested.
    pub async fn publish_sensor_data_in_a_loop(
        device_manager: DeviceManager,
        device_providers: Arc<Vec<Box<dyn DeviceProvider>>>,
        devices: Devices,
        cancellation_token: CancellationToken,
    ) -> Result<()> {
        let mut event_producers = create_event_producers(
            &devices,
            device_providers,
            device_manager.availability_topic(),
            cancellation_token.clone(),
            device_manager.publish_interval, // todo: take as param?
        )
        .await;
        while let Some(event_producer) = event_producers.next().await {
            match event_producer {
                Err(crate::update_engine::Error::Cancellation(_)) => {
                    trace!("Done publishing sensor data loop, exiting...");
                    break;
                }
                Err(err) => {
                    error!(category = "publish_sensor_data_in_a_loop"; "Error getting next event from update engine: {err}");
                }
                Ok(entities_data) => {
                    for entity_data in entities_data {
                        match entity_data {
                            UpdateEvent::Data(data) => {
                                trace!("Publishing sensor data to Home Assistant...");
                                if let Err(err) = device_manager.publish_sensor_data(data).await {
                                    error!(category = "publish_sensor_data_in_a_loop"; "Error publishing sensor data: {err}");
                                } else {
                                    trace!("Published all sensor data to Home Assistant");
                                }
                            }
                            UpdateEvent::DevicesRemoved(devices_vec) => {
                                if devices_vec.is_empty() {
                                    warn!("No devices to remove, skipping...");
                                    continue;
                                }
                                if log_enabled!(log::Level::Trace) {
                                    let ids_list = devices_vec
                                        .iter()
                                        .async_map(async |device_lock| {
                                            let d = device_lock.read().await;
                                            d.details.identifier.clone()
                                        })
                                        .await
                                        .join(", ");
                                    trace!("Removing device entities from Home Assistant, ids: {ids_list}...");
                                }
                                device_manager
                                    .publish_removed_entities_discovery(
                                        Devices::new_from_many_shared_devices(devices_vec, cancellation_token.clone())
                                            .await,
                                    )
                                    .await?
                            }
                            UpdateEvent::DevicesCreated(new_device_ids) => {
                                trace!("Publishing new device entities to Home Assistant...");
                                let new_devices = devices.filter(new_device_ids).await;
                                device_manager.publish_entities_discovery(new_devices.clone()).await?;
                                time::sleep(Duration::from_secs(1)).await;
                                device_manager.subscribe_to_commands(new_devices).await?;
                                device_manager.make_available().await?
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Elapsed(#[from] time::error::Elapsed),
    #[error(transparent)]
    Devices(#[from] crate::devices::Error),
    #[error("Client error: {0}")]
    Mqtt(String),
    #[error(transparent)]
    EventHandling(#[from] EventHandlingError),
    #[error(transparent)]
    Send(#[from] tokio::sync::watch::error::SendError<ChannelMessages>),
    #[error(transparent)]
    Tls(#[from] rustls::Error),
    #[error(transparent)]
    UpdateEngine(#[from] crate::update_engine::Error),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Channel closed")]
    ChannelClosed,
    #[error(transparent)]
    CancellationRequested(#[from] crate::cancellation_token::Error),
    #[error("Error in publish sensor loop")]
    PublishSensorLoop,
}

impl From<ClientError> for Error {
    fn from(e: ClientError) -> Self {
        Error::Mqtt(e.to_string())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum EventHandlingError {
    #[error(transparent)]
    Send(#[from] tokio::sync::watch::error::SendError<ChannelMessages>),
    #[error("{0}")]
    Connection(String),
}

#[cfg(test)]
mod mqtt_client {
    use rumqttc::v5::{ClientError, ConnectionError, Event, MqttOptions, mqttbytes::QoS};
    use std::pin::Pin;
    mockall::mock! {
        pub EventLoop {
            pub fn poll(&mut self) -> Pin<Box<dyn Future<Output = Result<Event, ConnectionError>> + Send>>;
        }
    }
    mockall::mock! {
        #[derive(Debug, Default)]
        pub AsyncClient {
            pub fn new(options: MqttOptions, capacity: usize) -> (Self, MockEventLoop);
            pub fn publish(&self, topic: String, qos: QoS, retain: bool, payload: String) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send>>;
            pub fn subscribe(&self, topic: String, qos: QoS) -> Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send>>;
            pub fn try_disconnect(&self) -> Result<(), ClientError>;
        }
        impl Clone for AsyncClient {
            fn clone(&self) -> Self;
        }
    }
}
#[cfg(test)]
mod tests {
    use std::future;

    use crate::devices::EntityDetails;
    use crate::devices::MockHandlesData;
    use crate::devices::test_helpers::*;

    use super::*;
    use mockall::predicate;
    use mqtt_client::__mock_MockAsyncClient::__new::Context;
    use serde_json::json;
    use serial_test::serial;
    use tokio::time::Duration;
    fn create_mock_client(setup_fn: fn(&mut AsyncClient)) -> Context {
        let mut mock_client = AsyncClient::default();
        setup_fn(&mut mock_client);
        let ctx = AsyncClient::new_context();
        ctx.expect().return_once(move |_, _| (mock_client, EventLoop::new()));
        ctx
    }

    fn make_device_manager() -> DeviceManager {
        let (manager, _, _) = DeviceManager::new(
            "localhost".to_string(),
            1883,
            "node1".to_string(),
            "u".to_string(),
            "p".to_string(),
            true,
            Duration::from_millis(10),
            CancellationToken::default(),
        )
        .unwrap();
        manager
    }

    #[tokio::test]
    #[serial]
    async fn test_publish_entities_discovery() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .with(
                    predicate::function(|t: &String| t == "homeassistant/device/test_device/config"),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::always(),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
        });
        make_device_manager()
            .publish_entities_discovery(make_empty_devices())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_subscribe_to_commands() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_subscribe()
                .with(
                    predicate::eq("homeassistant/status".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                )
                .returning(|_, _| Box::pin(async { Ok(()) }))
                .times(1);
        });
        make_device_manager()
            .subscribe_to_commands(make_empty_devices())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_publish_sensor_data() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .with(
                    predicate::eq("a/topic".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("payload1".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
            mock_client
                .expect_publish()
                .with(
                    predicate::eq("b/topic".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("42".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
        });
        make_device_manager()
            .publish_sensor_data(hashmap! {
                "a/topic".to_string() => "payload1".to_string(),
                "b/topic".to_string() => "42".to_string()
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_set_availability_only_when_connected() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .with(
                    predicate::eq("node1/availability".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("online".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
        });
        let manager = make_device_manager();
        manager.set_availability(true).await.unwrap();
        manager.set_connected(true);
        manager.set_availability(true).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_disconnect_only_when_connected() {
        let _c = create_mock_client(|mock_client| {
            mock_client.expect_try_disconnect().returning(|| Ok(())).times(1);
        });
        let manager = make_device_manager();
        manager.disconnect().await.unwrap();
        manager.set_connected(true);
        manager.disconnect().await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_initialize_sequence_with_empty_device() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .with(
                    predicate::function(|t: &String| t == "homeassistant/device/test_device/config"),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::always(),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
            mock_client
                .expect_subscribe()
                .with(
                    predicate::eq("homeassistant/status".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                )
                .returning(|_, _| Box::pin(async { Ok(()) }))
                .times(1);
            mock_client
                .expect_clone()
                .returning(|| {
                    let mut cloned_mock_client = AsyncClient::default();
                    cloned_mock_client
                        .expect_publish()
                        .with(
                            predicate::eq("dev1/availability".to_string()),
                            predicate::eq(QoS::AtLeastOnce),
                            predicate::eq(false),
                            predicate::eq("online".to_string()),
                        )
                        .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                        .times(1);
                    cloned_mock_client
                })
                .times(1);
        });
        make_device_manager().initialize(make_empty_devices()).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_initialize_sequence_with_device_with_entities_sends_state_and_subscribes() {
        let _c = create_mock_client(|client| {
            client
                .expect_publish()
                .with(
                    predicate::function(|t: &String| t == "homeassistant/device/test_device/config"),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::always(),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
            client
                .expect_subscribe()
                .with(
                    predicate::eq("homeassistant/status".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                )
                .returning(|_, _| Box::pin(async { Ok(()) }))
                .times(1);
            client
                .expect_subscribe()
                .with(
                    predicate::eq("dev1/test_switch/command".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                )
                .returning(|_, _| Box::pin(async { Ok(()) }))
                .times(1);
            client
                .expect_publish()
                .with(
                    predicate::eq("dev1/test_switch/state".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("OFF".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
            client
                .expect_clone()
                .returning(|| {
                    let mut cloned_mock_client = AsyncClient::default();
                    cloned_mock_client
                        .expect_publish()
                        .with(
                            predicate::eq("dev1/availability".to_string()),
                            predicate::eq(QoS::AtLeastOnce),
                            predicate::eq(false),
                            predicate::eq("online".to_string()),
                        )
                        .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                        .times(1);
                    cloned_mock_client
                })
                .times(1);
        });
        let mut entity_type = MockAnEntity::new();
        entity_type.expect_json_for_discovery().returning(|_, _| {
            Ok(json!({
                "name": "Test Switch",
                "unique_id": "node1_dev1_test_switch",
                "state_topic": "dev1/test_switch/state",
                "command_topic": "dev1/test_switch/command",
                "availability_topic": "dev1/availability",
                "payload_on": "ON",
                "payload_off": "OFF",
                "device": {
                    "identifiers": ["test_device"],
                    "name": "Test Device",
                    "manufacturer": "Mfg",
                    "sw_version": "1.0.0"
                }
            }))
        });
        entity_type.expect_details().return_const(
            EntityDetails::new("dev1".to_string(), "Test Switch".to_string(), "mdi:switch".to_string())
                .add_command("dev1/test_switch/command".to_string()),
        );
        let mut device = make_empty_device();
        device.entities.push(Box::new(entity_type));
        let mut data_handler = MockHandlesData::new();
        data_handler
            .expect_get_entity_data()
            .returning(|_| {
                Box::pin(future::ready(Ok(
                    hashmap! {"dev1/test_switch/state".to_string() => "OFF".to_string()},
                )))
            })
            .times(1);
        device.data_handlers.push(Box::new(data_handler));
        let manager = make_device_manager();
        manager.set_connected(true);
        manager
            .initialize(Devices::new_from_single_device(device, CancellationToken::default()))
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_deal_with_command_publishes_without_state_update() {
        let _c = create_mock_client(|_| {});
        let mut data_handler = MockHandlesData::new();
        data_handler
            .expect_handle_command()
            .with(
                predicate::eq("some/command"),
                predicate::eq("ignored"),
                predicate::always(),
            )
            .returning(|_, _, _| {
                Box::pin(async {
                    Ok(CommandResult {
                        handled: true,
                        state_update_topics: None,
                    })
                })
            })
            .times(1);
        let mut device = make_empty_device();
        if log_enabled!(log::Level::Debug) {
            let mut entity_type = MockAnEntity::new();
            entity_type.expect_details().return_const(
                EntityDetails::new("dev1".to_string(), "Test Switch".to_string(), "mdi:switch".to_string())
                    .add_command("dev1/test_switch/command".to_string()),
            );
        }
        device.data_handlers.push(Box::new(data_handler));
        make_device_manager()
            .deal_with_command(
                PublishResult {
                    topic: "some/command".to_string(),
                    payload: "ignored".to_string(),
                },
                Devices::new_from_single_device(device, CancellationToken::default()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_deal_with_command_publishes_with_state_update() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .with(
                    predicate::eq("x/topic".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("1".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
            mock_client
                .expect_publish()
                .with(
                    predicate::eq("y/topic".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("ON".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
        });
        let mut data_handler = MockHandlesData::new();
        data_handler
            .expect_handle_command()
            .with(
                predicate::eq("some/command"),
                predicate::eq("ignored"),
                predicate::always(),
            )
            .returning(|_, _, _| {
                Box::pin(async {
                    Ok(CommandResult {
                        handled: true,
                        state_update_topics: Some(hashmap!{"x/topic".to_string() => "1".to_string(), "y/topic".to_string() => "ON".to_string()}),
                    })
                })
            })
            .times(1);
        if log_enabled!(log::Level::Debug) {
            let mut entity_type = MockAnEntity::new();
            entity_type.expect_details().return_const(
                EntityDetails::new("dev1".to_string(), "Test Switch".to_string(), "mdi:switch".to_string())
                    .add_command("dev1/test_switch/command".to_string()),
            );
        }
        let mut device = make_empty_device();
        device.data_handlers.push(Box::new(data_handler));
        make_device_manager()
            .deal_with_command(
                PublishResult {
                    topic: "some/command".to_string(),
                    payload: "ignored".to_string(),
                },
                Devices::new_from_single_device(device, CancellationToken::default()),
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_remove_entities() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .with(
                    predicate::eq("homeassistant/device/test_device/config".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
        });
        let manager = make_device_manager();
        manager.set_connected(true);
        manager
            .publish_removed_entities_discovery(make_empty_devices())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_publish_sensor_data_for_all_devices_publishes_new_data() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .with(
                    predicate::eq("dev1/test_sensor/state".to_string()),
                    predicate::eq(QoS::AtLeastOnce),
                    predicate::eq(false),
                    predicate::eq("42".to_string()),
                )
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }))
                .times(1);
        });

        let mut data_handler = MockHandlesData::new();
        data_handler
            .expect_get_entity_data()
            .returning(|_| {
                Box::pin(future::ready(Ok(
                    hashmap! {"dev1/test_sensor/state".to_string() => "42".to_string()},
                )))
            })
            .times(1);
        let mut device = make_empty_device();
        device.data_handlers.push(Box::new(data_handler));
        let devices = Devices::new_from_single_device(device, CancellationToken::default());

        let manager = make_device_manager();
        manager.set_connected(true);

        manager.publish_sensor_data_for_all_devices(devices).await.unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn test_connection_manager_new() {
        let devices = make_empty_devices();
        let connection_manager = ConnectionManager::new(devices);

        assert!(!connection_manager.connected);
        assert!(connection_manager.join_handle.is_none());
        assert!(connection_manager.cancellation_token_source.is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_connection_manager_stop_with_no_task() {
        let devices = make_empty_devices();
        let connection_manager = ConnectionManager::new(devices);

        connection_manager.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn test_connection_manager_transition_to_connected() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
            mock_client
                .expect_subscribe()
                .returning(|_, _| Box::pin(async { Ok(()) }));
            mock_client.expect_clone().returning(|| {
                let mut cloned = AsyncClient::default();
                cloned
                    .expect_publish()
                    .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
                cloned
            });
        });

        let devices = make_empty_devices();
        let mut connection_manager = ConnectionManager::new(devices.clone());
        let manager = make_device_manager();

        assert!(!connection_manager.connected);

        connection_manager
            .deal_with_connection_status_change_and_manage_periodic_publishing(&manager, Arc::new(vec![]), true)
            .await
            .unwrap();

        assert!(connection_manager.connected);
        assert!(connection_manager.join_handle.is_some());
        assert!(connection_manager.cancellation_token_source.is_some());

        connection_manager.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn test_connection_manager_already_connected_skips_restart() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
            mock_client
                .expect_subscribe()
                .returning(|_, _| Box::pin(async { Ok(()) }));
            mock_client.expect_clone().returning(|| {
                let mut cloned = AsyncClient::default();
                cloned
                    .expect_publish()
                    .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
                cloned
            });
        });

        let devices = make_empty_devices();
        let mut connection_manager = ConnectionManager::new(devices.clone());
        let manager = make_device_manager();

        connection_manager
            .deal_with_connection_status_change_and_manage_periodic_publishing(&manager, Arc::new(vec![]), true)
            .await
            .unwrap();

        let first_handle_id = connection_manager.join_handle.as_ref().map(|h| h.id()).unwrap();

        connection_manager
            .deal_with_connection_status_change_and_manage_periodic_publishing(&manager, Arc::new(vec![]), true)
            .await
            .unwrap();

        let second_handle_id = connection_manager.join_handle.as_ref().map(|h| h.id()).unwrap();

        assert_eq!(first_handle_id, second_handle_id);

        connection_manager.stop().await;
    }

    #[tokio::test]
    #[serial]
    async fn test_connection_manager_already_disconnected_skips_abort() {
        let _c = create_mock_client(|_| {});

        let devices = make_empty_devices();
        let mut connection_manager = ConnectionManager::new(devices.clone());
        let manager = make_device_manager();

        assert!(!connection_manager.connected);

        connection_manager
            .deal_with_connection_status_change_and_manage_periodic_publishing(&manager, Arc::new(vec![]), false)
            .await
            .unwrap();

        assert!(!connection_manager.connected);
        assert!(connection_manager.join_handle.is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_connection_manager_transition_to_disconnected() {
        let _c = create_mock_client(|mock_client| {
            mock_client
                .expect_publish()
                .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
            mock_client
                .expect_subscribe()
                .returning(|_, _| Box::pin(async { Ok(()) }));
            mock_client.expect_clone().returning(|| {
                let mut cloned = AsyncClient::default();
                cloned
                    .expect_publish()
                    .returning(|_, _, _, _| Box::pin(async { Ok(()) }));
                cloned
            });
        });

        let devices = make_empty_devices();
        let mut connection_manager = ConnectionManager::new(devices.clone());
        let mut device_manager = make_device_manager();
        device_manager.publish_interval = Duration::from_millis(1000);
        let mut cancellation_token_source = CancellationTokenSource::new();
        device_manager.cancellation_token = cancellation_token_source.create_token().await;

        connection_manager
            .deal_with_connection_status_change_and_manage_periodic_publishing(&device_manager, Arc::new(vec![]), true)
            .await
            .unwrap();

        assert!(connection_manager.connected);
        assert!(connection_manager.join_handle.is_some());

        connection_manager
            .deal_with_connection_status_change_and_manage_periodic_publishing(&device_manager, Arc::new(vec![]), false)
            .await
            .unwrap();

        assert!(!connection_manager.connected);
        assert!(connection_manager.join_handle.is_none());
        assert!(connection_manager.cancellation_token_source.is_none());
    }
}
