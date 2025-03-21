//
// Copyright 2025 Signal Messenger, LLC.
// SPDX-License-Identifier: AGPL-3.0-only
//

//! An example tool that allows to extract attestation message from enclaves.
//!
//! IMPORTANT: It outputs binary data, so make sure to pipe the output properly.
//!
//! Usage: `./svr2_attestation --username USERNAME --password PASSWORD | xxd`

use std::io::Write;
use std::time::Duration;

use attest::enclave;
use attest::enclave::Handshake;
use clap::Parser as _;
use http::uri::PathAndQuery;
use http::HeaderName;
use libsignal_net::auth::Auth;
use libsignal_net::connect_state::{ConnectState, SUGGESTED_CONNECT_CONFIG};
use libsignal_net::enclave::{EnclaveKind, EndpointParams, MrEnclave, NewHandshake, SgxPreQuantum};
use libsignal_net::svr::SvrConnection;
use libsignal_net_infra::dns::DnsResolver;
use libsignal_net_infra::route::DirectOrProxyProvider;
use libsignal_net_infra::utils::ObservableEvent;
use libsignal_net_infra::EnableDomainFronting;

const WS2_CONFIG: libsignal_net_infra::ws2::Config = libsignal_net_infra::ws2::Config {
    local_idle_timeout: Duration::from_secs(10),
    remote_idle_ping_timeout: Duration::from_secs(10),
    remote_idle_disconnect_timeout: Duration::from_secs(30),
};

#[derive(clap::Parser)]
struct Args {
    #[arg(long, env = "USERNAME")]
    username: String,
    #[arg(long, env = "PASSWORD")]
    password: String,
    #[arg(
        long,
        default_value_t = false,
        help = "Make requests to prod environment"
    )]
    prod: bool,
}

struct LoggingNewHandshake<E: EnclaveKind>(E);

impl<E: EnclaveKind> EnclaveKind for LoggingNewHandshake<E> {
    type RaftConfigType = E::RaftConfigType;

    fn url_path(enclave: &[u8]) -> PathAndQuery {
        E::url_path(enclave)
    }
}

fn cast_params<'a, T, U>(params: &'a EndpointParams<'a, T>) -> EndpointParams<'a, U>
where
    T: EnclaveKind<RaftConfigType = U::RaftConfigType>,
    U: EnclaveKind,
{
    EndpointParams {
        mr_enclave: MrEnclave::new(params.mr_enclave.as_ref()),
        raft_config: params.raft_config.clone(),
    }
}

impl<E: NewHandshake + 'static> NewHandshake for LoggingNewHandshake<E> {
    fn new_handshake(
        params: &EndpointParams<Self>,
        attestation_message: &[u8],
    ) -> enclave::Result<Handshake> {
        std::io::stdout()
            .write_all(attestation_message)
            .expect("can write to stdout");
        E::new_handshake(&cast_params(params), attestation_message)
    }
}

#[tokio::main]
async fn main() {
    let Args {
        username,
        password,
        prod,
    } = Args::parse();

    let auth = Auth { username, password };

    let env = if prod {
        libsignal_net::env::PROD.svr2
    } else {
        libsignal_net::env::STAGING.svr2
    };

    let network_changed_event = ObservableEvent::default();
    let resolver = DnsResolver::new(&network_changed_event);

    let confirmation_header = env
        .domain_config
        .connect
        .confirmation_header_name
        .map(HeaderName::from_static);
    let connect_state = ConnectState::new(SUGGESTED_CONNECT_CONFIG);

    let params: EndpointParams<'_, LoggingNewHandshake<SgxPreQuantum>> = cast_params(&env.params);

    let _connection = SvrConnection::connect(
        &connect_state,
        &resolver,
        &network_changed_event,
        DirectOrProxyProvider::maybe_proxied(env.route_provider(EnableDomainFronting::No), None),
        confirmation_header,
        WS2_CONFIG,
        &params,
        auth,
    )
    .await
    .expect("can connect");
}
