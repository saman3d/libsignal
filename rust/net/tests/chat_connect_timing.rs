//
// Copyright 2024 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

use std::collections::HashMap;

use assert_matches::assert_matches;
use async_trait::async_trait;
use futures_util::{StreamExt as _, TryFutureExt as _};
use itertools::Itertools as _;
use libsignal_net::chat;
use libsignal_net::env::STAGING;
use libsignal_net::infra::errors::TransportConnectError;
use libsignal_net_infra::dns::dns_lookup::{DnsLookup, DnsLookupRequest};
use libsignal_net_infra::dns::lookup_result::LookupResult;
use libsignal_net_infra::dns::{self, DnsResolver};
use libsignal_net_infra::host::Host;
use libsignal_net_infra::utils::timed;
use test_case::test_case;
use tokio::time::{Duration, Instant};

mod fake_transport;
use fake_transport::{
    allow_domain_fronting, connect_websockets_on_incoming, error_all_hosts_after, FakeDeps,
};

use crate::fake_transport::{
    allow_all_routes, Behavior, FakeTransportTarget, TransportConnectEvent,
    TransportConnectEventStage,
};

#[test_case(Duration::from_secs(60))]
#[test_log::test(tokio::test(start_paused = true))]
async fn all_routes_connect_hangs_forever(expected_duration: Duration) {
    let (deps, _incoming_streams) = FakeDeps::new(&STAGING.chat_domain_config);

    let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;

    assert_eq!(elapsed, expected_duration);
    assert_matches!(outcome, Err(chat::ConnectError::Timeout));
}

#[test_case(Duration::from_millis(500))]
#[test_log::test(tokio::test(start_paused = true))]
async fn only_proxies_are_reachable(expected_duration: Duration) {
    let (deps, incoming_streams) = FakeDeps::new(&STAGING.chat_domain_config);
    deps.transport_connector
        .set_behaviors(allow_domain_fronting(
            &STAGING.chat_domain_config,
            deps.static_ip_map(),
        ));

    tokio::spawn(connect_websockets_on_incoming(incoming_streams));

    let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;

    assert_matches!(outcome, Ok(_));
    assert_eq!(elapsed, expected_duration);
}

#[test_case(Duration::from_millis(500))]
#[test_log::test(tokio::test(start_paused = true))]
async fn direct_connect_fails_after_30s_but_proxies_reachable(expected_duration: Duration) {
    let (deps, incoming_streams) = FakeDeps::new(&STAGING.chat_domain_config);
    deps.transport_connector.set_behaviors(
        error_all_hosts_after(
            &STAGING.chat_domain_config,
            deps.static_ip_map(),
            Duration::from_secs(30),
            || TransportConnectError::TcpConnectionFailed,
        )
        .chain(allow_domain_fronting(
            &STAGING.chat_domain_config,
            deps.static_ip_map(),
        )),
    );
    tokio::spawn(connect_websockets_on_incoming(incoming_streams));

    let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;

    assert_eq!(elapsed, expected_duration);
    assert_matches!(outcome, Ok(_));
}

#[test_case(Duration::from_secs(60))]
#[test_log::test(tokio::test(start_paused = true))]

async fn transport_connects_but_websocket_never_responds(expected_duration: Duration) {
    let chat_domain_config = STAGING.chat_domain_config;
    let (deps, incoming_streams) = FakeDeps::new(&chat_domain_config);
    deps.transport_connector
        .set_behaviors(allow_all_routes(&chat_domain_config, deps.static_ip_map()));

    let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;

    // Now that the connect attempt is done, collect (and close) the incoming streams.
    // (If we did this concurrently, the connection logic would move on to the next route.)
    // Note that we have to guarantee there won't be any more connection attempts for collect()!
    drop(deps);
    let incoming_stream_hosts: Vec<_> =
        incoming_streams.map(|(host, _stream)| host).collect().await;

    assert_eq!(elapsed, expected_duration);
    assert_matches!(outcome, Err(chat::ConnectError::Timeout));

    assert_eq!(
        &incoming_stream_hosts,
        &[Host::Domain(chat_domain_config.connect.hostname.into())],
        "should only have one websocket connection"
    );
}

