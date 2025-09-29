extern crate docker_status_mqtt_proc_macros;
#[macro_use]
extern crate log;
#[macro_use]
mod macros;

use std::sync::Arc;

use rumqttc::v5::EventLoop;
use tokio::sync::{RwLock, watch::Receiver};
mod device_manager;
use device_manager::*;

use crate::{
    ha_devices::{Device, DeviceProvider},
    sample_device::SampleDeviceProvider,
};

mod cancellation_token;
mod datetime;
mod docker;
mod ha_devices;
mod logger;
mod sample_device;
mod sleep;
// mod observer;
// use docker::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    logger::start()?;
    run().await?;
    info!("Shutting down...");
    Ok(())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let device_name = std::env::var("DEVICE_NAME").unwrap_or("Rust Server Device".to_string());
    let device_providers: Vec<Box<dyn DeviceProvider>> = vec![Box::new(SampleDeviceProvider::new(
        device_name.clone(),
        true,
    ))];
    let devices = futures::future::join_all(
        device_providers
            .iter()
            .map(|provider| provider.get_devices()),
    )
    .await
    .into_iter()
    .flatten()
    .collect::<Vec<Device>>();
    let devices_arc = Arc::new(RwLock::new(devices));

    let (mut device_manager, eventloop, rx) = create_mqtt_connection()?;
    device_manager.publish_sensor_data_periodically(rx, devices_arc.clone())?;
    info!("Configured, initiating connection and message exchange...");
    device_manager.deal_with_event_loop(eventloop).await?;
    Ok(())
}

fn create_mqtt_connection()
-> Result<(DeviceManager, EventLoop, Receiver<ChannelMessages>), Box<dyn std::error::Error>> {
    let broker_host = std::env::var("MQTT_BROKER_HOST").unwrap_or("localhost".to_string());
    let broker_port: u16 = std::env::var("MQTT_BROKER_PORT")
        .unwrap_or("8883".to_string())
        .parse()
        .unwrap_or(8883);
    let username = std::env::var("MQTT_USERNAME").unwrap_or("mqtt_user".to_string());
    let password = std::env::var("MQTT_PASSWORD").unwrap_or("password".to_string());
    let disable_tls = matches!(
        std::env::var("DISABLE_TLS")
            .unwrap_or("false".to_string())
            .to_lowercase()
            .as_str(),
        "1" | "true"
    );
    DeviceManager::new(
        &broker_host,
        broker_port,
        "temp_connection_id", // todo: search for a way to identify the connection, which may provide several devices
        &username,
        &password,
        disable_tls,
    )
}
