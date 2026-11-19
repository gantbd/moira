//! Canonical bounded bytes for the revisioned deliberation registry.

use super::*;
use crate::deliberation::persistence::{
    CodecErrorV1 as ChildCodecErrorV1, CodecLimitsV1 as ChildCodecLimitsV1, decode as decode_child,
    encode as encode_child,
};
use crate::deliberation::review::persistence::{
    ReviewCodecErrorV1, ReviewCodecLimitsV1, decode_review_checkpoint, encode_review_checkpoint,
};

const MAGIC: &[u8; 8] = b"MOIRADR5";
const CODEC_VERSION: u8 = 5;
const LEGACY_MAGIC_V4: &[u8; 8] = b"MOIRADR4";
const LEGACY_CODEC_VERSION_V4: u8 = 4;
const LEGACY_MAGIC_V3: &[u8; 8] = b"MOIRADR3";
const LEGACY_CODEC_VERSION_V3: u8 = 3;
const LEGACY_MAGIC_V2: &[u8; 8] = b"MOIRADR2";
const LEGACY_CODEC_VERSION_V2: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecLimitsV1 {
    pub max_bytes: usize,
    pub max_deliberation_bytes: usize,
    pub registry: RegistryLimitsV1,
}

impl Default for CodecLimitsV1 {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_deliberation_bytes: 8 * 1024 * 1024,
            registry: RegistryLimitsV1::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecErrorV1 {
    InvalidLimits,
    Limit,
    Capacity,
    Truncated,
    Invalid,
    NonCanonical,
    Child(ChildCodecErrorV1),
    Review(ReviewCodecErrorV1),
    Registry(RegistryRefusalV1),
}

struct Writer {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Writer {
    fn new(maximum: usize) -> Self {
        Self {
            bytes: Vec::new(),
            maximum,
        }
    }
    fn raw(&mut self, value: &[u8]) -> Result<(), CodecErrorV1> {
        if value.len() > self.maximum.saturating_sub(self.bytes.len()) {
            return Err(CodecErrorV1::Limit);
        }
        self.bytes
            .try_reserve(value.len())
            .map_err(|_| CodecErrorV1::Capacity)?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    fn byte(&mut self, value: u8) -> Result<(), CodecErrorV1> {
        self.raw(&[value])
    }
    fn number(&mut self, value: u64) -> Result<(), CodecErrorV1> {
        self.raw(&value.to_be_bytes())
    }
    fn count(&mut self, value: usize) -> Result<(), CodecErrorV1> {
        self.number(u64::try_from(value).map_err(|_| CodecErrorV1::Limit)?)
    }
    fn bytes(&mut self, value: &[u8], maximum: usize) -> Result<(), CodecErrorV1> {
        if value.len() > maximum {
            return Err(CodecErrorV1::Limit);
        }
        self.count(value.len())?;
        self.raw(value)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], maximum: usize) -> Result<Self, CodecErrorV1> {
        if bytes.len() > maximum {
            return Err(CodecErrorV1::Limit);
        }
        Ok(Self { bytes, position: 0 })
    }
    fn raw<const N: usize>(&mut self) -> Result<[u8; N], CodecErrorV1> {
        let end = self.position.checked_add(N).ok_or(CodecErrorV1::Invalid)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CodecErrorV1::Truncated)?;
        self.position = end;
        value.try_into().map_err(|_| CodecErrorV1::Invalid)
    }
    fn byte(&mut self) -> Result<u8, CodecErrorV1> {
        Ok(self.raw::<1>()?[0])
    }
    fn number(&mut self) -> Result<u64, CodecErrorV1> {
        Ok(u64::from_be_bytes(self.raw()?))
    }
    fn count(&mut self, maximum: usize) -> Result<usize, CodecErrorV1> {
        let value = usize::try_from(self.number()?).map_err(|_| CodecErrorV1::Limit)?;
        if value > maximum {
            return Err(CodecErrorV1::Limit);
        }
        Ok(value)
    }
    fn bytes(&mut self, maximum: usize) -> Result<&'a [u8], CodecErrorV1> {
        let count = self.count(maximum)?;
        let end = self
            .position
            .checked_add(count)
            .ok_or(CodecErrorV1::Invalid)?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(CodecErrorV1::Truncated)?;
        self.position = end;
        Ok(value)
    }
    fn finish(self) -> Result<(), CodecErrorV1> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecErrorV1::Invalid)
        }
    }
}

