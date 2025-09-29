use chrono::Utc;
use rumqttc::v5::ConnectionError::*;
use rumqttc::{
    Transport,
    v5::{
        AsyncClient, ConnectionError, Event, EventLoop, MqttOptions,
        mqttbytes::{QoS, v5::Packet},
    },
};
use rustls_platform_verifier::ConfigVerifierExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;
use tokio::{
    sync::watch::{Receiver, Sender},
    task::JoinHandle,
    time::{self, timeout},
};
use tokio_rustls::rustls::ClientConfig;

use crate::cancellation_token::{CancellationToken, CancellationTokenSource};
use crate::datetime::pretty_format;
use crate::ha_devices::{Device, EntityData};
use crate::sleep::sleep_cancellable;

#[derive(Clone)]
pub struct DeviceManager {
    client: AsyncClient,
    discovery_prefix: String,
    connected: Arc<RwLock<Option<bool>>>,
    tx: Sender<ChannelMessages>,
    default_timeout: Duration,
}

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

impl DeviceManager {
    pub fn new(
        broker_host: &str,
        broker_port: u16,
        connection_id: &str,
        username: &str,
        password: &str,
        disable_tls: bool,
    ) -> Result<(Self, EventLoop, Receiver<ChannelMessages>), Box<dyn std::error::Error>> {
        let mut mqtt_options = MqttOptions::new(connection_id, broker_host, broker_port);
        mqtt_options.set_keep_alive(Duration::from_secs(45));
        mqtt_options.set_clean_start(false);
        mqtt_options.set_credentials(username, password);
        if disable_tls {
            warn!("WARNING: TLS is disabled, connection will be unencrypted!");
        } else {
            mqtt_options.set_transport(Transport::tls_with_config(
                ClientConfig::with_platform_verifier()?.into(),
            ));
        }
        info!(
            "Connecting to MQTT broker at {broker_host}:{broker_port} with connection ID: {connection_id} and user: {username}"
        );
        let (client, eventloop) = AsyncClient::new(mqtt_options, 10);
        let (tx, rx) = tokio::sync::watch::channel(ChannelMessages::Connected(false));
        Ok((
            Self {
                client,
                discovery_prefix: "homeassistant".to_string(),
                connected: Arc::new(RwLock::new(None)),
                tx,
                default_timeout: Duration::from_secs(5),
            },
            eventloop,
            rx,
        ))
    }

    async fn publish_entities_discovery(
        &self,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for device in devices.read().await.iter() {
            let discovery_json = device.json_for_discovery().await?.to_string();
            let discovery_topic = device.discovery_topic(&self.discovery_prefix);
            trace!(
                "Publishing discovery for device: {}, topic: {discovery_topic}, payload: {discovery_json}",
                device.details.name,
            );
            timeout(
                self.default_timeout,
                self.client
                    .publish(discovery_topic, QoS::AtLeastOnce, false, discovery_json),
            )
            .await??;
            info!(
                "Published device discoveries for device: {}",
                device.details.name
            );
        }
        Ok(())
    }

    async fn subscribe_to_commands(
        &self,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!("Subscribing to Home Assistant status topic...");
        timeout(
            self.default_timeout,
            self.client.subscribe(
                format!("{}/status", &self.discovery_prefix),
                QoS::AtLeastOnce,
            ),
        )
        .await??;
        trace!("Subscribed to Home Assistant status topic");
        info!("Subscribing to command topics...");
        for device in devices.read().await.iter() {
            for command_topic in device.command_topics().iter() {
                timeout(
                    self.default_timeout,
                    self.client.subscribe(command_topic, QoS::AtLeastOnce),
                )
                .await??;
                trace!(
                    "Subscribed to command topic {command_topic} for device {}",
                    device.details.name
                );
            }
        }
        trace!("Subscribed to command topics");
        Ok(())
    }

