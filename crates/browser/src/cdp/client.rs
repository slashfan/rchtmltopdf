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
    methods: Option<Vec<String>>,
    /// `None` matches events from any session, and from the browser itself.
    session: Option<SessionId>,
    sender: mpsc::UnboundedSender<Event>,
}

impl Subscriber {
    fn matches(&self, event: &Event) -> bool {
        let method_matches = self
            .methods
            .as_ref()
            .is_none_or(|wanted| wanted.contains(&event.method));
        let session_matches = match &self.session {
            None => true,
            Some(wanted) => event.session_id.as_ref() == Some(wanted),
        };
        method_matches && session_matches
    }
}

/// Why a connection ended, kept so a caller arriving afterwards is told
/// something better than "it closed".
///
/// Not the error itself: several callers may be waiting and [`Error`] cannot be
/// cloned, so enough is kept to rebuild the same error for each of them.
#[derive(Debug, Clone)]
enum Ended {
    /// The browser closed its end. The ordinary case.
    EndOfStream,
    /// The stream could no longer be framed, which means it desynchronised.
    MessageTooLarge { limit: usize },
    /// Reading or writing failed.
    Io(String),
}

impl Ended {
    fn as_error(&self) -> Error {
        match self {
            Ended::EndOfStream => Error::ConnectionClosed,
            Ended::MessageTooLarge { limit } => Error::MessageTooLarge { limit: *limit },
            Ended::Io(detail) => Error::Io(std::io::Error::other(detail.clone())),
        }
    }
}

/// Closes the connection however a task ends: normally, by panic, or by being
/// aborted.
///
/// Putting this in each task is what makes the guarantee unconditional. A close
/// written as the last line of a task body is skipped by both a panic and an
/// abort, and an abort is exactly what dropping the client does, so every caller
/// would wait for ever on a path nobody tests.
struct CloseOnDrop {
    inner: Arc<Inner>,
    reason: Ended,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.inner.close(self.reason.clone());
    }
}

