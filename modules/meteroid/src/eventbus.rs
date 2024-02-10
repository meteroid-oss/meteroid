use std::fmt::Debug;
use std::sync::Arc;

#[derive(thiserror::Error, Debug)]
pub enum EventBusError {
    #[error("Failed to publish event")]
    PublishFailed,
    //#[error("Failed to handle event")]
    //EventHandlerFailed(Box<dyn std::error::Error>),
}

/**
 * EventBus is a simple event bus implementation.
 * It allows to have one publisher many subscribers.
 * NOTE:
 *   It doesn't use persistent storage, the publisher is decoupled from the subscribers
 *   so if the process dies, all in-flight events are lost.
 */
pub struct EventBus<E> {
    pub sender: tokio::sync::broadcast::Sender<E>,
}

#[async_trait::async_trait]
pub trait EventHandler<E>: Send + Sync {
    async fn handle(&self, event: E) -> Result<(), EventBusError>;
}

impl<E: Debug + Clone + Send + 'static> EventBus<E> {
    pub fn new() -> Self {
        let (snd, _) = tokio::sync::broadcast::channel(1000);

        EventBus { sender: snd }
    }

    pub fn subscribe(&self, handler: Arc<dyn EventHandler<E>>) {
        let mut rx = self.sender.subscribe();
        tokio::spawn(async move {
            loop {
                let handler = handler.clone();
                match rx.recv().await {
                    Ok(event) => {
                        tokio::spawn(async move {
                            match handler.handle(event).await {
                                Ok(_) => {}
                                Err(e) => {
                                    log::error!("Error handling event. Ignoring it. {:?}", e)
                                }
                            };
                        });
                    }
                    Err(e) => {
                        log::error!(
                            "Error receiving event from broadcast channel. Ignoring it. {:?}",
                            e
                        );
                    }
                };
            }
        });
    }

    pub fn publish(&self, event: E) -> Result<usize, EventBusError> {
        self.sender
            .send(event)
            .map_err(|_| EventBusError::PublishFailed)
    }
}

#[cfg(test)]
mod tests {
    use crate::eventbus;
    use crate::eventbus::{EventBus, EventHandler};
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct CapturingEventHandler {
        items: Arc<Mutex<Vec<u8>>>,
    }

    impl CapturingEventHandler {
        pub fn new() -> Self {
            CapturingEventHandler {
                items: Arc::new(Mutex::new(vec![])),
            }
        }

        pub fn items(&self) -> HashSet<u8> {
            self.items.lock().unwrap().clone().into_iter().collect()
        }
    }

    #[async_trait::async_trait]
    impl EventHandler<u8> for CapturingEventHandler {
        async fn handle(&self, event: u8) -> Result<(), eventbus::EventBusError> {
            let mut guard = self.items.lock().unwrap();
            guard.push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn eventbus_test() {
        let bus = EventBus::new();

        let handler1 = CapturingEventHandler::new();
        let handler2 = CapturingEventHandler::new();

        bus.subscribe(Arc::new(handler1.clone()));
        bus.subscribe(Arc::new(handler2.clone()));

        bus.publish(1).unwrap();
        bus.publish(2).unwrap();
        bus.publish(3).unwrap();
        bus.publish(4).unwrap();
        bus.publish(5).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let expected: HashSet<u8> = [1, 2, 3, 4, 5].iter().cloned().collect();

        assert_eq!(handler1.clone().items(), expected);
        assert_eq!(handler2.clone().items(), expected);
    }
}