    async fn _remove_entities(
        &self,
        devices: &Vec<Device>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        trace!("Removing all devices and entities from Home Assistant...");
        for device in devices {
            info!("Making device {} unavailable.", device.details.name);
            timeout(
                self.default_timeout,
                self.client.publish(
                    device.availability_topic(),
                    QoS::AtLeastOnce,
                    false,
                    "offline",
                ),
            )
            .await??;
            info!(
                "Removing device entities for device: {}",
                device.details.name
            );
            timeout(
                self.default_timeout,
                self.client.publish(
                    device.discovery_topic(&self.discovery_prefix),
                    QoS::AtLeastOnce,
                    false,
                    "",
                ),
            )
            .await??;
        }
        info!("Removed all devices and entities from Home Assistant");
        Ok(())
    }

    async fn publish_sensor_data_for_all_devices(
        &self,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        trace!("Publishing sensor data to Home Assistant...");
        if *self.connected.read().await == Some(false) {
            trace!("Not connected to MQTT broker, skipping sensor data publishing");
            return Ok(());
        }
        info!("Publishing sensor data...");
        let mut entities_data = Vec::<EntityData>::new();
        let devices_lock = devices.read().await;
        let devices_iter = devices_lock.iter();
        for device in devices_iter {
            trace!("Getting entities data for device: {}", device.details.name);
            let data = device.get_entities_data().await?;
            trace!(
                "Got entities data for device: {}: {data:?}",
                device.details.name
            );
            entities_data.extend(data);
        }
        self.publish_sensor_data(entities_data).await?;
        trace!("Published all sensor data to Home Assistant");
        Ok(())
    }

    async fn publish_sensor_data(
        &self,
        entities_data: Vec<EntityData>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for entity_data in entities_data {
            if log_enabled!(log::Level::Trace) {
                trace!(
                    "Publishing sensor data to topic: {}, payload: {}",
                    &entity_data.topic, &entity_data.payload
                );
            } else if log_enabled!(log::Level::Info) {
                info!("Publishing sensor data to topic: {}", &entity_data.topic);
            }
            timeout(
                self.default_timeout,
                self.client.publish(
                    entity_data.topic,
                    QoS::AtLeastOnce,
                    false,
                    entity_data.payload,
                ),
            )
            .await??;
            trace!("Published sensor data to topic");
        }
        Ok(())
    }

    async fn deal_with_command(
        &self,
        publish_result: PublishResult,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        trace!(
            "Dealing with command on topic: {}, payload: {}",
            publish_result.topic, publish_result.payload
        );
        if publish_result.topic == format!("{}/status", self.discovery_prefix) {
            if publish_result.payload == "online" {
                info!("Broker is online, initializing in 5 seconds...");
                time::sleep(self.default_timeout).await;
                self.initialize(devices).await?;
            } else {
                info!("Broker is going offline...");
            }
            return Ok(());
        }
        let mut handled = false;
        let mut devices_lock = devices.write().await;
        for device in devices_lock.iter_mut() {
            trace!(
                "Checking if device {} can handle command...",
                device.details.name
            );
            let command_handle_result = device
                .handle_command(&publish_result.topic, &publish_result.payload)
                .await?;
            if command_handle_result.handled {
                trace!(
                    "Device {} handled command on topic: {}, payload: {}",
                    device.details.name, publish_result.topic, publish_result.payload,
                );
                match command_handle_result.state_update {
                    Some(state_update) if !state_update.is_empty() => {
                        trace!("Command resulted in state update: {state_update:?}");
                        match self.publish_sensor_data(state_update).await {
                            Ok(_) => trace!("Sensor data published after command handling"),
                            Err(e) => {
                                error!(category = "deal_with_command"; "Error publishing sensor data after command handling: {e}")
                            }
                        }
                    }
                    _ => {
                        trace!("Command did not result in state update");
                    }
                }
            } else {
                trace!(
                    "Device {} did not handle command on topic: {}, payload: {}",
                    device.details.name, publish_result.topic, publish_result.payload
                );
            }
            handled = handled || command_handle_result.handled;
            trace!(
                "Device {} finished checking command, will now go to next device...",
                device.details.name
            );
        }
        if !handled {
            warn!(
                "Received message on unknown topic: {} and payload:\n{}",
                publish_result.topic, publish_result.payload
            );
        }
        trace!("Completed dealing with command");
        Ok(())
    }

