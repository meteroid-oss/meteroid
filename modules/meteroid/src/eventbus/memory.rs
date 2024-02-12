use crate::eventbus::{EventBus, EventBusError, EventHandler};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

/**
 * Simple in-memory event bus implementation based on tokio::sync::broadcast.
 * It allows to have one publisher and many subscribers.
 * NOTE:
 *   As it doesn't use persistent storage and the publisher is decoupled from the subscribers,
 *   if the process dies then all in-flight events are lost.
 */
pub struct InMemory<E> {
    pub sender: tokio::sync::broadcast::Sender<E>,
}

#[async_trait::async_trait]
impl<E: Debug + Clone + Send + 'static> EventBus<E> for InMemory<E> {
    async fn subscribe(&self, handler: Arc<dyn EventHandler<E>>) {
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
                    Err(e) => match e {
                        RecvError::Lagged(lagged) => {
                            log::warn!("Receiver lagged by {}", lagged)
                        }
                        RecvError::Closed => {
                            log::info!("Broadcast channel closed. Stopping event handler");
                            break;
                        }
                    },
                };
            }
        });
    }

    async fn publish(&self, event: E) -> Result<(), EventBusError> {
        self.sender
            .send(event)
            .map(|_| ())
            .map_err(|_| EventBusError::PublishFailed)
    }
}

impl<E: Debug + Clone + Send + 'static> InMemory<E> {
    pub fn new() -> Self {
        let (snd, _) = tokio::sync::broadcast::channel(1000);

        InMemory { sender: snd }
    }
}

#[cfg(test)]
mod tests {
    use crate::eventbus;
    use crate::eventbus::memory::InMemory;
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
        let bus = InMemory::new();

        let handler1 = CapturingEventHandler::new();
        let handler2 = CapturingEventHandler::new();

        bus.subscribe(Arc::new(handler1.clone())).await;
        bus.subscribe(Arc::new(handler2.clone())).await;

        bus.publish(1).await.unwrap();
        bus.publish(2).await.unwrap();
        bus.publish(3).await.unwrap();
        bus.publish(4).await.unwrap();
        bus.publish(5).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let expected: HashSet<u8> = [1, 2, 3, 4, 5].iter().cloned().collect();

        assert_eq!(handler1.clone().items(), expected);
        assert_eq!(handler2.clone().items(), expected);
    }
}
