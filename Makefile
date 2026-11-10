.PHONY: check check-sdk-boundaries fmt lint test doc

check: check-sdk-boundaries fmt lint test doc

check-sdk-boundaries:
	./scripts/check-sdk-boundaries.sh

fmt:
	cargo fmt --all -- --check

lint:
	cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings

test:
	cargo test --workspace --all-targets --locked --offline

doc:
	cargo test --workspace --doc --locked --offline
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked --offline