    async fn disconnect(&self) -> Result<(), Box<dyn std::error::Error>> {
        trace!("Disconnecting from MQTT broker if connected...");
        if *self.connected.read().await == Some(true) {
            info!("Disconnecting from MQTT broker...");
            self.client.try_disconnect()?;
        } else {
            trace!("Not connected to MQTT broker, skipping disconnect");
        }
        Ok(())
    }

    async fn make_available(
        &self,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.set_availability(true, devices).await?;
        Ok(())
    }

    fn make_unavailable(
        &self,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> impl Future<Output = Result<(), Box<dyn std::error::Error>>> {
        self.set_availability(false, devices)
    }

    async fn set_availability(
        &self,
        available: bool,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let availability_text = if available { "online" } else { "offline" };
        trace!("Publishing availability as {availability_text} for all devices...");
        if *self.connected.read().await == Some(true) {
            for device in devices.read().await.iter() {
                info!(
                    "Publishing availability as {availability_text} for device {}...",
                    device.details.name
                );
                timeout(
                    self.default_timeout,
                    self.client.publish(
                        device.availability_topic(),
                        QoS::AtLeastOnce,
                        false,
                        availability_text,
                    ),
                )
                .await??;
                trace!("Published availability to topic");
            }
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        let is_connected = { *self.connected.read().await };
        match is_connected {
            Some(true) => {
                error!("Disconnected from MQTT broker after event {event}: Message: {message}");
                *self.connected.write().await = Some(false);
                self.tx.send(ChannelMessages::Connected(false))?;
                debug!("Sent disconnected message to channel after event {event}");
            }
            Some(false) => trace!(
                "Not sending disconnected message to channel because was already disconnected"
            ),
            None => {
                error!("Initial connection failed with event: {event}");
                *self.connected.write().await = Some(false);
            }
        }
        Ok(())
    }

    async fn deal_with_event(
        &mut self,
        event_result: Result<Event, ConnectionError>,
        should_stop: bool,
    ) -> Result<EventHandled, Box<dyn std::error::Error>> {
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
                *self.connected.write().await = Some(true);
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
                std::process::exit(1);
            }
            Err(Io(x)) => {
                trace!("Received I/O error: {x}, will wait...");
                self.got_disconnected(
                    &format!("MQTT I/O connection error ({x}), will retry..."),
                    "I/O Error",
                )
                .await?;
                should_wait = true;
            }
            Err(Tls(e)) => {
                error!("MQTT TLS error ({e}), exiting...");
                std::process::exit(1);
            }
            Err(MqttState(error)) => {
                if should_stop {
                    debug!("Got MQTT state error after stop request ({error}), exiting loop...");
                    should_break = true;
                } else {
                    trace!("Received MQTT state error ({error}), will wait...");
                    self.got_disconnected(
                        "MQTT state error, retrying in 5 seconds...",
                        "I/O Error",
                    )
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
    pub async fn initialize(
        &self,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        debug!("Initializing DeviceManager...");
        trace!("Publishing entities discovery on initialization...");
        self.publish_entities_discovery(devices.clone())
            .await
            .unwrap_or_else(|e| error!("Error publishing entities discovery: {e}"));
        trace!("Subscribing to command topics on initialization...");
        self.subscribe_to_commands(devices.clone()).await?;
        trace!("Publishing sensor data for all devices on initialization...");
        self.publish_sensor_data_for_all_devices(devices.clone())
            .await?;
        trace!("Making devices available on initialization...");
        self.make_available(devices).await?;
        trace!("Initializatin done...");
        Ok(())
    }

    pub async fn deal_with_event_loop(
        &mut self,
        mut eventloop: EventLoop,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut should_stop = false;
        let mut stop_at = None;
        let mut should_wait = false;
        loop {
            if should_wait {
                trace!("Sleeping...");
                should_wait = false;
                if !sleep_cancellable(Duration::from_secs(5)).await.completed {
                    info!("Received Ctrl+C, shutting down...");
                    should_stop = true;
                    stop_at = Some(Utc::now() + Duration::from_secs(5));
                    if *self.connected.read().await == Some(true) {
                        trace!(
                            "Was connected, will make unavailable and disconnect after stop request..."
                        );
                        self.tx.send(ChannelMessages::Stopping)?;
                    } else {
                        trace!(
                            "Was not connected, no need to wait for event loop and will skip make unavailable and disconnecting..."
                        );
                        break;
                    }
                }
            }
            let wait_duration = if should_stop {
                if *self.connected.read().await == Some(false) {
                    trace!("Not connected and stop requested, exiting event loop...");
                    break;
                }
                let difference: chrono::TimeDelta = stop_at.unwrap() - Utc::now();
                if difference.num_seconds() <= 0 {
                    info!("Graceful shutdown period elapsed, forcing disconnection...");
                    break;
                }
                difference.to_std().unwrap_or(Duration::from_secs(0))
            } else {
                Duration::from_secs(u64::MAX)
            };
            if log_enabled!(log::Level::Trace) {
                if wait_duration.as_secs() == u64::MAX {
                    trace!("Waiting indefinitely for events...");
                } else {
                    trace!("Waiting for events for {}...", pretty_format(wait_duration));
                }
            }
            tokio::select! {
                sleep_result = sleep_cancellable(wait_duration) => {
                    if sleep_result.completed {
                        error!("Forcing disconnection...");
                        break;
                    } else {
                        info!("Received Ctrl+C, shutting down...");
                        should_stop = true;
                        stop_at = Some(Utc::now() + Duration::from_secs(5));
                        if *self.connected.read().await == Some(true) {
                            trace!("Was connected, will make unavailable and disconnect after stop request...");
                            self.tx.send(ChannelMessages::Stopping)?;
                        } else {
                            trace!("Was not connected, no need to wait for event loop and will skip make unavailable and disconnecting...");
                            break;
                        }
                    }
                }
                loop_result = eventloop.poll() => {
                    trace!("Event loop polled, result: {loop_result:?}");
                    match self.deal_with_event(loop_result, should_stop).await {
                        Ok(event_handled) => {
                            should_wait = event_handled.should_wait;
                            if event_handled.should_break {
                                trace!("Event handled, breaking as requested...");
                                break;
                            }
                            trace!("Event handled, continuing event loop pool...");
                        }
                        Err(e) => {
                            error!("Error dealing with event: {e}, will retry in 5 seconds...");
                            should_wait = true;
                        }
                    }
                }
            }
        }
        debug!("Exiting event loop");
        Ok(())
    }

    pub fn publish_sensor_data_periodically(
        &self,
        rx: Receiver<ChannelMessages>,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        trace!("Starting periodic sensor data publishing task...");
        let other_mqtt_device = self.clone();
        // let _local = LocalSet::new();
        tokio::spawn(async move {
            match other_mqtt_device
                .maintain_message_traffic(rx, devices.clone())
                .await
            {
                Ok(_) => trace!("Periodic sensor data publishing task exited normally"),
                Err(e) => error!("Error in periodic sensor data publishing task: {e}"),
            }
        });
        Ok(())
    }

    pub async fn maintain_message_traffic(
        self,
        mut rx: Receiver<ChannelMessages>,
        devices: Arc<RwLock<Vec<Device>>>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut connection_manager = ConnectionManager {
            connected: false,
            join_handle: None,
            cancellation_token_source: None,
            devices: devices.clone(),
        };
        loop {
            trace!("Before waiting for next message...");
            if rx.changed().await.is_err() {
                info!("Channel closed, exiting message sync loop");
                break;
            }
            let message = rx.borrow_and_update().clone();
            trace!("Got next channel message: {:?}", message);
            match message {
                ChannelMessages::Connected(is_connected_message) => {
                    connection_manager
                        .deal_with_connection_status_change_and_manage_periodic_publishing(
                            &self,
                            is_connected_message,
                        )
                        .await?
                }
                ChannelMessages::Message(publish_result) => {
                    self.deal_with_command(publish_result.clone(), devices.clone())
                        .await?
                }
                ChannelMessages::Stopping => {
                    trace!("Received stop message, will stop state publishing...");
                    connection_manager
                        .deal_with_connection_status_change_and_manage_periodic_publishing(
                            &self, false,
                        )
                        .await?;
                    trace!("Received stop message, will make devices unavailable...");
                    self.make_unavailable(devices.clone()).await?;
                    trace!("Received stop message, will disconnect...");
                    self.disconnect().await?;
                    trace!("Received stop message, done...");
                }
            }
            trace!("Channel loop iteration complete");
        }
        trace!("Exiting message traffic handling function");
        Ok(())
    }

    pub async fn publish_sensor_data_in_a_loop(
        self,
        devices: Arc<RwLock<Vec<Device>>>,
        cancellation_token: CancellationToken,
    ) {
        // todo: propagate cancellation token to all async calls
        let mut interval = time::interval(Duration::from_secs(15));
        interval.tick().await; // to set the initial instant
        loop {
            // this is the main loop for publishing sensor data periodically
            trace!(category = "main_loop"; "In the main loop for publishing sensor data, now will wait for next interval tick...");
            if cancellation_token.wait_on(interval.tick()).await.is_err() {
                debug!(category = "main_loop"; "Cancellation token triggered, stopping periodic sensor data publishing task");
                break;
            }
            trace!(category = "main_loop"; "Interval ticked, now checking connection state...");
            match cancellation_token
                .wait_on(self.publish_sensor_data_for_all_devices(devices.clone()))
                .await
            {
                Ok(Ok(())) => trace!(category = "main_loop"; "Published sensor data successfully"),
                Ok(Err(e)) => error!(category = "main_loop"; "Error publishing sensor data: {e}"),
                Err(_) => {
                    debug!(category = "main_loop"; "Cancellation token triggered while publishing sensor data, stopping periodic sensor data publishing task...");
                    break;
                }
            }
            trace!(category = "main_loop"; "Completed one iteration of periodic sensor data publishing task, now continuing...");
        }
    }
}

pub struct CommandResult {
    pub handled: bool,
    pub state_update: Option<Vec<EntityData>>,
}

#[derive(Debug, Clone)]
pub struct PublishResult {
    pub topic: String,
    pub payload: String,
}

struct ConnectionManager {
    connected: bool,
    join_handle: Option<JoinHandle<()>>, // todo: remove join handle? Using cancellation token should be enough
    cancellation_token_source: Option<CancellationTokenSource>,
    devices: Arc<RwLock<Vec<Device>>>,
}

impl ConnectionManager {
    pub async fn deal_with_connection_status_change_and_manage_periodic_publishing(
        &mut self,
        device_manager: &DeviceManager,
        is_connected_message: bool,
    ) -> Result<(), Box<dyn std::error::Error>> {
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
                trace!(
                    "Was connected, now is disconnected, will abort periodic sensor data publishing task"
                );
            } else {
                trace!(
                    "Was not connected, now is connected, will start periodic sensor data publishing task below"
                );
            }
            debug!("Aborting periodic sensor data publishing task...");
            self.cancellation_token_source
                .take()
                .unwrap()
                .cancel()
                .await;
            let join_handle = self.join_handle.take().unwrap();
            let timeout_duration = Duration::from_secs(5);
            let wait_until = Utc::now() + timeout_duration;
            trace!(
                "Waiting for periodic sensor data publishing task to finish, will timeout in {}...",
                pretty_format(timeout_duration)
            );
            while !join_handle.is_finished() && Utc::now() < wait_until {
                trace!("Waiting for periodic sensor data publishing task to finish...");
                time::sleep(Duration::from_millis(100)).await;
            }
            if join_handle.is_finished() {
                trace!("Periodic sensor data publishing task finished.");
            } else {
                error!("Periodic sensor data publishing task did not finish in time, aborting...");
                join_handle.abort();
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
            let cancellation_token = cancellation_token_source.create_token();
            self.cancellation_token_source = Some(cancellation_token_source);
            let other_mqtt_device = device_manager.clone();
            self.join_handle = Some(tokio::spawn(async move {
                other_mqtt_device
                    .publish_sensor_data_in_a_loop(devices_clone.clone(), cancellation_token)
                    .await;
            }));
        } else {
            trace!("Channel received Disconnected message");
        }
        Ok(())
    }
}
