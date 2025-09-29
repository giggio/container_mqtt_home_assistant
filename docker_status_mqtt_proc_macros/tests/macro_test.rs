#[cfg(test)]
mod tests {
    use docker_status_mqtt_proc_macros::*;

    pub struct ComponentDetails {
        pub id: String,
    }

    pub trait ComponentDetailsGetter {
        fn details(&self) -> &ComponentDetails;
    }

    #[derive(ComponentDetailsGetter)]
    pub struct Switch {
        pub details: ComponentDetails,
    }

    #[test]
    fn it_works() {
        let switch = Switch {
            details: ComponentDetails {
                id: "switch_1".to_string(),
            },
        };
        assert_eq!(switch.details().id, "switch_1".to_string());
    }
}
