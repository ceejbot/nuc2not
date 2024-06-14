_help:
	just -l

# Run tests using nextest
test:
	cargo nextest run

# Format using nightly
fmt:
	cargo +nightly fmt

# Install required tools
setup:
	brew tap ceejbot/tap
	brew install fzf tomato cargo-nextest
	rustup install nightly

# Tag a new version for release.
tag VERSION:
	#!/usr/bin/env bash
	set -e
	version="{{VERSION}}"
	version=${version/v/}
	tomato set package.version "$version" Cargo.toml
	# update the lock file
	cargo check
	git commit Cargo.toml Cargo.lock -m "${version}"
	git tag "${version}"
	echo "Release tagged for version ${version}"
