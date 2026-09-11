//! The protocol client: one connection, many outstanding commands.

use super::framing::{Framed, frame};
use super::message::{Command, Event, Incoming, SessionId};
use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

/// A stream of events matching one subscription.
pub struct Events {
    receiver: mpsc::UnboundedReceiver<Event>,
}

impl Events {
    /// The next matching event, or `None` once the connection has closed.
    pub async fn next(&mut self) -> Option<Event> {
        self.receiver.recv().await
    }
}

struct Subscriber {
    /// `None` matches every method.
    method: Option<String>,
    /// `None` matches events from any session, and from the browser itself.
    session: Option<SessionId>,
    sender: mpsc::UnboundedSender<Event>,
}

impl Subscriber {
    fn matches(&self, event: &Event) -> bool {
        let method_matches = self.method.as_deref().is_none_or(|m| m == event.method);
        let session_matches = match &self.session {
            None => true,
            Some(wanted) => event.session_id.as_ref() == Some(wanted),
        };
        method_matches && session_matches
    }
}

struct Inner {
    next_id: AtomicU64,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>,
    subscribers: Mutex<Vec<Subscriber>>,
    closed: AtomicBool,
}

impl Inner {
    /// Sort one decoded message into a reply or an event.
    ///
    /// A message that will not decode is dropped rather than killing the
    /// connection. It would mean a browser bug, and losing one message is a
    /// better failure than losing the session. A reply whose id nobody is
    /// waiting for is dropped the same way: it means the caller gave up.
    fn dispatch(&self, bytes: &[u8]) {
        let Ok(incoming) = serde_json::from_slice::<Incoming>(bytes) else {
            return;
        };

        match incoming.id {
            Some(id) => {
                let Some(sender) = self.pending.lock().unwrap().remove(&id) else {
                    return;
                };
                let outcome = match incoming.error {
                    Some(wire) => Err(Error::Protocol(wire.into())),
                    None => Ok(incoming.result.unwrap_or(Value::Null)),
                };
                let _ = sender.send(outcome);
            }
            None => {
                let Some(method) = incoming.method else {
                    return;
                };
                self.publish(Event {
                    method,
                    params: incoming.params.unwrap_or(Value::Null),
                    session_id: incoming.session_id,
                });
            }
        }
    }

    fn publish(&self, event: Event) {
        let mut subscribers = self.subscribers.lock().unwrap();
        subscribers.retain(|subscriber| {
            if subscriber.sender.is_closed() {
                return false;
            }
            if subscriber.matches(&event) {
                subscriber.sender.send(event.clone()).is_ok()
            } else {
                true
            }
        });
    }

    /// Tear the connection down: every waiting caller is told, every subscriber
    /// sees the end of its stream.
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let waiting: Vec<_> = self.pending.lock().unwrap().drain().collect();
        for (_, sender) in waiting {
            let _ = sender.send(Err(Error::ConnectionClosed));
        }
        self.subscribers.lock().unwrap().clear();
    }

    async fn request(
        &self,
        session: Option<&SessionId>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Error::ConnectionClosed);
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = serde_json::to_vec(&Command {
            id,
            method,
            params: if params.is_null() {
                None
            } else {
                Some(&params)
            },
            session_id: session,
        })?;

        let (sender, receiver) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, sender);

        if self.outgoing.send(frame(&payload)).is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(Error::ConnectionClosed);
        }

        receiver.await.map_err(|_| Error::ConnectionClosed)?
    }

    fn subscribe(&self, session: Option<SessionId>, method: Option<&str>) -> Events {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.subscribers.lock().unwrap().push(Subscriber {
            method: method.map(str::to_owned),
            session,
            sender,
        });
        Events { receiver }
    }
}

/// A connection to a browser.
///
/// Commands may be in flight concurrently; replies are matched by id, so they
/// can arrive in any order. Dropping the client closes the connection.
pub struct Client {
    inner: Arc<Inner>,
    reader_task: JoinHandle<()>,
    writer_task: JoinHandle<()>,
}

