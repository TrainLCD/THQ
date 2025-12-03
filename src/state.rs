use std::{collections::HashMap, collections::VecDeque, sync::Arc};

use axum::extract::ws::Message;
use tokio::sync::{mpsc, RwLock};
use uuid::Uuid;

pub type ClientTx = mpsc::Sender<Message>;

#[derive(Clone)]
pub struct TelemetryHub {
    subscribers: Arc<RwLock<HashMap<Uuid, ClientTx>>>,
    buffer: Arc<RwLock<VecDeque<String>>>,
    capacity: usize,
}

impl TelemetryHub {
    pub fn new(capacity: usize) -> Self {
        Self {
            subscribers: Arc::new(RwLock::new(HashMap::new())),
            buffer: Arc::new(RwLock::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    pub async fn add_subscriber(&self, id: Uuid, tx: ClientTx) {
        let mut subs = self.subscribers.write().await;
        subs.insert(id, tx);
    }

    pub async fn remove_subscriber(&self, id: &Uuid) {
        let mut subs = self.subscribers.write().await;
        subs.remove(id);
    }

    pub async fn snapshot(&self) -> Vec<String> {
        let buf = self.buffer.read().await;
        buf.iter().cloned().collect()
    }

    pub async fn broadcast(&self, msg: String) {
        {
            let mut buf = self.buffer.write().await;
            if buf.len() >= self.capacity {
                buf.pop_front();
            }
            buf.push_back(msg.clone());
        }

        let targets = {
            let subs = self.subscribers.read().await;
            subs.iter()
                .map(|(id, tx)| (*id, tx.clone()))
                .collect::<Vec<_>>()
        };

        let mut stale = Vec::new();
        for (id, tx) in targets {
            if tx.is_closed() {
                stale.push(id);
                continue;
            }
            if let Err(err) = tx.try_send(Message::Text(msg.clone())) {
                tracing::warn!(?id, "broadcast send failed: {err}");
            }
        }

        if !stale.is_empty() {
            let mut subs = self.subscribers.write().await;
            for id in stale {
                subs.remove(&id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn broadcast_reaches_active_subscriber() {
        let hub = TelemetryHub::new(10);
        let (tx, mut rx) = mpsc::channel(4);
        let client_id = Uuid::new_v4();
        hub.add_subscriber(client_id, tx).await;

        hub.broadcast("hello".to_string()).await;

        let msg = rx.recv().await.expect("message should arrive");
        match msg {
            Message::Text(t) => assert_eq!(t, "hello"),
            _ => panic!("expected text message"),
        }
    }

    #[tokio::test]
    async fn ring_buffer_drops_oldest_when_full() {
        let hub = TelemetryHub::new(2);
        hub.broadcast("one".to_string()).await;
        hub.broadcast("two".to_string()).await;
        hub.broadcast("three".to_string()).await;

        let snapshot = hub.snapshot().await;
        assert_eq!(snapshot, vec!["two".to_string(), "three".to_string()]);
    }
}
