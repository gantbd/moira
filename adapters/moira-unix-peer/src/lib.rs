//! OS-backed peer authentication for Moira's selected local Unix profile.
//!
//! This adapter maps a kernel-established effective UID/GID pair only to
//! Moira-owned read and mutation facts. Saga operator authority is a separate,
//! injected decision and is never inferred from the operating-system account.

#![forbid(unsafe_code)]

use moira_local_daemon::unix::{AgentParticipationAuthenticator, Authenticator, SagaAuthenticator};
use moira_protocol::{OperatorOperationV1, agent_participation as participation};
use moira_service::{
    AccessV1,
    agent_participation::{PermissionAccessV1, PermissionAuthorizer},
    saga::{OperatorAccessV1, OperatorAuthorizer},
};
use std::{collections::BTreeMap, os::unix::net::UnixStream};

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct MappingV1 {
    pub effective_uid: u32,
    pub effective_gid: u32,
    pub access: AccessV1,
}

/// OS peer authentication composed with an independent, command-scoped
/// permission authorizer. OS identity never implies provider permission.
pub struct UnixAgentParticipationAuthenticatorV1<P> {
    peer: UnixPeerAuthenticatorV1,
    permission: P,
}

impl<P> UnixAgentParticipationAuthenticatorV1<P> {
    pub const fn new(peer: UnixPeerAuthenticatorV1, permission: P) -> Self {
        Self { peer, permission }
    }

    pub fn into_parts(self) -> (UnixPeerAuthenticatorV1, P) {
        (self.peer, self.permission)
    }
}

impl<P: PermissionAuthorizer> AgentParticipationAuthenticator
    for UnixAgentParticipationAuthenticatorV1<P>
{
    type Error = SagaAuthenticationErrorV1<P::Error>;

    fn authenticate(&mut self, stream: &UnixStream) -> Result<AccessV1, Self::Error> {
        self.peer
            .authenticate_peer(stream)
            .map_err(SagaAuthenticationErrorV1::Peer)
    }

    fn authorize_permission(
        &mut self,
        selection: participation::PermissionSelectionV1,
    ) -> Result<Option<PermissionAccessV1>, Self::Error> {
        self.permission
            .authorize_permission(selection)
            .map_err(SagaAuthenticationErrorV1::Operator)
    }
}

impl std::fmt::Debug for MappingV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MappingV1")
            .field("effective_identity", &"<redacted>")
            .field("access", &self.access)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationErrorV1 {
    InvalidLimit,
    Empty,
    Capacity,
    DuplicatePeer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthenticationErrorV1 {
    CredentialUnavailable,
    UnknownPeer,
}

/// Bounded exact mapping from kernel peer identities to ordinary Moira access.
pub struct UnixPeerAuthenticatorV1 {
    mappings: BTreeMap<(u32, u32), AccessV1>,
}

impl UnixPeerAuthenticatorV1 {
    pub fn new(
        max_mappings: usize,
        mappings: Vec<MappingV1>,
    ) -> Result<Self, ConfigurationErrorV1> {
        if max_mappings == 0 {
            return Err(ConfigurationErrorV1::InvalidLimit);
        }
        if mappings.is_empty() {
            return Err(ConfigurationErrorV1::Empty);
        }
        if mappings.len() > max_mappings {
            return Err(ConfigurationErrorV1::Capacity);
        }
        let mut retained = BTreeMap::new();
        for mapping in mappings {
            if retained
                .insert(
                    (mapping.effective_uid, mapping.effective_gid),
                    mapping.access,
                )
                .is_some()
            {
                return Err(ConfigurationErrorV1::DuplicatePeer);
            }
        }
        Ok(Self { mappings: retained })
    }

    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
    }

    fn authenticate_peer(&self, stream: &UnixStream) -> Result<AccessV1, AuthenticationErrorV1> {
        let peer = peer_effective_ids(stream)
            .map_err(|()| AuthenticationErrorV1::CredentialUnavailable)?;
        self.mappings
            .get(&peer)
            .copied()
            .ok_or(AuthenticationErrorV1::UnknownPeer)
    }
}

impl Authenticator for UnixPeerAuthenticatorV1 {
    type Error = AuthenticationErrorV1;

