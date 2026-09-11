use std::collections::HashSet;

use crate::{AttemptId, DomainError, ExecutionId, Outcome, WorkId};

/// Schema version of a persisted execution specification.
pub const EXECUTION_SPEC_SCHEMA_VERSION: u32 = 1;
/// Schema version of a persisted control-plane confirmed outcome.
pub const CONFIRMED_OUTCOME_SCHEMA_VERSION: u32 = 1;

/// Reaction fixed for channel failures in one immutable execution spec.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChannelPolicy {
    Fail,
    Park,
    RetryThenFail,
}

impl ChannelPolicy {
    pub const ALL: [Self; 3] = [Self::Fail, Self::Park, Self::RetryThenFail];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fail => "fail",
            Self::Park => "park",
            Self::RetryThenFail => "retry_then_fail",
        }
    }
}

/// A content-addressed SHA-256 digest persisted in an execution snapshot.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ContentDigest(String);

impl ContentDigest {
    pub fn parse(raw: impl AsRef<str>) -> Result<Self, DomainError> {
        let raw = raw.as_ref();
        let Some(hex) = raw.strip_prefix("sha256:") else {
            return Err(DomainError::InvalidExecutionSpec);
        };
        if hex.len() != 64
            || !hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        {
            return Err(DomainError::InvalidExecutionSpec);
        }
        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Current closed set of secret-reference sources.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum SecretSource {
    EnvironmentVariable(String),
}

impl SecretSource {
    pub fn environment_variable(name: impl Into<String>) -> Result<Self, DomainError> {
        let name = name.into();
        let mut chars = name.chars();
        let valid_first = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        if !valid_first || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(DomainError::InvalidExecutionSpec);
        }
        Ok(Self::EnvironmentVariable(name))
    }

    pub fn reference(&self) -> &str {
        match self {
            Self::EnvironmentVariable(name) => name,
        }
    }
}

/// Typed configuration reference for a secret made available to a Worker.
/// It stores a source identifier and has no field for the secret value.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SecretRef {
    name: String,
    source: SecretSource,
}

impl SecretRef {
    pub fn new(name: impl Into<String>, source: SecretSource) -> Result<Self, DomainError> {
        let name = name.into();
        if !valid_text(&name) {
            return Err(DomainError::InvalidExecutionSpec);
        }
        Ok(Self { name, source })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn source(&self) -> &SecretSource {
        &self.source
    }
}

/// Configuration snapshotted once for a logical execution.
///
/// Private fields and read-only accessors make mutation impossible after
/// construction. Adapters persist this value; resume loads it rather than
/// resolving a profile again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionSpec {
    schema_version: u32,
    work_id: WorkId,
    worker_profile: String,
    worker_config_digest: ContentDigest,
    runtime_digest: ContentDigest,
    wall_clock_budget_ms: u64,
    retry_limit: u32,
    channel_policy: ChannelPolicy,
    secret_refs: Vec<SecretRef>,
}

impl ExecutionSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        schema_version: u32,
        work_id: WorkId,
        worker_profile: impl Into<String>,
        worker_config_digest: ContentDigest,
        runtime_digest: ContentDigest,
        wall_clock_budget_ms: u64,
        retry_limit: u32,
        channel_policy: ChannelPolicy,
        secret_refs: Vec<SecretRef>,
    ) -> Result<Self, DomainError> {
        if schema_version != EXECUTION_SPEC_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedSchemaVersion(schema_version));
        }
        let worker_profile = worker_profile.into();
        let unique_names: HashSet<&str> = secret_refs.iter().map(|item| item.name()).collect();
        if !valid_text(&worker_profile)
            || wall_clock_budget_ms == 0
            || unique_names.len() != secret_refs.len()
        {
            return Err(DomainError::InvalidExecutionSpec);
        }
        Ok(Self {
            schema_version,
            work_id,
            worker_profile,
            worker_config_digest,
            runtime_digest,
            wall_clock_budget_ms,
            retry_limit,
            channel_policy,
            secret_refs,
        })
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn work_id(&self) -> &WorkId {
        &self.work_id
    }

    pub fn worker_profile(&self) -> &str {
        &self.worker_profile
    }

    pub fn worker_config_digest(&self) -> &ContentDigest {
        &self.worker_config_digest
    }

    pub fn runtime_digest(&self) -> &ContentDigest {
        &self.runtime_digest
    }

    pub fn wall_clock_budget_ms(&self) -> u64 {
        self.wall_clock_budget_ms
    }

    pub fn retry_limit(&self) -> u32 {
        self.retry_limit
    }

    pub fn channel_policy(&self) -> ChannelPolicy {
        self.channel_policy
    }

    pub fn secret_refs(&self) -> &[SecretRef] {
        &self.secret_refs
    }
}

/// Outcome stamped by the control plane after matching active-attempt proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmedOutcome {
    schema_version: u32,
    work_id: WorkId,
    execution_id: ExecutionId,
    attempt_id: AttemptId,
    outcome: Outcome,
    confirmed_at_unix_ms: u64,
}

