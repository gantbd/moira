.PHONY: check check-peer-profiles check-release-contract check-compatibility-baseline check-physical-integrity check-supply-chain check-advisory-attestation check-assurance-mutants check-sdk-boundaries fmt lint test doc soak soak-process soak-hermes-process soak-live-execution performance build-live-execution-helpers qualify-aegis-process qualify-agent-local qualify-agent-reattach-recovery qualify-agora-local-port qualify-agora-recovery qualify-chronos-recovery qualify-daimon-process qualify-hermes-process qualify-live-execution-chain qualify-methodical-agent-live-provider qualify-methodical-chatbot-live-provider qualify-methodical-chatbot-peers qualify-methodical-peers qualify-methodical-profiles qualify-methodical-tools qualify-methodical-workflow-live-provider qualify-methodical-workflow-peer-live-provider qualify-vellum-process

check: check-release-contract check-compatibility-baseline check-supply-chain check-advisory-attestation check-assurance-mutants check-sdk-boundaries fmt lint test doc

check-peer-profiles: qualify-aegis-process qualify-agent-local qualify-agora-local-port qualify-agora-recovery qualify-chronos-recovery qualify-daimon-process qualify-hermes-process qualify-live-execution-chain qualify-methodical-peers qualify-vellum-process

check-release-contract:
	bash scripts/check-release-contract.sh

check-compatibility-baseline:
	bash scripts/check-compatibility-baseline.sh

check-physical-integrity:
	cargo test -p moira-sqlite --test physical_integrity --locked --offline

check-supply-chain:
	bash scripts/check-supply-chain.sh

check-advisory-attestation:
	bash scripts/check-advisory-attestation.sh

check-assurance-mutants:
	bash scripts/check-assurance-mutants.sh

check-sdk-boundaries:
	./scripts/check-sdk-boundaries.sh

fmt:
	cargo fmt -p moira-core -p moira-sqlite -p moira-agora-sqlite -p moira-execution-sqlite -p moira-aegis -p moira-agora -p moira-agora-local -p moira-chronos -p moira-daimon -p moira-dendrite -p moira-hermes -p moira-rune-validator -p moira-unix-peer -p moira-methodical-local -p moira-agent-local -p moira-client -p moira-host -p moira-local-daemon -p moira-local-harness -p moira-protocol -p moira-service -p moira-wire -p moira-sdk-embedded -- --check

lint:
	cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings

test:
	cargo test --workspace --all-targets --locked --offline

doc:
	cargo test --workspace --doc --locked --offline
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline

soak:
	cargo test -p moira-service --test saga_service_behavior five_thousand_saga_lifecycles --locked --offline -- --ignored --nocapture

soak-process:
	cargo test --manifest-path qualification/aegis-moira-process/Cargo.toml --test federation_process sixteen_federation_process_cycles --locked --offline -- --ignored --nocapture

soak-hermes-process:
	cargo test --manifest-path qualification/hermes-process-delivery/Cargo.toml --test hermes_process sixteen_hermes_delivery_process_cycles --locked --offline -- --ignored --nocapture

soak-live-execution: build-live-execution-helpers
	AEGIS_CHAIN_HELPER='$(abspath ../aegis/target/debug/examples/chain_daemon)' CHRONOS_CHAIN_HELPER='$(abspath ../daimon/qualification/chronos-attempt/target/debug/examples/running_daemon)' cargo test --manifest-path qualification/live-execution-chain/Cargo.toml --test live_chain sixteen_live_execution_process_cycles --locked --offline -- --ignored --nocapture

performance:
	rustc -Vv
	uname -srm
	cargo test -p moira-service --test saga_service_behavior local_candidate_performance_gate --release --locked --offline -- --ignored --nocapture

qualify-aegis-process:
	cargo fmt --manifest-path qualification/aegis-moira-process/Cargo.toml -- --check
	cargo test --manifest-path qualification/aegis-moira-process/Cargo.toml --all-targets --locked --offline

qualify-agent-local:
	cargo fmt -p moira-agent-local -- --check
	cargo clippy -p moira-agent-local --all-targets --locked --offline -- -D warnings
	cargo test -p moira-agent-local --all-targets --locked --offline

qualify-agent-reattach-recovery:
	cargo test --manifest-path ../daimon/Cargo.toml -p daimon-local-agent --test process_journey executable_restarts_idle_acp_session_and_requires_explicit_reattach --locked --offline -- --exact --nocapture
	DAIMON_LOCAL_AGENT_PROGRAM='$(abspath ../daimon/target/debug/daimon-local-agent)' cargo test -p moira-agent-local --test combined_reattach_process moira_recovers_through_real_daimon_process_kill_and_explicit_confirmation --locked --offline -- --ignored --exact --nocapture
	cargo test -p moira-agent-local --test real_process_journey executable_discovers_confirms_and_dispatches_governed_reattach --locked --offline -- --exact --nocapture
	cargo test --manifest-path ../moira-tui/Cargo.toml -p moira-tui-cli --test permission_confirmation cli_views_then_confirms_only_the_server_prepared_reattach --locked --offline -- --exact --nocapture