    fn authenticate(&mut self, stream: &UnixStream) -> Result<AccessV1, Self::Error> {
        self.authenticate_peer(stream)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum SagaAuthenticationErrorV1<E> {
    Peer(AuthenticationErrorV1),
    Operator(E),
}

/// Saga authentication that composes OS peer access with independent operator
/// authorization. The peer mapping is never consulted for operator decisions.
pub struct UnixSagaAuthenticatorV1<O> {
    peer: UnixPeerAuthenticatorV1,
    operator: O,
}

impl<O> UnixSagaAuthenticatorV1<O> {
    pub const fn new(peer: UnixPeerAuthenticatorV1, operator: O) -> Self {
        Self { peer, operator }
    }

    pub fn into_parts(self) -> (UnixPeerAuthenticatorV1, O) {
        (self.peer, self.operator)
    }
}

impl<O: OperatorAuthorizer> SagaAuthenticator for UnixSagaAuthenticatorV1<O> {
    type Error = SagaAuthenticationErrorV1<O::Error>;

    fn authenticate(&mut self, stream: &UnixStream) -> Result<AccessV1, Self::Error> {
        self.peer
            .authenticate_peer(stream)
            .map_err(SagaAuthenticationErrorV1::Peer)
    }

    fn authorize_operator(
        &mut self,
        operation: OperatorOperationV1,
    ) -> Result<Option<OperatorAccessV1>, Self::Error> {
        self.operator
            .authorize_operator(operation)
            .map_err(SagaAuthenticationErrorV1::Operator)
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn peer_effective_ids(stream: &UnixStream) -> Result<(u32, u32), ()> {
    nix::unistd::getpeereid(stream)
        .map(|(uid, gid)| (uid.as_raw(), gid.as_raw()))
        .map_err(|_| ())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_effective_ids(stream: &UnixStream) -> Result<(u32, u32), ()> {
    nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
        .map(|credentials| (credentials.uid(), credentials.gid()))
        .map_err(|_| ())
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "linux",
    target_os = "android"
)))]
fn peer_effective_ids(_: &UnixStream) -> Result<(u32, u32), ()> {
    Err(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actual_mapping() -> MappingV1 {
        MappingV1 {
            effective_uid: nix::unistd::geteuid().as_raw(),
            effective_gid: nix::unistd::getegid().as_raw(),
            access: AccessV1 {
                read: true,
                mutate: false,
            },
        }
    }

    #[test]
    fn kernel_peer_identity_maps_to_redacted_moira_access() {
        let (server, _client) = UnixStream::pair().unwrap();
        let mapping = actual_mapping();
        let mut authenticator = UnixPeerAuthenticatorV1::new(1, vec![mapping]).unwrap();
        assert_eq!(authenticator.authenticate(&server).unwrap(), mapping.access);
        assert!(!format!("{mapping:?}").contains(&nix::unistd::geteuid().to_string()));
    }

    #[test]
    fn bounds_duplicates_and_unknown_peers_fail_closed() {
        let mapping = actual_mapping();
        assert!(matches!(
            UnixPeerAuthenticatorV1::new(0, vec![mapping]),
            Err(ConfigurationErrorV1::InvalidLimit)
        ));
        assert!(matches!(
            UnixPeerAuthenticatorV1::new(1, vec![]),
            Err(ConfigurationErrorV1::Empty)
        ));
        assert!(matches!(
            UnixPeerAuthenticatorV1::new(1, vec![mapping, mapping]),
            Err(ConfigurationErrorV1::Capacity)
        ));
        assert!(matches!(
            UnixPeerAuthenticatorV1::new(2, vec![mapping, mapping]),
            Err(ConfigurationErrorV1::DuplicatePeer)
        ));
        let (server, _client) = UnixStream::pair().unwrap();
        let mut unknown = UnixPeerAuthenticatorV1::new(
            1,
            vec![MappingV1 {
                effective_uid: mapping.effective_uid.wrapping_add(1),
                ..mapping
            }],
        )
        .unwrap();
        assert_eq!(
            unknown.authenticate(&server),
            Err(AuthenticationErrorV1::UnknownPeer)
        );
    }
}