pub fn encode(
    checkpoint: &RegistryCheckpointV1,
    limits: CodecLimitsV1,
) -> Result<Vec<u8>, CodecErrorV1> {
    validate_codec_limits(limits)?;
    validate_host_bounds(checkpoint.limits, limits.registry)?;
    DeliberationRegistryV1::recover(checkpoint.clone()).map_err(CodecErrorV1::Registry)?;
    let mut writer = Writer::new(limits.max_bytes);
    writer.raw(MAGIC)?;
    writer.byte(CODEC_VERSION)?;
    encode_limits(&mut writer, checkpoint.limits)?;
    writer.number(checkpoint.revision.0)?;
    writer.count(checkpoint.deliberations.len())?;
    let child_limits = ChildCodecLimitsV1 {
        max_bytes: limits.max_deliberation_bytes,
        domain: limits.registry.deliberation,
    };
    for deliberation in &checkpoint.deliberations {
        let bytes = encode_child(deliberation, child_limits).map_err(CodecErrorV1::Child)?;
        writer.bytes(&bytes, limits.max_deliberation_bytes)?;
    }
    writer.count(checkpoint.reviews.len())?;
    let review_limits = review_limits(limits);
    for review in &checkpoint.reviews {
        let bytes =
            encode_review_checkpoint(review, review_limits).map_err(CodecErrorV1::Review)?;
        writer.bytes(&bytes, limits.max_deliberation_bytes)?;
    }
    Ok(writer.bytes)
}

pub fn decode(bytes: &[u8], limits: CodecLimitsV1) -> Result<RegistryCheckpointV1, CodecErrorV1> {
    validate_codec_limits(limits)?;
    let mut reader = Reader::new(bytes, limits.max_bytes)?;
    let magic = reader.raw::<8>()?;
    let version = reader.byte()?;
    let legacy = match (&magic, version) {
        (value, CODEC_VERSION) if value == MAGIC => None,
        (value, LEGACY_CODEC_VERSION_V4) if value == LEGACY_MAGIC_V4 => Some(false),
        (value, LEGACY_CODEC_VERSION_V3) if value == LEGACY_MAGIC_V3 => Some(true),
        (value, LEGACY_CODEC_VERSION_V2) if value == LEGACY_MAGIC_V2 => Some(true),
        _ => return Err(CodecErrorV1::Invalid),
    };
    let registry_limits = decode_limits(&mut reader, limits.registry)?;
    let revision = RegistryRevisionV1(reader.number()?);
    let count = reader.count(registry_limits.max_deliberations)?;
    let child_limits = ChildCodecLimitsV1 {
        max_bytes: limits.max_deliberation_bytes,
        domain: registry_limits.deliberation,
    };
    let mut deliberations = Vec::new();
    deliberations
        .try_reserve(count)
        .map_err(|_| CodecErrorV1::Capacity)?;
    for _ in 0..count {
        let child = reader.bytes(limits.max_deliberation_bytes)?;
        deliberations.push(decode_child(child, child_limits).map_err(CodecErrorV1::Child)?);
    }
    let mut reviews = Vec::new();
    if legacy != Some(true) {
        let review_count = reader.count(registry_limits.max_deliberations)?;
        reviews
            .try_reserve(review_count)
            .map_err(|_| CodecErrorV1::Capacity)?;
        let review_limits = review_limits(limits);
        for _ in 0..review_count {
            let child = reader.bytes(limits.max_deliberation_bytes)?;
            reviews.push(
                decode_review_checkpoint(child, review_limits).map_err(CodecErrorV1::Review)?,
            );
        }
    }
    reader.finish()?;
    let checkpoint = RegistryCheckpointV1 {
        version: CODEC_VERSION,
        limits: registry_limits,
        revision,
        deliberations,
        reviews,
    };
    DeliberationRegistryV1::recover(checkpoint.clone()).map_err(CodecErrorV1::Registry)?;
    if legacy.is_none() && encode(&checkpoint, limits)? != bytes {
        return Err(CodecErrorV1::NonCanonical);
    }
    Ok(checkpoint)
}