struct Inner {
    next_id: AtomicU64,
    outgoing: mpsc::UnboundedSender<Vec<u8>>,
    /// `None` once the connection has gone.
    ///
    /// The map is the flag. Keeping a separate boolean meant a request could
    /// check it, then insert its channel *after* the map had been drained, and
    /// wait for a reply nobody would ever deliver. Here that is unrepresentable
    /// rather than merely unlikely.
    pending: Mutex<Option<HashMap<u64, oneshot::Sender<Result<Value>>>>>,
    /// Why it ended, once it has.
    ended: Mutex<Option<Ended>>,
    subscribers: Mutex<Vec<Subscriber>>,
    /// A lock-free hint for [`Client::is_closed`]. Never used to decide whether
    /// a request may proceed; the map decides that.
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
                let Some(sender) = self
                    .pending
                    .lock()
                    .unwrap()
                    .as_mut()
                    .and_then(|waiting| waiting.remove(&id))
                else {
                    // Nobody is waiting: the caller gave up, or the connection
                    // has already been torn down.
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

    /// Tear the connection down: every waiting caller is told why, every
    /// subscriber sees the end of its stream.
    ///
    /// Safe to call more than once; the first reason recorded is the one kept,
    /// because it is the one that actually explains what happened.
    fn close(&self, reason: Ended) {
        self.closed.store(true, Ordering::SeqCst);

        {
            let mut ended = self.ended.lock().unwrap();
            if ended.is_none() {
                *ended = Some(reason.clone());
            }
        }

        let waiting = self.pending.lock().unwrap().take();
        if let Some(waiting) = waiting {
            for (_, sender) in waiting {
                let _ = sender.send(Err(reason.as_error()));
            }
        }
        self.subscribers.lock().unwrap().clear();
    }

    /// The error to hand a caller that arrives after the connection has gone.
    fn ended_error(&self) -> Error {
        self.ended
            .lock()
            .unwrap()
            .as_ref()
            .map_or(Error::ConnectionClosed, Ended::as_error)
    }

    async fn request(
        &self,
        session: Option<&SessionId>,
        method: &str,
        params: Value,
    ) -> Result<Value> {
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
        // Registering and finding the connection closed are the same operation,
        // so there is no window between them.
        match self.pending.lock().unwrap().as_mut() {
            Some(waiting) => waiting.insert(id, sender),
            None => return Err(self.ended_error()),
        };

        if self.outgoing.send(frame(&payload)).is_err() {
            if let Some(waiting) = self.pending.lock().unwrap().as_mut() {
                waiting.remove(&id);
            }
            return Err(self.ended_error());
        }

        match receiver.await {
            Ok(outcome) => outcome,
            // The sender was dropped without answering, which only close() does.
            Err(_) => Err(self.ended_error()),
        }
    }

    fn subscribe(&self, session: Option<SessionId>, methods: Option<Vec<String>>) -> Events {
        let (sender, receiver) = mpsc::unbounded_channel();
        self.subscribers.lock().unwrap().push(Subscriber {
            methods,
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
            pending: Mutex::new(Some(HashMap::new())),
            ended: Mutex::new(None),
            subscribers: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        });

        let reader_task = tokio::spawn({
            let inner = Arc::clone(&inner);
            async move {
                // Closes the connection whichever way this task ends, including
                // a panic and the abort that dropping the client performs.
                let mut guard = CloseOnDrop {
                    inner,
                    reason: Ended::EndOfStream,
                };
                let mut framed = Framed::new(reader);

                loop {
                    match framed.next_message().await {
                        Ok(Some(message)) => guard.inner.dispatch(&message),
                        Ok(None) => break,
                        // A stream that can no longer be framed has
                        // desynchronised, and carrying on would mismatch replies
                        // to callers. Report that rather than letting it look
                        // like the browser simply went away.
                        Err(Error::MessageTooLarge { limit }) => {
                            guard.reason = Ended::MessageTooLarge { limit };
                            break;
                        }
                        Err(error) => {
                            guard.reason = Ended::Io(error.to_string());
                            break;
                        }
                    }
                }
            }
        });

        let writer_task = tokio::spawn({
            let inner = Arc::clone(&inner);
            async move {
                let mut guard = CloseOnDrop {
                    inner,
                    reason: Ended::EndOfStream,
                };
                let mut writer = writer;

                while let Some(message) = queue.recv().await {
                    if let Err(error) = writer.write_all(&message).await {
                        guard.reason = Ended::Io(error.to_string());
                        break;
                    }
                    if let Err(error) = writer.flush().await {
                        guard.reason = Ended::Io(error.to_string());
                        break;
                    }
                }

                // Dropping the writer closes the descriptor, which is what tells
                // the browser to exit.
                drop(writer);
            }
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
        self.inner.subscribe(None, Some(vec![method.to_owned()]))
    }

    /// Subscribe to several event methods at once.
    ///
    /// Worth having rather than falling back to every event: a subscription
    /// receives what it asks for and nothing else, so a caller measuring quiet
    /// periods is not woken by unrelated chatter, and the queue does not fill
    /// with events nobody reads.
    pub fn subscribe_many(&self, methods: &[&str]) -> Events {
        self.inner.subscribe(
            None,
            Some(methods.iter().map(|m| (*m).to_owned()).collect()),
        )
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
        self.inner.close(Ended::EndOfStream);
        // Each task holds its own guard, so aborting mid-await still closes.
        self.reader_task.abort();
        self.writer_task.abort();
    }
}

/// A handle addressing one attached target.
#[derive(Clone)]
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
        self.inner
            .subscribe(Some(self.id.clone()), Some(vec![method.to_owned()]))
    }

    /// Subscribe to several event methods from this session.
    pub fn subscribe_many(&self, methods: &[&str]) -> Events {
        self.inner.subscribe(
            Some(self.id.clone()),
            Some(methods.iter().map(|m| (*m).to_owned()).collect()),
        )
    }

    /// Subscribe to every event from this session.
    pub fn subscribe_all(&self) -> Events {
        self.inner.subscribe(Some(self.id.clone()), None)
    }

    /// Send a command without waiting for its reply.
    ///
    /// For shutdown, where there is nothing left to await on and no caller to
    /// report to. Nothing else should use it: a dropped reply is a dropped
    /// error.
    pub fn send_detached(&self, method: &str, params: Value) {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let Ok(payload) = serde_json::to_vec(&Command {
            id,
            method,
            params: if params.is_null() {
                None
            } else {
                Some(&params)
            },
            session_id: Some(&self.id),
        }) else {
            return;
        };
        let _ = self.inner.outgoing.send(frame(&payload));
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
