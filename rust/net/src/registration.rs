//
// Copyright 2025 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

mod error;
pub use error::*;

mod request;
pub use request::*;

mod service;
pub use service::*;

mod session_id;
pub use session_id::*;

#[cfg(test)]
mod testutil {
    use std::convert::Infallible;
    use std::future::Future;
    use std::marker::PhantomData;

    use futures_util::future::BoxFuture;
    use futures_util::FutureExt as _;
    use tokio::sync::{mpsc, oneshot};

    use crate::chat::fake::FakeChatRemote;
    use crate::chat::ws2::ListenerEvent;
    use crate::chat::{ChatConnection, ConnectError as ChatConnectError};
    use crate::registration::ConnectChat;

    /// Fake [`ConnectChat`] impl that writes the remote end to a channel.
    pub(super) struct FakeChatConnect {
        pub(super) remote: mpsc::UnboundedSender<FakeChatRemote>,
    }

    pub(super) struct DropOnDisconnect<T>(Option<T>);

    impl<T> DropOnDisconnect<T> {
        pub(super) fn new(value: T) -> Self {
            Self(Some(value))
        }

        pub(super) fn into_listener(mut self) -> crate::chat::ws2::EventListener
        where
            T: Send + 'static,
        {
            Box::new(move |event| match event {
                ListenerEvent::ReceivedAlerts(alerts) => {
                    if !alerts.is_empty() {
                        unreachable!("unexpected alerts: {alerts:?}")
                    }
                }
                ListenerEvent::ReceivedMessage(_, _) => unreachable!("no incoming messages"),
                ListenerEvent::Finished(_reason) => drop(self.0.take()),
            })
        }
    }

    impl ConnectChat for FakeChatConnect {
        fn connect_chat(
            &self,
            on_disconnect: oneshot::Sender<Infallible>,
        ) -> BoxFuture<'_, Result<ChatConnection, ChatConnectError>> {
            let (fake_chat, fake_remote) = ChatConnection::new_fake(
                tokio::runtime::Handle::current(),
                DropOnDisconnect::new(on_disconnect).into_listener(),
                [],
            );
            async {
                let _ignore_failure = self.remote.send(fake_remote);
                Ok(fake_chat)
            }
            .boxed()
        }
    }

    /// [`ConnectChat`] impl that wraps a [`Fn`].
    pub(super) struct ConnectChatFn<'a, F>(F, PhantomData<&'a ()>);

    impl<F> ConnectChatFn<'_, F> {
        pub(super) fn new(f: F) -> Self {
            Self(f, PhantomData)
        }
    }

    impl<'a, F, Fut> ConnectChat for ConnectChatFn<'a, F>
    where
        F: Fn(oneshot::Sender<Infallible>) -> Fut + Send,
        Fut: Future<Output = Result<ChatConnection, ChatConnectError>> + Send + 'a,
    {
        fn connect_chat(
            &self,
            on_disconnect: oneshot::Sender<Infallible>,
        ) -> BoxFuture<'_, Result<ChatConnection, ChatConnectError>> {
            self.0(on_disconnect).boxed()
        }
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr as _;

    use tokio::sync::mpsc;

    use super::*;
    use crate::proto::chat_websocket::WebSocketRequestMessage;
    use crate::registration::testutil::FakeChatConnect;

    #[tokio::test]
    async fn create_session() {
        let (fake_chat_remote_tx, mut fake_chat_remote_rx) = mpsc::unbounded_channel();
        let fake_connect = FakeChatConnect {
            remote: fake_chat_remote_tx,
        };

        let create_session = RegistrationService::create_session(
            CreateSession {
                number: "+18005550101".to_owned(),
                ..Default::default()
            },
            Box::new(fake_connect),
        );

        const SESSION_ID: &str = "sessionId";
        let make_session = || RegistrationSession {
            allowed_to_request_code: true,
            verified: false,
            ..Default::default()
        };

        tokio::spawn(async move {
            let fake_chat_remote = fake_chat_remote_rx.recv().await.expect("started connect");

            let incoming_request = fake_chat_remote
                .receive_request()
                .await
                .expect("still receiving")
                .expect("received request");

            assert_eq!(
                incoming_request,
                WebSocketRequestMessage {
                    verb: Some("POST".to_string()),
                    path: Some("/v1/verification/session".to_string()),
                    body: Some(b"{\"number\":\"+18005550101\"}".into()),
                    headers: vec!["content-type: application/json".to_string()],
                    id: Some(0),
                }
            );

            fake_chat_remote
                .send_response(
                    RegistrationResponse {
                        session_id: SESSION_ID.to_owned(),
                        session: make_session(),
                    }
                    .into_websocket_response(incoming_request.id()),
                )
                .expect("sent");
        });

        let service = create_session.await.expect("can create session");

        assert_eq!(**service.session_id(), SESSION_ID);
        assert_eq!(service.session_state(), &make_session())
    }

    #[tokio::test]
    async fn resume_session() {
        let (fake_chat_remote_tx, mut fake_chat_remote_rx) = mpsc::unbounded_channel();
        let fake_connect = FakeChatConnect {
            remote: fake_chat_remote_tx,
        };
        const SESSION_ID: &str = "abcabc";

        let resume_session = RegistrationService::resume_session(
            SessionId::from_str(SESSION_ID).unwrap(),
            Box::new(fake_connect),
        );

        tokio::spawn(async move {
            let fake_chat_remote = fake_chat_remote_rx.recv().await.expect("sender not closed");
            let incoming_request = fake_chat_remote
                .receive_request()
                .await
                .expect("still receiving")
                .expect("received request");

            assert_eq!(
                incoming_request,
                WebSocketRequestMessage {
                    verb: Some("GET".to_string()),
                    path: Some("/v1/verification/session/abcabc".to_string()),
                    body: None,
                    headers: vec![],
                    id: Some(0),
                }
            );

            fake_chat_remote
                .send_response(
                    RegistrationResponse {
                        session_id: SESSION_ID.to_owned(),
                        session: RegistrationSession {
                            allowed_to_request_code: true,
                            verified: false,
                            ..Default::default()
                        },
                    }
                    .into_websocket_response(0),
                )
                .expect("not disconnected");
            fake_chat_remote
        });

        let session_client = resume_session.await;

        // At this point the client should be connected and can make additional
        // requests.
        let session_client = session_client.expect("resumed session");
        assert_eq!(
            session_client.session_id(),
            &SessionId::from_str(SESSION_ID).unwrap()
        );
    }
}
