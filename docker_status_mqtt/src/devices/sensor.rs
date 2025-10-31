use async_trait::async_trait;
use docker_status_mqtt_proc_macros::*;
use serde_json::{Map, Value, json};
use std::fmt::Debug;

use crate::{
    cancellation_token::CancellationToken,
    devices::{EntityDetails, EntityDetailsGetter, EntityType, Error, device::Device},
};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(EntityDetailsGetter, Debug)]
pub struct Sensor {
    pub details: EntityDetails,
    pub unit_of_measurement: Option<String>,
    pub device_class: Option<String>,
}

impl Sensor {
    pub fn new(
        device_identifier: impl Into<String>,
        name: impl Into<String>,
        icon: impl Into<String>,
        unit_of_measurement: impl Into<String>,
        device_class: impl Into<String>,
    ) -> Self {
        Self::new_with_details(
            EntityDetails::new(device_identifier, name, icon),
            Some(unit_of_measurement.into()),
            Some(device_class.into()),
        )
    }
    pub fn new_simple(device_identifier: impl Into<String>, name: impl Into<String>, icon: impl Into<String>) -> Self {
        Self::new_with_details(EntityDetails::new(device_identifier, name, icon), None, None)
    }
    pub fn new_with_details(
        details: EntityDetails,
        unit_of_measurement: Option<String>,
        device_class: Option<String>,
    ) -> Self {
        Sensor {
            details,
            unit_of_measurement,
            device_class,
        }
    }
}

#[async_trait]
impl EntityType for Sensor {
    async fn json_for_discovery<'a>(
        &'a self,
        device: &'a Device,
        _cancellation_token: CancellationToken,
    ) -> Result<serde_json::Value> {
        let json = json!({
            "device_class": self.device_class,
            "platform": "sensor",
            "unit_of_measurement": self.unit_of_measurement,
        });
        let mut entity_details_json = self.details.json_for_discovery(device).await?;
        if let (Value::Object(entity_details_map), Value::Object(sensor_map)) = (&mut entity_details_json, json) {
            entity_details_map.extend(sensor_map);
        } else {
            return Err(Error::IncorrectJsonStructure);
        }
        let mut map: Map<String, Value> = Map::new();
        map.insert(self.details.id.clone(), entity_details_json);
        Ok(Value::Object(map))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::test_module::test_helpers::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[tokio::test]
    async fn test_sensor_json_for_discovery() {
        let device = create_test_device();
        let sensor = Sensor::new("test_device", "Temperature", "mdi:thermometer", "°C", "temperature");
        let json = sensor
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "temperature": {
                "device_class": "temperature",
                "icon": "mdi:thermometer",
                "name": "Temperature",
                "platform": "sensor",
                "state_topic": "test_device/temperature/state",
                "unique_id": "test_device_temperature",
                "unit_of_measurement": "°C"
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_sensor_json_for_discovery_without_device_class_and_unit() {
        let device = create_test_device_with_identifier("test_device");
        let sensor = Sensor::new_simple(
            "test_device".to_string(),
            "Generic Sensor".to_string(),
            "mdi:sensor".to_string(),
        );
        let json = sensor
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "generic_sensor": {
                "icon": "mdi:sensor",
                "name": "Generic Sensor",
                "platform": "sensor",
                "state_topic": "test_device/generic_sensor/state",
                "unique_id": "test_device_generic_sensor",
                "device_class": null,
                "unit_of_measurement": null
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }

    #[tokio::test]
    async fn test_sensor_json_for_discovery_with_attributes() {
        let device = create_test_device_with_identifier("test_device");
        let details = EntityDetails::new(
            "test_device".to_string(),
            "Sensor with Attributes".to_string(),
            "mdi:sensor".to_string(),
        )
        .has_attributes();
        let sensor = Sensor::new_with_details(
            details,
            Some("units".to_string()),
            Some("some_device_class".to_string()),
        );
        let json = sensor
            .json_for_discovery(&device, CancellationToken::default())
            .await
            .unwrap();
        let expected_json = json!({
            "sensor_with_attributes": {
                "device_class": null,
                "icon": "mdi:sensor",
                "json_attributes_template": "{{ value_json | tojson }}",
                "json_attributes_topic": "test_device/sensor_with_attributes_attributes/state",
                "name": "Sensor with Attributes",
                "platform": "sensor",
                "state_topic": "test_device/sensor_with_attributes/state",
                "unique_id": "test_device_sensor_with_attributes",
                "unit_of_measurement": "units",
                "device_class": "some_device_class"
            }
        });
        assert_eq!(
            serde_json::to_string_pretty(&json).unwrap(),
            serde_json::to_string_pretty(&expected_json).unwrap()
        );
    }
}
