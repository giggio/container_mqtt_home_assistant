// use std::sync::{Arc, Mutex, Weak};
//
// pub trait Event: Send + Sync {}
//
// pub trait Subscriber<E>: Send + Sync
// where
//     E: Event,
// {
//     fn notify(&mut self, event: &E);
// }
//
// pub struct Publisher<E>
// where
//     E: Event,
// {
//     weak_subscribers: Vec<Weak<Mutex<dyn Subscriber<E>>>>,
// }
//
// impl<E> Publisher<E>
// where
//     E: Event,
// {
//     pub fn new() -> Publisher<E> {
//         Publisher {
//             weak_subscribers: Vec::new(),
//         }
//     }
//
//     pub fn register_subscriber(&mut self, subscriber: Arc<Mutex<dyn Subscriber<E>>>) {
//         self.weak_subscribers.push(Arc::downgrade(&subscriber));
//     }
//
//     #[cfg(test)]
//     pub fn count_of_subscribers(&self) -> usize {
//         self.weak_subscribers.len()
//     }
//
//     pub fn publish(&mut self, event: E) -> Result<(), ObserverError> {
//         trace!("Publishing event to {} subscribers", self.weak_subscribers.len());
//         let mut cleanup = false;
//         for subscriber in self.weak_subscribers.iter() {
//             if let Some(subscriber_rc) = subscriber.upgrade() {
//                 let mut subscriber = subscriber_rc
//                     .lock()
//                     .map_err(|_| ObserverError::PoisonedMutex("Subscriber mutex is poisoned.".to_owned()))?;
//                 subscriber.notify(&event);
//             } else {
//                 cleanup = true;
//             }
//         }
//         if cleanup {
//             self.cleanup_dead_subscribers();
//         }
//         Ok(())
//     }
//
//     fn cleanup_dead_subscribers(&mut self) {
//         trace!("Cleaning up dead subscribers");
//         self.weak_subscribers
//             .retain(|subscriber| subscriber.upgrade().is_some());
//     }
//
//     #[allow(dead_code)]
//     pub fn clear(&mut self) {
//         self.weak_subscribers.clear();
//     }
// }
//
// #[derive(thiserror::Error, Debug)]
// pub enum ObserverError {
//     #[error("Subscriber mutex is poisoned: {0}")]
//     PoisonedMutex(String),
//     #[allow(dead_code)]
//     #[error("Unknown device error")]
//     Unknown,
// }
//
// #[cfg(test)]
// mod tests {
//     use super::*;
//
//     pub enum MyEvents {
//         Message(String),
//         VerboseMessage(String),
//     }
//
//     impl Event for MyEvents {}
//
//     struct MySubscriber {
//         pub message: Option<String>,
//         pub verbose_message: Option<String>,
//     }
//     impl Subscriber<MyEvents> for MySubscriber {
//         fn notify(&mut self, _event: &MyEvents) {
//             match _event {
//                 MyEvents::Message(m) => {
//                     self.message = Some(m.to_owned());
//                 }
//                 MyEvents::VerboseMessage(m) => {
//                     self.verbose_message = Some(m.to_owned());
//                 }
//             }
//         }
//     }
//
//     #[test]
//     fn events_one_subscriber() {
//         let mut d: Publisher<MyEvents> = Publisher::new();
//         let subscriber_rc = Arc::new(Mutex::new(MySubscriber {
//             message: None,
//             verbose_message: None,
//         }));
//         d.register_subscriber(subscriber_rc.clone());
//         d.publish(MyEvents::Message("hello".to_owned())).unwrap();
//
//         assert_eq!(
//             subscriber_rc.lock().unwrap().message,
//             Some("hello".to_owned())
//         );
//         assert_eq!(subscriber_rc.lock().unwrap().verbose_message, None);
//         assert_eq!(d.count_of_subscribers(), 1);
//         d.publish(MyEvents::VerboseMessage("verbose hello".to_owned())).unwrap();
//         assert_eq!(
//             subscriber_rc.lock().unwrap().verbose_message,
//             Some("verbose hello".to_owned())
//         );
//         assert_eq!(d.count_of_subscribers(), 1);
//     }
//
//     #[test]
//     fn no_subscribers() {
//         let mut d: Publisher<MyEvents> = Publisher::new();
//
//         {
//             let subscriber_rc = Arc::new(Mutex::new(MySubscriber {
//                 message: None,
//                 verbose_message: None,
//             }));
//             d.register_subscriber(subscriber_rc.clone());
//         }
//         d.publish(MyEvents::VerboseMessage("hello".to_owned()))
//             .unwrap();
//         assert_eq!(d.count_of_subscribers(), 0);
//     }
// }
