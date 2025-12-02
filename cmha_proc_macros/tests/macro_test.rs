#[cfg(test)]
mod tests {
    use cmha_proc_macros::*;

    pub struct EntityDetails {
        pub id: String,
    }

    pub trait EntityDetailsGetter {
        fn details(&self) -> &EntityDetails;
    }

    #[derive(EntityDetailsGetter)]
    pub struct Switch {
        pub details: EntityDetails,
    }

    #[test]
    fn it_works() {
        let switch = Switch {
            details: EntityDetails {
                id: "switch_1".to_string(),
            },
        };
        assert_eq!(switch.details().id, "switch_1".to_string());
    }
}
