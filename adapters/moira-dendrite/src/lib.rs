//! Bounded loopback MCP client for Dendrite's structured grounding tools.
//!
//! Display text is ignored. Only `structuredContent` enters the deliberation
//! evidence path, and surface digests are recomputed before use.

use moira_core::deliberation::{
    CandidateV1, ClaimKindV1, CriterionV1, DeliberationIdV1, DeliberationPlanV1,
    EnsembleMemberIdV1, MethodRefV1, PersonaLensV1, SurfaceDepthV1,
    grounding::{
        self, FoldGroundingV1, GroundingRefusalV1, MemberIntentV1, OrthogonalityPacketV1,
        SurfaceEvidenceV1,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    time::Duration,
};

const PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientLimitsV1 {
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_members: usize,
    pub timeout: Duration,
}

impl Default for ClientLimitsV1 {
    fn default() -> Self {
        Self {
            max_request_bytes: 256 * 1024,
            max_response_bytes: 2 * 1024 * 1024,
            max_members: 16,
            timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientErrorV1 {
    InvalidConfig,
    Limit,
    Io(std::io::ErrorKind),
    Http(u16),
    Protocol,
    Remote,
    InvalidEvidence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroundedMemberRequestV1 {
    pub id: EnsembleMemberIdV1,
    pub lens: PersonaLensV1,
    pub fold_target: String,
    pub orthogonality_role: String,
    pub depth: SurfaceDepthV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanGroundingRequestV1 {
    pub id: DeliberationIdV1,
    pub question_digest: [u8; 32],
    pub method_load: String,
    pub method_state: String,
    pub fold_load: String,
    pub fold_tags: Vec<String>,
    pub orthogonality_load: String,
    pub claims: Vec<ClaimKindV1>,
    pub criteria: Vec<CriterionV1>,
    pub candidates: Vec<CandidateV1>,
    pub members: Vec<GroundedMemberRequestV1>,
    pub fuel: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanGroundingErrorV1 {
    Client(ClientErrorV1),
    InvalidMethodContext,
    IdentityChanged,
    Grounding(GroundingRefusalV1),
}

impl From<ClientErrorV1> for PlanGroundingErrorV1 {
    fn from(value: ClientErrorV1) -> Self {
        Self::Client(value)
    }
}

impl From<GroundingRefusalV1> for PlanGroundingErrorV1 {
    fn from(value: GroundingRefusalV1) -> Self {
        Self::Grounding(value)
    }
}

pub struct DendriteClient {
    address: SocketAddrV4,
    bearer: Option<String>,
    limits: ClientLimitsV1,
    session: Option<String>,
    next_id: u64,
}

impl std::fmt::Debug for DendriteClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DendriteClient(<redacted>)")
    }
}

impl DendriteClient {
    pub fn connect_loopback(
        port: u16,
        bearer: Option<String>,
        limits: ClientLimitsV1,
    ) -> Result<Self, ClientErrorV1> {
        if port == 0
            || limits.max_request_bytes == 0
            || limits.max_response_bytes == 0
            || limits.max_members < 2
            || limits.timeout.is_zero()
            || bearer.as_ref().is_some_and(|value| {
                value.is_empty()
                    || value.len() > 8_192
                    || value.bytes().any(|byte| matches!(byte, b'\r' | b'\n'))
            })
        {
            return Err(ClientErrorV1::InvalidConfig);
        }
        let mut client = Self {
            address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
            bearer,
            limits,
            session: None,
            next_id: 1,
        };
        let response = client.rpc(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "moira", "version": "0.1.0"}
            }),
        )?;
        if response
            .pointer("/result/protocolVersion")
            .and_then(Value::as_str)
            .is_none()
        {
            return Err(ClientErrorV1::Protocol);
        }
        client.notification("notifications/initialized", json!({}))?;
        Ok(client)
    }

    pub fn query_fold(
        &mut self,
        load_id: &str,
        tags: &[String],
        members: &[(EnsembleMemberIdV1, String)],
    ) -> Result<Vec<FoldGroundingV1>, ClientErrorV1> {
        if load_id.is_empty() || tags.is_empty() || members.len() > self.limits.max_members {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        let result = self.call_tool("query_fold", json!({"load_id": load_id, "tags": tags}))?;
        let groundings = result
            .get("groundings")
            .and_then(Value::as_array)
            .ok_or(ClientErrorV1::InvalidEvidence)?;
        if groundings.len() != members.len() {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        members
            .iter()
            .map(|(member, expected_ref)| {
                let item = groundings
                    .iter()
                    .find(|item| item.get("ref").and_then(Value::as_str) == Some(expected_ref))
                    .ok_or(ClientErrorV1::InvalidEvidence)?;
                Ok(FoldGroundingV1 {
                    member: *member,
                    authority_digest: digest(item.get("digest"))?,
                })
            })
            .collect()
    }

    /// Resolve the exact loaded Rune identity and one selected state's
    /// structured narrative identity. Selection remains a consumer decision.
    pub fn resolve_method(
        &mut self,
        load_id: &str,
        state: &str,
    ) -> Result<MethodRefV1, ClientErrorV1> {
        if load_id.is_empty() || state.is_empty() {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        let machine_digest = self.content_identity(load_id)?;
        let element = self.call_tool(
            "describe_element",
            json!({"load_id": load_id, "kind": "state", "name": state}),
        )?;
        if element.get("kind").and_then(Value::as_str) != Some("state")
            || element.get("name").and_then(Value::as_str) != Some(state)
        {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        let canonical = serde_json::to_vec(&element).map_err(|_| ClientErrorV1::Protocol)?;
        Ok(MethodRefV1 {
            machine_digest,
            state_digest: Sha256::digest(canonical).into(),
        })
    }

    /// Return Dendrite's exact identity for one loaded Rune entry.
    pub fn content_identity(&mut self, load_id: &str) -> Result<[u8; 32], ClientErrorV1> {
        if load_id.is_empty() {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        let identity = self.call_tool("content_identity", json!({"load_id": load_id}))?;
        if identity.get("load_id").and_then(Value::as_str) != Some(load_id) {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        digest(identity.get("digest"))
    }

    pub fn state_context_tags(
        &mut self,
        load_id: &str,
        state: &str,
    ) -> Result<Vec<String>, ClientErrorV1> {
        let result = self.call_tool("list_context_tags", json!({"load_id": load_id}))?;
        let object = result.as_object().ok_or(ClientErrorV1::InvalidEvidence)?;
        let Some(tags) = object.get(state) else {
            return Ok(Vec::new());
        };
        let tags = tags.as_array().ok_or(ClientErrorV1::InvalidEvidence)?;
        tags.iter()
            .map(|tag| {
                tag.as_str()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or(ClientErrorV1::InvalidEvidence)
            })
            .collect()
    }

    pub fn query_surface(
        &mut self,
        load_id: &str,
        target: &str,
        member: EnsembleMemberIdV1,
        expected_authority: [u8; 32],
        depth: SurfaceDepthV1,
    ) -> Result<SurfaceEvidenceV1, ClientErrorV1> {
        let depth_text = match depth {
            SurfaceDepthV1::Ambient => "ambient",
            SurfaceDepthV1::Activated => "activated",
            SurfaceDepthV1::Reference => "reference",
        };
        let result = self.call_tool(
            "query_surface",
            json!({"load_id": load_id, "target": target, "depth": depth_text}),
        )?;
        if result.get("target").and_then(Value::as_str) != Some(target)
            || digest(result.get("grounding_digest"))? != expected_authority
        {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        let surface = result
            .get("surface")
            .ok_or(ClientErrorV1::InvalidEvidence)?;
        if surface.get("depth").and_then(Value::as_str) != Some(depth_text) {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        let version = result
            .get("version")
            .and_then(Value::as_u64)
            .ok_or(ClientErrorV1::InvalidEvidence)?;
        let canonical = canonical_surface(version, surface)?;
        let computed: [u8; 32] = Sha256::digest(&canonical).into();
        if digest(result.get("surface_digest"))? != computed {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        Ok(SurfaceEvidenceV1 {
            member,
            authority_digest: expected_authority,
            surface_digest: computed,
            depth,
        })
    }

    pub fn query_orthogonality(
        &mut self,
        load_id: &str,
        roles: &[(EnsembleMemberIdV1, String)],
    ) -> Result<OrthogonalityPacketV1, ClientErrorV1> {
        if roles.is_empty()
            || roles.len() > self.limits.max_members
            || roles.iter().any(|(_, role)| role.is_empty())
        {
            return Err(ClientErrorV1::InvalidEvidence);
        }
        let names = roles.iter().map(|(_, role)| role).collect::<Vec<_>>();
        let query_value = json!({"load_id": load_id, "roles": names});
        let query = serde_json::to_vec(&query_value).map_err(|_| ClientErrorV1::Protocol)?;
        // Orthogonality is pairwise, so a singleton has no counterexample pair.
        // Pin that theorem to Dendrite's current lattice identity instead of
        // misreporting a lattice-check invocation, which requires two roles.
        if roles.len() == 1 {
            let lattice_digest = self.content_identity(load_id)?;
            let result = serde_json::to_vec(&json!({
                "all_orthogonal": true,
                "basis": "singleton_vacuous_pairwise_orthogonality",
                "lattice_digest": lattice_digest,
                "role": roles[0].1,
            }))
            .map_err(|_| ClientErrorV1::Protocol)?;
            return Ok(OrthogonalityPacketV1 {
                members: vec![roles[0].0],
                query_digest: Sha256::digest(query).into(),
                result_digest: Sha256::digest(result).into(),
                all_orthogonal: true,
            });
        }
        let result = self.call_tool("query_orthogonality", query_value)?;
        let all_orthogonal = result
            .get("all_orthogonal")
            .and_then(Value::as_bool)
            .ok_or(ClientErrorV1::InvalidEvidence)?;
        let canonical = serde_json::to_vec(&result).map_err(|_| ClientErrorV1::Protocol)?;
        Ok(OrthogonalityPacketV1 {
            members: roles.iter().map(|(member, _)| *member).collect(),
            query_digest: Sha256::digest(query).into(),
            result_digest: Sha256::digest(canonical).into(),
            all_orthogonal,
        })
    }

    /// Ground one complete plan against a Rune method state, its authored
    /// context tags, selected SME surfaces and one concern lattice.
    pub fn ground_plan(
        &mut self,
        request: PlanGroundingRequestV1,
    ) -> Result<DeliberationPlanV1, PlanGroundingErrorV1> {
        let method = self.resolve_method(&request.method_load, &request.method_state)?;
        let authored_tags = self.state_context_tags(&request.method_load, &request.method_state)?;
        let selected_tags = request
            .fold_tags
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if request.fold_tags.is_empty()
            || selected_tags.len() != request.fold_tags.len()
            || request
                .fold_tags
                .iter()
                .any(|tag| !authored_tags.contains(tag))
        {
            return Err(PlanGroundingErrorV1::InvalidMethodContext);
        }
        if self.content_identity(&request.method_load)? != method.machine_digest {
            return Err(PlanGroundingErrorV1::IdentityChanged);
        }
        let fold_identity = self.content_identity(&request.fold_load)?;
        let fold_members = request
            .members
            .iter()
            .map(|member| (member.id, member.fold_target.clone()))
            .collect::<Vec<_>>();
        let fold = self.query_fold(&request.fold_load, &request.fold_tags, &fold_members)?;
        let surfaces = request
            .members
            .iter()
            .map(|member| {
                let authority = fold
                    .iter()
                    .find(|value| value.member == member.id)
                    .ok_or(ClientErrorV1::InvalidEvidence)?
                    .authority_digest;
                self.query_surface(
                    &request.fold_load,
                    &member.fold_target,
                    member.id,
                    authority,
                    member.depth,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if self.content_identity(&request.fold_load)? != fold_identity {
            return Err(PlanGroundingErrorV1::IdentityChanged);
        }
        let roles = request
            .members
            .iter()
            .map(|member| (member.id, member.orthogonality_role.clone()))
            .collect::<Vec<_>>();
        let lattice_digest = self.content_identity(&request.orthogonality_load)?;
        let orthogonality = self.query_orthogonality(&request.orthogonality_load, &roles)?;
        if self.content_identity(&request.orthogonality_load)? != lattice_digest {
            return Err(PlanGroundingErrorV1::IdentityChanged);
        }
        let intents = request
            .members
            .iter()
            .map(|member| {
                Ok(MemberIntentV1 {
                    id: member.id,
                    lens: member.lens,
                    concern_root_digest: concern_root_digest(
                        lattice_digest,
                        &member.orthogonality_role,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, ClientErrorV1>>()?;
        let ensemble = grounding::assemble(&intents, &fold, &surfaces, orthogonality)?;
        Ok(DeliberationPlanV1 {
            id: request.id,
            question_digest: request.question_digest,
            method,
            claims: request.claims,
            criteria: request.criteria,
            candidates: request.candidates,
            members: ensemble.members,
            orthogonality: ensemble.orthogonality,
            fuel: request.fuel,
        })
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, ClientErrorV1> {
        let response = self.rpc("tools/call", json!({"name": name, "arguments": arguments}))?;
        if response.pointer("/result/isError").and_then(Value::as_bool) == Some(true) {
            return Err(ClientErrorV1::Remote);
        }
        response
            .pointer("/result/structuredContent")
            .cloned()
            .ok_or(ClientErrorV1::InvalidEvidence)
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, ClientErrorV1> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(ClientErrorV1::Limit)?;
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": id, "method": method, "params": params
        }))
        .map_err(|_| ClientErrorV1::Protocol)?;
        let response = self.post(&body, true)?;
        let value: Value =
            serde_json::from_slice(&response.body).map_err(|_| ClientErrorV1::Protocol)?;
        if value.get("id").and_then(Value::as_u64) != Some(id) || value.get("error").is_some() {
            return Err(ClientErrorV1::Remote);
        }
        Ok(value)
    }

    fn notification(&mut self, method: &str, params: Value) -> Result<(), ClientErrorV1> {
        let body = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "method": method, "params": params
        }))
        .map_err(|_| ClientErrorV1::Protocol)?;
        self.post(&body, false).map(|_| ())
    }

    fn post(&mut self, body: &[u8], expect_body: bool) -> Result<HttpResponse, ClientErrorV1> {
        if body.len() > self.limits.max_request_bytes {
            return Err(ClientErrorV1::Limit);
        }
        let mut stream = TcpStream::connect_timeout(&self.address.into(), self.limits.timeout)
            .map_err(|error| ClientErrorV1::Io(error.kind()))?;
        stream
            .set_read_timeout(Some(self.limits.timeout))
            .map_err(|error| ClientErrorV1::Io(error.kind()))?;
        stream
            .set_write_timeout(Some(self.limits.timeout))
            .map_err(|error| ClientErrorV1::Io(error.kind()))?;
        let mut headers = format!(
            "POST /mcp HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMCP-Protocol-Version: {PROTOCOL_VERSION}\r\nContent-Length: {}\r\nConnection: close\r\n",
            self.address,
            body.len()
        );
        if let Some(session) = &self.session {
            headers.push_str(&format!("Mcp-Session-Id: {session}\r\n"));
        }
        if let Some(token) = &self.bearer {
            headers.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        headers.push_str("\r\n");
        stream
            .write_all(headers.as_bytes())
            .and_then(|_| stream.write_all(body))
            .map_err(|error| ClientErrorV1::Io(error.kind()))?;
        read_response(&mut stream, self.limits.max_response_bytes, expect_body).inspect(
            |response| {
                if self.session.is_none() {
                    self.session = response.session.clone();
                }
            },
        )
    }
}

/// Bind a concern role to the exact lattice document from which it was
/// selected. The role name alone is not grounding.
pub fn concern_root_digest(
    lattice_digest: [u8; 32],
    role: &str,
) -> Result<[u8; 32], ClientErrorV1> {
    let role_len = u32::try_from(role.len()).map_err(|_| ClientErrorV1::Limit)?;
    if role.is_empty() {
        return Err(ClientErrorV1::InvalidEvidence);
    }
    let mut hash = Sha256::new();
    hash.update(b"MOIRACR1");
    hash.update(lattice_digest);
    hash.update(role_len.to_be_bytes());
    hash.update(role.as_bytes());
    Ok(hash.finalize().into())
}

struct HttpResponse {
    body: Vec<u8>,
    session: Option<String>,
}

fn read_response(
    stream: &mut TcpStream,
    maximum: usize,
    expect_body: bool,
) -> Result<HttpResponse, ClientErrorV1> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8_192];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| ClientErrorV1::Io(error.kind()))?;
        if count == 0 {
            break;
        }
        if count > maximum.saturating_sub(bytes.len()) {
            return Err(ClientErrorV1::Limit);
        }
        bytes.extend_from_slice(&buffer[..count]);
        if expect_body && extract_sse(&bytes).is_some() {
            break;
        }
    }
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(ClientErrorV1::Protocol)?;
    let head = std::str::from_utf8(&bytes[..split]).map_err(|_| ClientErrorV1::Protocol)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(ClientErrorV1::Protocol)?;
    if !(200..300).contains(&status) {
        return Err(ClientErrorV1::Http(status));
    }
    let session = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("mcp-session-id")
            .then(|| value.trim().to_owned())
    });
    let raw = &bytes[split + 4..];
    let body = if head
        .lines()
        .any(|line| line.to_ascii_lowercase().contains("text/event-stream"))
    {
        extract_sse(raw).ok_or(ClientErrorV1::Protocol)?
    } else {
        raw.to_vec()
    };
    Ok(HttpResponse { body, session })
}

fn extract_sse(bytes: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let start = text.find("data: ")? + 6;
    let end = text[start..]
        .find("\n\n")
        .or_else(|| text[start..].find("\r\n\r\n"))?;
    Some(text.as_bytes()[start..start + end].to_vec())
}

fn digest(value: Option<&Value>) -> Result<[u8; 32], ClientErrorV1> {
    let value = value
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or(ClientErrorV1::InvalidEvidence)?;
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ClientErrorV1::InvalidEvidence);
    }
    let mut result = [0u8; 32];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| ClientErrorV1::InvalidEvidence)?;
    }
    Ok(result)
}

fn canonical_surface(version: u64, surface: &Value) -> Result<Vec<u8>, ClientErrorV1> {
    let depth = surface
        .get("depth")
        .and_then(Value::as_str)
        .ok_or(ClientErrorV1::InvalidEvidence)?;
    let entries = surface
        .get("entries")
        .and_then(Value::as_array)
        .ok_or(ClientErrorV1::InvalidEvidence)?;
    if !matches!(version, 1 | 2) {
        return Err(ClientErrorV1::InvalidEvidence);
    }
    let mut canonical = format!(
        "{{\"version\":{version},\"depth\":{},\"entries\":[",
        json_string(depth)?
    );
    for (index, entry) in entries.iter().enumerate() {
        if index != 0 {
            canonical.push(',');
        }
        let kind = entry
            .get("kind")
            .and_then(Value::as_str)
            .ok_or(ClientErrorV1::InvalidEvidence)?;
        let text = entry
            .get("text")
            .and_then(Value::as_str)
            .ok_or(ClientErrorV1::InvalidEvidence)?;
        if version == 1 {
            canonical.push_str(&format!(
                "{{\"kind\":{},\"text\":{}}}",
                json_string(kind)?,
                json_string(text)?
            ));
        } else {
            let id = entry
                .get("id")
                .and_then(Value::as_str)
                .ok_or(ClientErrorV1::InvalidEvidence)?;
            canonical.push_str(&format!(
                "{{\"id\":{},\"kind\":{},\"text\":{}}}",
                json_string(id)?,
                json_string(kind)?,
                json_string(text)?
            ));
        }
    }
    canonical.push_str("]}");
    Ok(canonical.into_bytes())
}

fn json_string(value: &str) -> Result<String, ClientErrorV1> {
    serde_json::to_string(value).map_err(|_| ClientErrorV1::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_surface_matches_dendrites_ordered_contract() {
        let surface = json!({
            "depth": "ambient",
            "entries": [{"kind": "guidance", "text": "retain exact authority"}]
        });
        assert_eq!(
            canonical_surface(1, &surface).unwrap(),
            br#"{"version":1,"depth":"ambient","entries":[{"kind":"guidance","text":"retain exact authority"}]}"#
        );
    }

    #[test]
    fn sse_extraction_ignores_display_framing_and_returns_one_json_event() {
        assert_eq!(
            extract_sse(b"event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1}\n\nignored"),
            Some(br#"{"jsonrpc":"2.0","id":1}"#.to_vec())
        );
    }

    #[test]
    fn digest_parser_is_closed_to_exact_sha256_shape() {
        let valid = Value::String(format!("sha256:{}", "ab".repeat(32)));
        assert_eq!(digest(Some(&valid)).unwrap(), [0xab; 32]);
        assert_eq!(
            digest(Some(&Value::String("sha256:ab".into()))),
            Err(ClientErrorV1::InvalidEvidence)
        );
        assert_eq!(
            digest(Some(&Value::String(format!("sha256:{}", "AB".repeat(32))))),
            Err(ClientErrorV1::InvalidEvidence)
        );
    }

    #[test]
    fn concern_identity_binds_both_lattice_and_role() {
        let first = concern_root_digest([1; 32], "rune:architecture").unwrap();
        assert_ne!(
            first,
            concern_root_digest([2; 32], "rune:architecture").unwrap()
        );
        assert_ne!(first, concern_root_digest([1; 32], "rune:network").unwrap());
        assert_eq!(
            concern_root_digest([1; 32], ""),
            Err(ClientErrorV1::InvalidEvidence)
        );
    }
}