fn review_limits(limits: CodecLimitsV1) -> ReviewCodecLimitsV1 {
    ReviewCodecLimitsV1 {
        max_bytes: limits.max_deliberation_bytes,
        review: crate::deliberation::review::ReviewLimitsV1::default(),
        max_reports: limits.registry.deliberation.max_members,
        max_evidence_per_report: limits.registry.deliberation.max_evidence_per_report,
        max_dissent_per_report: limits.registry.deliberation.max_dissent_per_report,
    }
}

fn encode_limits(writer: &mut Writer, limits: RegistryLimitsV1) -> Result<(), CodecErrorV1> {
    writer.count(limits.max_deliberations)?;
    let domain = limits.deliberation;
    for value in [
        domain.max_claims,
        domain.max_criteria,
        domain.max_candidates,
        domain.max_members,
        domain.max_evidence_per_report,
        domain.max_dissent_per_report,
    ] {
        writer.count(value)?;
    }
    writer.number(domain.max_fuel.into())
}

fn decode_limits(
    reader: &mut Reader<'_>,
    host: RegistryLimitsV1,
) -> Result<RegistryLimitsV1, CodecErrorV1> {
    let limits = RegistryLimitsV1 {
        max_deliberations: positive_count(reader, host.max_deliberations)?,
        deliberation: DeliberationLimitsV1 {
            max_claims: positive_count(reader, host.deliberation.max_claims)?,
            max_criteria: positive_count(reader, host.deliberation.max_criteria)?,
            max_candidates: positive_count(reader, host.deliberation.max_candidates)?,
            max_members: positive_count(reader, host.deliberation.max_members)?,
            max_evidence_per_report: positive_count(
                reader,
                host.deliberation.max_evidence_per_report,
            )?,
            max_dissent_per_report: positive_count(
                reader,
                host.deliberation.max_dissent_per_report,
            )?,
            max_fuel: reader
                .number()?
                .try_into()
                .map_err(|_| CodecErrorV1::Limit)?,
        },
    };
    validate_host_bounds(limits, host)?;
    Ok(limits)
}

fn positive_count(reader: &mut Reader<'_>, maximum: usize) -> Result<usize, CodecErrorV1> {
    let value = reader.count(maximum)?;
    if value == 0 {
        Err(CodecErrorV1::InvalidLimits)
    } else {
        Ok(value)
    }
}

fn validate_codec_limits(limits: CodecLimitsV1) -> Result<(), CodecErrorV1> {
    if limits.max_bytes == 0
        || limits.max_bytes > isize::MAX as usize
        || limits.max_deliberation_bytes == 0
        || limits.max_deliberation_bytes > limits.max_bytes
    {
        return Err(CodecErrorV1::InvalidLimits);
    }
    validate_host_bounds(limits.registry, limits.registry)
}

fn validate_host_bounds(
    value: RegistryLimitsV1,
    host: RegistryLimitsV1,
) -> Result<(), CodecErrorV1> {
    let value_domain = value.deliberation;
    let host_domain = host.deliberation;
    if value.max_deliberations == 0
        || value.max_deliberations > host.max_deliberations
        || value_domain.max_claims == 0
        || value_domain.max_claims > host_domain.max_claims
        || value_domain.max_criteria == 0
        || value_domain.max_criteria > host_domain.max_criteria
        || value_domain.max_candidates == 0
        || value_domain.max_candidates > host_domain.max_candidates
        || value_domain.max_members == 0
        || value_domain.max_members > host_domain.max_members
        || value_domain.max_evidence_per_report == 0
        || value_domain.max_evidence_per_report > host_domain.max_evidence_per_report
        || value_domain.max_dissent_per_report == 0
        || value_domain.max_dissent_per_report > host_domain.max_dissent_per_report
        || value_domain.max_fuel == 0
        || value_domain.max_fuel > host_domain.max_fuel
    {
        Err(CodecErrorV1::Limit)
    } else {
        Ok(())
    }
}