qualify-agora-local-port:
	cargo build --manifest-path ../aegis/Cargo.toml -p aegis-daemon --bin aegis-daemon --locked --offline
	cargo build --manifest-path ../agora/Cargo.toml -p agora-private-local --bin agora-private-local --locked --offline
	cargo fmt --manifest-path qualification/agora-local-port/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/agora-local-port/Cargo.toml --target-dir target/qualification-agora-local-port --all-targets --locked --offline -- -D warnings
	MOIRA_AEGIS_DAEMON_PROGRAM='$(abspath ../aegis/target/debug/aegis-daemon)' MOIRA_AGORA_DAEMON_PROGRAM='$(abspath ../agora/target/debug/agora-private-local)' cargo test --manifest-path qualification/agora-local-port/Cargo.toml --target-dir target/qualification-agora-local-port --all-targets --locked --offline

qualify-agora-recovery:
	cargo fmt --manifest-path qualification/agora-service-recovery/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/agora-service-recovery/Cargo.toml --all-targets --locked --offline -- -D warnings
	cargo test --manifest-path qualification/agora-service-recovery/Cargo.toml --all-targets --locked --offline

qualify-vellum-process:
	cargo build --manifest-path ../vellum/Cargo.toml -p vellum-daemon --bin vellum-daemon --locked --offline
	cargo fmt --manifest-path qualification/vellum-process-publication/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/vellum-process-publication/Cargo.toml --target-dir target/qualification-vellum-process --all-targets --locked --offline -- -D warnings
	MOIRA_VELLUM_DAEMON_PROGRAM='$(abspath ../vellum/target/debug/vellum-daemon)' cargo test --manifest-path qualification/vellum-process-publication/Cargo.toml --target-dir target/qualification-vellum-process --all-targets --locked --offline

qualify-chronos-recovery:
	cargo fmt --manifest-path qualification/chronos-service-recovery/Cargo.toml -- --check
	cargo test --manifest-path qualification/chronos-service-recovery/Cargo.toml --all-targets --locked --offline

qualify-daimon-process:
	cargo fmt --manifest-path qualification/daimon-process-execution/Cargo.toml -- --check
	cargo test --manifest-path qualification/daimon-process-execution/Cargo.toml --all-targets --locked --offline

qualify-hermes-process:
	cargo fmt --manifest-path qualification/hermes-process-delivery/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/hermes-process-delivery/Cargo.toml --all-targets --locked --offline -- -D warnings
	cargo test --manifest-path qualification/hermes-process-delivery/Cargo.toml --all-targets --locked --offline

build-live-execution-helpers:
	cargo build --manifest-path ../aegis/Cargo.toml -p aegis-local-daemon --example chain_daemon --locked --offline
	cargo build --manifest-path ../daimon/qualification/chronos-attempt/Cargo.toml --example running_daemon --locked --offline

qualify-live-execution-chain: build-live-execution-helpers
	cargo fmt --manifest-path qualification/live-execution-chain/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/live-execution-chain/Cargo.toml --all-targets --locked --offline -- -D warnings
	AEGIS_CHAIN_HELPER='$(abspath ../aegis/target/debug/examples/chain_daemon)' CHRONOS_CHAIN_HELPER='$(abspath ../daimon/qualification/chronos-attempt/target/debug/examples/running_daemon)' cargo test --manifest-path qualification/live-execution-chain/Cargo.toml --all-targets --locked --offline -- --test-threads=1

qualify-methodical-tools:
	@test -n "$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)" || (echo 'MOIRA_METHODICAL_LIVE_DENDRITE_PORT is required' >&2; exit 2)
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' cargo test -p moira-methodical-local tests::live_grounded_tool_turn_crosses_aegis_chronos_daimon_and_durable_resume --locked --offline -- --exact --nocapture

qualify-methodical-profiles:
	@test -n "$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)" || (echo 'MOIRA_METHODICAL_LIVE_DENDRITE_PORT is required' >&2; exit 2)
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' cargo test -p moira-methodical-local tests::live_grounded_chatbot_profile_uses_one_authored_expert_and_private_model_socket --locked --offline -- --exact --nocapture
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' cargo test -p moira-methodical-local tests::live_real_corpus_to_private_model_socket_to_sqlite_journey --locked --offline -- --exact --nocapture
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' cargo test -p moira-methodical-local tests::live_grounded_tool_turn_crosses_aegis_chronos_daimon_and_durable_resume --locked --offline -- --exact --nocapture

qualify-methodical-chatbot-peers:
	@test -n "$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)" || (echo 'MOIRA_METHODICAL_LIVE_DENDRITE_PORT is required' >&2; exit 2)
	cargo fmt --manifest-path qualification/methodical-peer-live/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/methodical-peer-live/Cargo.toml --all-targets --locked --offline -- -D warnings
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' MOIRA_PEER_VELLUM_CLAIM=31313131313131313131313131313131 cargo test --manifest-path qualification/methodical-peer-live/Cargo.toml tests::grounded_chatbot_operator_crosses_actual_daimon_agora_and_vellum --locked --offline -- --exact --nocapture --test-threads=1