#[test_case(Duration::from_millis(500), Duration::from_millis(500))]
#[test_log::test(tokio::test(start_paused = true))]
async fn connect_again_skips_timed_out_routes(
    expected_first_duration: Duration,
    expected_second_duration: Duration,
) {
    let (deps, incoming_streams) = FakeDeps::new(&STAGING.chat_domain_config);

    // For this test, only the proxy targets are reachable. The connection
    // manager should "learn" from the first attempt, after which a later
    // attempt will skip those routes and connect quickly.
    deps.transport_connector
        .set_behaviors(allow_domain_fronting(
            &STAGING.chat_domain_config,
            deps.static_ip_map(),
        ));
    tokio::spawn(connect_websockets_on_incoming(incoming_streams));

    {
        let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;
        assert_matches!(outcome, Ok(_));
        assert_eq!(elapsed, expected_first_duration);
    }
    {
        let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;
        assert_matches!(outcome, Ok(_));
        assert_eq!(elapsed, expected_second_duration);
    }
}

#[test_log::test(tokio::test(start_paused = true))]
async fn runs_one_tls_handshake_at_a_time() {
    let domain_config = STAGING.chat_domain_config;
    let (deps, incoming_streams) = FakeDeps::new(&domain_config);

    const TLS_HANDSHAKE_DELAY: Duration = Duration::from_secs(5);
    tokio::spawn(connect_websockets_on_incoming(incoming_streams));
    deps.transport_connector.set_behaviors(
        allow_all_routes(&domain_config, deps.static_ip_map()).map(|(target, behavior)| {
            // Pretend that TLS handshakes take a long time to complete.
            let new_behavior = match &target {
                FakeTransportTarget::Tls { .. } => Behavior::Delay {
                    delay: TLS_HANDSHAKE_DELAY,
                    then: behavior.into(),
                },
                FakeTransportTarget::TcpThroughProxy { .. } | FakeTransportTarget::Tcp { .. } => {
                    behavior
                }
            };
            (target, new_behavior)
        }),
    );

    let start = Instant::now();
    let (timing, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;
    assert_matches!(outcome, Ok(_));

    let events = deps
        .transport_connector
        .recorded_events
        .lock()
        .unwrap()
        .drain(..)
        .map(|(event, when)| (event, when.duration_since(start)))
        .collect_vec();

    const FIRST_DELAY: Duration = Duration::from_millis(500);
    const SECOND_DELAY: Duration = Duration::from_millis(1500);

    use TransportConnectEvent::*;
    use TransportConnectEventStage::*;
    assert_matches!(
        &*events,
        [
            // There are 3 successful TCP connections made but only one TLS
            // handshake is attempted. The other connections are abandoned when
            // the first TLS handshake completes, so we never see any TLS
            // handshake events for them.
            ((TcpConnect(_), Start), Duration::ZERO),
            ((TcpConnect(_), End), Duration::ZERO),
            ((TlsHandshake(Host::Domain(first_sni)), Start), Duration::ZERO),
            ((TcpConnect(_), Start), FIRST_DELAY),
            ((TcpConnect(_), End), FIRST_DELAY),
            ((TcpConnect(_), Start), SECOND_DELAY),
            ((TcpConnect(_), End), SECOND_DELAY),
            ((TlsHandshake(_), End), TLS_HANDSHAKE_DELAY),
        ] => assert_eq!(&**first_sni, STAGING.chat_domain_config.connect.hostname)
    );
    assert_eq!(timing, Duration::from_secs(5));
}

#[test_log::test(tokio::test(start_paused = true))]
async fn tcp_connects_but_tls_never_responds() {
    let domain_config = STAGING.chat_domain_config;
    let (deps, incoming_streams) = FakeDeps::new(&domain_config);

    tokio::spawn(connect_websockets_on_incoming(incoming_streams));
    deps.transport_connector.set_behaviors(
        allow_all_routes(&domain_config, deps.static_ip_map()).map(|(target, behavior)| {
            let new_behavior = match &target {
                FakeTransportTarget::Tls { .. } => Behavior::DelayForever,
                FakeTransportTarget::TcpThroughProxy { .. } | FakeTransportTarget::Tcp { .. } => {
                    behavior
                }
            };
            (target, new_behavior)
        }),
    );

    let (timing, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;
    assert_matches!(outcome, Err(chat::ConnectError::Timeout));
    assert_eq!(timing, Duration::from_secs(60));

    use TransportConnectEvent::*;
    use TransportConnectEventStage::*;
    let tls_events = deps
        .transport_connector
        .recorded_events
        .lock()
        .unwrap()
        .drain(..)
        .map(|(event, _when)| event)
        .filter(|event| matches!(event, (TlsHandshake(..), _)))
        .collect_vec();

    assert_eq!(
        &tls_events,
        &[(
            TlsHandshake(Host::Domain(domain_config.connect.hostname.into())),
            Start
        )],
        "TLS handshake does not complete and no other handshakes start",
    );
}

#[derive(Debug)]
struct DnsLookupThatNeverCompletes;
#[async_trait]
impl DnsLookup for DnsLookupThatNeverCompletes {
    async fn dns_lookup(
        &self,
        _request: DnsLookupRequest,
    ) -> dns::Result<dns::lookup_result::LookupResult> {
        std::future::pending().await
    }
}

#[derive(Debug)]
struct DnsLookupThatFailsSlowly(Duration);
#[async_trait]
impl DnsLookup for DnsLookupThatFailsSlowly {
    async fn dns_lookup(
        &self,
        _request: DnsLookupRequest,
    ) -> dns::Result<dns::lookup_result::LookupResult> {
        tokio::time::sleep(self.0).await;
        Err(dns::DnsError::LookupFailed)
    }
}

#[derive(Debug)]
struct DnsLookupThatRunsSlowly(Duration, HashMap<&'static str, LookupResult>);
#[async_trait]
impl DnsLookup for DnsLookupThatRunsSlowly {
    async fn dns_lookup(
        &self,
        request: DnsLookupRequest,
    ) -> dns::Result<dns::lookup_result::LookupResult> {
        tokio::time::sleep(self.0).await;
        self.1
            .get(&*request.hostname)
            .cloned()
            .ok_or(dns::DnsError::LookupFailed)
    }
}

const DNS_STRATEGY_TIMEOUT: Duration = Duration::from_secs(7);

#[test_case(DnsLookupThatNeverCompletes, DNS_STRATEGY_TIMEOUT)]
#[test_case(
    DnsLookupThatFailsSlowly(Duration::from_secs(3)),
    Duration::from_secs(3)
)]
#[test_log::test(tokio::test(start_paused = true))]
async fn custom_dns_failure(lookup: impl DnsLookup + 'static, expected_duration: Duration) {
    let chat_domain_config = STAGING.chat_domain_config;
    let (mut deps, incoming_streams) = FakeDeps::new(&chat_domain_config);
    deps.dns_resolver = DnsResolver::new_custom(vec![(Box::new(lookup), DNS_STRATEGY_TIMEOUT)]);

    // Don't do anything with the incoming transport streams, just let them
    // accumulate in the unbounded stream.
    let _ignore_incoming_streams = incoming_streams;

    let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;

    assert_eq!(elapsed, expected_duration);
    assert_matches!(outcome, Err(chat::ConnectError::AllAttemptsFailed));
}

#[test_case(false, Duration::from_secs(60))]
#[test_case(true, Duration::from_secs(3))]
#[test_log::test(tokio::test(start_paused = true))]
async fn slow_dns(should_accept_connection: bool, expected_duration: Duration) {
    let chat_domain_config = STAGING.chat_domain_config;
    let (mut deps, incoming_streams) = FakeDeps::new(&chat_domain_config);
    deps.dns_resolver = DnsResolver::new_custom(vec![(
        Box::new(DnsLookupThatRunsSlowly(
            Duration::from_secs(3),
            deps.static_ip_map().clone(),
        )),
        DNS_STRATEGY_TIMEOUT,
    )]);

    if should_accept_connection {
        deps.transport_connector
            .set_behaviors(allow_all_routes(&chat_domain_config, deps.static_ip_map()));
    }

    tokio::spawn(connect_websockets_on_incoming(incoming_streams));
    let (elapsed, outcome) = timed(deps.connect_chat().map_ok(|_| ())).await;

    assert_eq!(elapsed, expected_duration);
    if should_accept_connection {
        outcome.expect("accepted")
    } else {
        assert_matches!(outcome, Err(chat::ConnectError::Timeout));
    }
}