impl ConfirmedOutcome {
    pub fn new(
        schema_version: u32,
        execution_id: ExecutionId,
        attempt_id: AttemptId,
        spec: &ExecutionSpec,
        outcome: Outcome,
        confirmed_at_unix_ms: u64,
    ) -> Result<Self, DomainError> {
        if schema_version != CONFIRMED_OUTCOME_SCHEMA_VERSION {
            return Err(DomainError::UnsupportedSchemaVersion(schema_version));
        }
        if outcome.worker_profile() != spec.worker_profile() {
            return Err(DomainError::InvalidConfirmedOutcome);
        }
        Ok(Self {
            schema_version,
            work_id: spec.work_id().clone(),
            execution_id,
            attempt_id,
            outcome,
            confirmed_at_unix_ms,
        })
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn work_id(&self) -> &WorkId {
        &self.work_id
    }

    pub fn execution_id(&self) -> &ExecutionId {
        &self.execution_id
    }

    pub fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    pub fn confirmed_at_unix_ms(&self) -> u64 {
        self.confirmed_at_unix_ms
    }
}

fn valid_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1024 && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OUTCOME_SCHEMA_VERSION, OutcomeKind};

    const CONFIG_DIGEST: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const RUNTIME_DIGEST: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";

    fn spec() -> ExecutionSpec {
        ExecutionSpec::new(
            EXECUTION_SPEC_SCHEMA_VERSION,
            WorkId::parse("work-1").unwrap(),
            "stub",
            ContentDigest::parse(CONFIG_DIGEST).unwrap(),
            ContentDigest::parse(RUNTIME_DIGEST).unwrap(),
            5_000,
            2,
            ChannelPolicy::RetryThenFail,
            vec![
                SecretRef::new(
                    "TOKEN",
                    SecretSource::environment_variable("TOKEN").unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap()
    }

    #[test]
    fn spec_exposes_the_immutable_execution_snapshot() {
        let spec = spec();
        assert_eq!(spec.work_id().as_str(), "work-1");
        assert_eq!(spec.worker_profile(), "stub");
        assert_eq!(spec.worker_config_digest().as_str(), CONFIG_DIGEST);
        assert_eq!(spec.runtime_digest().as_str(), RUNTIME_DIGEST);
        assert_eq!(spec.wall_clock_budget_ms(), 5_000);
        assert_eq!(spec.retry_limit(), 2);
        assert_eq!(spec.channel_policy(), ChannelPolicy::RetryThenFail);
        assert_eq!(spec.secret_refs()[0].source().reference(), "TOKEN");
    }

    #[test]
    fn duplicate_secret_mount_names_are_rejected() {
        let err = ExecutionSpec::new(
            EXECUTION_SPEC_SCHEMA_VERSION,
            WorkId::parse("work-1").unwrap(),
            "stub",
            ContentDigest::parse(CONFIG_DIGEST).unwrap(),
            ContentDigest::parse(RUNTIME_DIGEST).unwrap(),
            5_000,
            0,
            ChannelPolicy::Fail,
            vec![
                SecretRef::new(
                    "TOKEN",
                    SecretSource::environment_variable("FIRST").unwrap(),
                )
                .unwrap(),
                SecretRef::new(
                    "TOKEN",
                    SecretSource::environment_variable("SECOND").unwrap(),
                )
                .unwrap(),
            ],
        )
        .unwrap_err();
        assert_eq!(err, DomainError::InvalidExecutionSpec);
    }

    #[test]
    fn secret_source_accepts_a_reference_name_not_an_inline_value() {
        assert!(SecretSource::environment_variable("MODEL_TOKEN").is_ok());
        assert_eq!(
            SecretSource::environment_variable("token value with spaces"),
            Err(DomainError::InvalidExecutionSpec)
        );
    }

    #[test]
    fn digest_rejects_unversioned_or_malformed_content() {
        assert!(ContentDigest::parse(CONFIG_DIGEST).is_ok());
        assert_eq!(
            ContentDigest::parse("worker-config"),
            Err(DomainError::InvalidExecutionSpec)
        );
        assert_eq!(
            ContentDigest::parse("sha256:ABC"),
            Err(DomainError::InvalidExecutionSpec)
        );
    }

    #[test]
    fn confirmed_outcome_binds_work_execution_and_attempt() {
        let spec = spec();
        let confirmed = ConfirmedOutcome::new(
            CONFIRMED_OUTCOME_SCHEMA_VERSION,
            ExecutionId::parse("execution-1").unwrap(),
            AttemptId::parse("attempt-1").unwrap(),
            &spec,
            Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap(),
            42,
        )
        .unwrap();
        assert_eq!(confirmed.schema_version(), CONFIRMED_OUTCOME_SCHEMA_VERSION);
        assert_eq!(confirmed.work_id(), spec.work_id());
        assert_eq!(confirmed.execution_id().as_str(), "execution-1");
        assert_eq!(confirmed.attempt_id().as_str(), "attempt-1");
        assert_eq!(confirmed.outcome().kind(), OutcomeKind::Succeeded);
        assert_eq!(confirmed.confirmed_at_unix_ms(), 42);
    }

    #[test]
    fn confirmed_outcome_rejects_a_foreign_profile() {
        let err = ConfirmedOutcome::new(
            CONFIRMED_OUTCOME_SCHEMA_VERSION,
            ExecutionId::parse("execution-1").unwrap(),
            AttemptId::parse("attempt-1").unwrap(),
            &spec(),
            Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "foreign").unwrap(),
            42,
        )
        .unwrap_err();
        assert_eq!(err, DomainError::InvalidConfirmedOutcome);
    }

    #[test]
    fn confirmed_outcome_rejects_an_unknown_schema_version() {
        let err = ConfirmedOutcome::new(
            99,
            ExecutionId::parse("execution-1").unwrap(),
            AttemptId::parse("attempt-1").unwrap(),
            &spec(),
            Outcome::new(OUTCOME_SCHEMA_VERSION, OutcomeKind::Succeeded, "stub").unwrap(),
            42,
        )
        .unwrap_err();
        assert_eq!(err, DomainError::UnsupportedSchemaVersion(99));
    }
}
