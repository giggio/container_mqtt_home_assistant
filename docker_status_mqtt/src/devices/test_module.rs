#[cfg(test)]
pub mod test_helpers {
    use std::future;

    use serde_json::json;

    use crate::{
        cancellation_token::CancellationToken,
        devices::{
            Devices, EntityDetails, MockAnEntityType, MockEntity,
            device::{Device, DeviceDetails, DeviceOrigin},
        },
    };

    pub fn create_device_details() -> DeviceDetails {
        DeviceDetails {
            name: "Test Device".to_string(),
            identifier: "test_device".to_string(),
            manufacturer: "Test Mfg".to_string(),
            sw_version: "1.0.0".to_string(),
            via_device: None,
        }
    }

    pub fn create_device_origin() -> DeviceOrigin {
        DeviceOrigin {
            name: "test-origin".to_string(),
            sw: "1.0.0".to_string(),
            url: "http://example.com".to_string(),
        }
    }

    pub fn create_test_device() -> Device {
        Device::new(
            create_device_details(),
            create_device_origin(),
            "dev1/availability".to_string(),
            "device_manager_1".to_string(),
            CancellationToken::default(),
        )
    }

    pub fn create_test_device_with_identifier(identifier: &str) -> Device {
        Device::new(
            DeviceDetails {
                name: "Test Device".to_string(),
                identifier: identifier.to_string(),
                manufacturer: "Test Mfg".to_string(),
                sw_version: "1.0.0".to_string(),
                via_device: None,
            },
            create_device_origin(),
            "dev1/availability".to_string(),
            "device_manager_1".to_string(),
            CancellationToken::default(),
        )
    }

    pub fn get_mock_entity() -> MockEntity {
        let mut entity_type = MockAnEntityType::new();
        entity_type
            .expect_json_for_discovery()
            .returning(|_, _| Ok(json!({ "test": true })));
        entity_type.expect_details().return_const(
            EntityDetails::new("dev1".to_string(), "Test Name".to_string(), "testicon".to_string())
                .add_command("dev1/test_name/command".to_string()),
        );
        let mut mock_entity = MockEntity::new();
        mock_entity
            .expect_get_data()
            .return_const(Box::new(entity_type))
            .times(..);
        mock_entity
            .expect_get_entity_data()
            .returning(|_| {
                Box::pin(future::ready(Ok(
                    hashmap! {"dev1/test_name/state".to_string() => "test_state".to_string()},
                )))
            })
            .times(..);
        mock_entity
    }

    pub fn make_empty_device() -> Device {
        Device::new(
            DeviceDetails {
                name: "Test Device".to_string(),
                identifier: "test_device".to_string(),
                manufacturer: "Mfg".to_string(),
                sw_version: "1.0.0".to_string(),
                via_device: None,
            },
            DeviceOrigin {
                name: "origin".to_string(),
                sw: "1.0.0".to_string(),
                url: "http://example".to_string(),
            },
            "dev1/availability".to_string(),
            "device_manager_1".to_string(),
            CancellationToken::default(),
        )
    }

    pub fn make_empty_devices() -> Devices {
        Devices::new_from_single_device(make_empty_device(), CancellationToken::default())
    }
}