impl Client {
    /// Start talking over an already-open pair of streams.
    ///
    /// Wiring those streams to a real browser's descriptors 3 and 4 is the
    /// launcher's job, deliberately not this module's, so the protocol can be
    /// exercised without spawning anything.
    pub fn new<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let (outgoing, mut queue) = mpsc::unbounded_channel::<Vec<u8>>();
        let inner = Arc::new(Inner {
            next_id: AtomicU64::new(1),
            outgoing,
            pending: Mutex::new(HashMap::new()),
            subscribers: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        });

        let reader_inner = Arc::clone(&inner);
        let reader_task = tokio::spawn(async move {
            let mut framed = Framed::new(reader);
            // Stops on end of stream, and equally on a framing error: a stream we
            // can no longer parse is a stream we can no longer trust, and carrying
            // on would silently mismatch replies to callers.
            while let Ok(Some(message)) = framed.next_message().await {
                reader_inner.dispatch(&message);
            }
            reader_inner.close();
        });

        let writer_inner = Arc::clone(&inner);
        let writer_task = tokio::spawn(async move {
            let mut writer = writer;
            while let Some(message) = queue.recv().await {
                if writer.write_all(&message).await.is_err() || writer.flush().await.is_err() {
                    break;
                }
            }
            // Dropping the writer closes the underlying descriptor, which is what
            // tells the browser to exit.
            drop(writer);
            writer_inner.close();
        });

        Self {
            inner,
            reader_task,
            writer_task,
        }
    }

    /// Send a command to the browser itself and wait for its reply.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.inner.request(None, method, params).await
    }

    /// Subscribe to one event method, from the browser or any session.
    pub fn subscribe(&self, method: &str) -> Events {
        self.inner.subscribe(None, Some(method))
    }

    /// Subscribe to every event, which is mostly useful for diagnostics.
    pub fn subscribe_all(&self) -> Events {
        self.inner.subscribe(None, None)
    }

    /// Attach to a target and get a session to drive it.
    ///
    /// Always attaches flattened, so the session id travels as a top-level field
    /// on later messages. The alternative is the legacy envelope, where each
    /// message is serialised into a string inside another command, and nothing
    /// here supports that.
    pub async fn attach_to_target(&self, target_id: &str) -> Result<Session> {
        let result = self
            .send(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;

        let session_id = result
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Malformed {
                detail: "Target.attachToTarget replied without a sessionId".into(),
            })?;

        Ok(Session {
            inner: Arc::clone(&self.inner),
            id: SessionId::new(session_id),
        })
    }

    /// Build a session handle for an id obtained some other way, such as from a
    /// `Target.attachedToTarget` event.
    pub fn session(&self, id: SessionId) -> Session {
        Session {
            inner: Arc::clone(&self.inner),
            id,
        }
    }

    /// Whether the connection has gone.
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }
}

impl Drop for Client {
    /// An abrupt close.
    ///
    /// Queued messages that have not reached the pipe are lost. To shut a
    /// browser down gracefully, send the closing command and await its reply
    /// before dropping this.
    fn drop(&mut self) {
        self.inner.close();
        self.reader_task.abort();
        self.writer_task.abort();
    }
}

/// A handle addressing one attached target.
pub struct Session {
    inner: Arc<Inner>,
    id: SessionId,
}

impl Session {
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Send a command to this session and wait for its reply.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value> {
        self.inner.request(Some(&self.id), method, params).await
    }

    /// Subscribe to one event method, from this session only.
    pub fn subscribe(&self, method: &str) -> Events {
        self.inner.subscribe(Some(self.id.clone()), Some(method))
    }

    /// Subscribe to every event from this session.
    pub fn subscribe_all(&self) -> Events {
        self.inner.subscribe(Some(self.id.clone()), None)
    }
}

// Debug by hand: the shared state holds channels and in-flight callers, none of
// which are useful or printable. What a reader wants is the identity and whether
// the connection is still up.

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("closed", &self.inner.closed.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Events {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Events").finish_non_exhaustive()
    }
}