qualify-methodical-chatbot-live-provider:
	@test -n "$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)" || (echo 'MOIRA_METHODICAL_LIVE_DENDRITE_PORT is required' >&2; exit 2)
	@test -n "$(MOIRA_METHODICAL_LIVE_MODEL)" || (echo 'MOIRA_METHODICAL_LIVE_MODEL is required' >&2; exit 2)
	cargo fmt --manifest-path qualification/methodical-peer-live/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/methodical-peer-live/Cargo.toml --all-targets --locked --offline -- -D warnings
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' MOIRA_METHODICAL_LIVE_MODEL='$(MOIRA_METHODICAL_LIVE_MODEL)' MOIRA_METHODICAL_LIVE_MODEL_ADDRESS='$(or $(MOIRA_METHODICAL_LIVE_MODEL_ADDRESS),127.0.0.1:11434)' MOIRA_PEER_VELLUM_CLAIM=31313131313131313131313131313131 cargo test --manifest-path qualification/methodical-peer-live/Cargo.toml tests::grounded_chatbot_operator_crosses_actual_daimon_agora_and_vellum --locked --offline -- --exact --nocapture --test-threads=1

qualify-methodical-workflow-live-provider:
	@test -n "$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)" || (echo 'MOIRA_METHODICAL_LIVE_DENDRITE_PORT is required' >&2; exit 2)
	@test -n "$(MOIRA_METHODICAL_LIVE_MODEL)" || (echo 'MOIRA_METHODICAL_LIVE_MODEL is required' >&2; exit 2)
	cargo fmt --manifest-path qualification/methodical-peer-live/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/methodical-peer-live/Cargo.toml --all-targets --locked --offline -- -D warnings
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' MOIRA_METHODICAL_LIVE_MODEL='$(MOIRA_METHODICAL_LIVE_MODEL)' MOIRA_METHODICAL_LIVE_MODEL_ADDRESS='$(or $(MOIRA_METHODICAL_LIVE_MODEL_ADDRESS),127.0.0.1:11434)' cargo test --manifest-path qualification/methodical-peer-live/Cargo.toml tests::grounded_workflow_ensemble_crosses_actual_live_provider --locked --offline -- --exact --nocapture --test-threads=1

qualify-methodical-workflow-peer-live-provider:
	@test -n "$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)" || (echo 'MOIRA_METHODICAL_LIVE_DENDRITE_PORT is required' >&2; exit 2)
	@test -n "$(MOIRA_METHODICAL_LIVE_MODEL)" || (echo 'MOIRA_METHODICAL_LIVE_MODEL is required' >&2; exit 2)
	cargo fmt --manifest-path qualification/methodical-peer-live/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/methodical-peer-live/Cargo.toml --all-targets --locked --offline -- -D warnings
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' MOIRA_METHODICAL_LIVE_MODEL='$(MOIRA_METHODICAL_LIVE_MODEL)' MOIRA_METHODICAL_LIVE_MODEL_ADDRESS='$(or $(MOIRA_METHODICAL_LIVE_MODEL_ADDRESS),127.0.0.1:11434)' MOIRA_PEER_VELLUM_CLAIM=31313131313131313131313131313131 cargo test --manifest-path qualification/methodical-peer-live/Cargo.toml tests::authorized_live_provider_workflow_crosses_distinct_aegis_agora_and_vellum --locked --offline -- --exact --nocapture --test-threads=1

qualify-methodical-agent-live-provider:
	@test -n "$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)" || (echo 'MOIRA_METHODICAL_LIVE_DENDRITE_PORT is required' >&2; exit 2)
	@test -n "$(MOIRA_METHODICAL_LIVE_MODEL)" || (echo 'MOIRA_METHODICAL_LIVE_MODEL is required' >&2; exit 2)
	cargo fmt --manifest-path qualification/methodical-peer-live/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/methodical-peer-live/Cargo.toml --all-targets --locked --offline -- -D warnings
	MOIRA_METHODICAL_LIVE_DENDRITE_PORT='$(MOIRA_METHODICAL_LIVE_DENDRITE_PORT)' MOIRA_METHODICAL_LIVE_MODEL='$(MOIRA_METHODICAL_LIVE_MODEL)' MOIRA_METHODICAL_LIVE_MODEL_ADDRESS='$(or $(MOIRA_METHODICAL_LIVE_MODEL_ADDRESS),127.0.0.1:11434)' cargo test --manifest-path qualification/methodical-peer-live/Cargo.toml tests::grounded_agent_native_tool_turn_crosses_actual_live_provider --locked --offline -- --exact --nocapture --test-threads=1

qualify-methodical-peers:
	cargo fmt --manifest-path qualification/methodical-peer-live/Cargo.toml -- --check
	cargo clippy --manifest-path qualification/methodical-peer-live/Cargo.toml --all-targets --locked --offline -- -D warnings
	MOIRA_PEER_VELLUM_CLAIM=31313131313131313131313131313131 cargo test --manifest-path qualification/methodical-peer-live/Cargo.toml --all-targets --locked --offline -- --test-threads=1

