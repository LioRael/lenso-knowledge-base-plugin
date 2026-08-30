# Release process

Releases are produced only from a clean `main` branch by
`.github/workflows/release-plz.yml`. The workflow uses GitHub OIDC Trusted
Publishing and deliberately has no crates.io token fallback.

Before enabling publication, allocate `lenso-capability-knowledge-base` once on
crates.io and configure Trusted Publishing for owner `LioRael`, repository
`lenso-knowledge-base-plugin`, workflow `release-plz.yml`, with no GitHub
environment restriction.

The PostgreSQL provider is currently a linked package and remains
`publish = false`. A clean source checkout resolves Auth SDK, Access Control,
Organization Membership, Search, and Search Index through immutable Git
coordinates. Crates.io publication remains deferred until those exact
contracts are available as registry dependencies; replace the Git coordinates,
regenerate `Cargo.lock`, and rerun every gate before enabling publication.
