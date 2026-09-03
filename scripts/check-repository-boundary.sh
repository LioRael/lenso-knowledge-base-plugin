#!/usr/bin/env bash
set -euo pipefail

expected_crates=$'lenso-capability-knowledge-base\nlenso-knowledge-base-agent-tools-plugin\nlenso-knowledge-base-postgres-plugin'
actual_crates="$({
  find crates -mindepth 1 -maxdepth 1 -type d -exec basename {} \;
} | LC_ALL=C sort)"

if [[ "$actual_crates" != "$expected_crates" ]]; then
  printf 'unexpected crate ownership:\n%s\n' "$actual_crates" >&2
  exit 1
fi

expected_path_dependencies=$'./crates/lenso-knowledge-base-agent-tools-plugin/Cargo.toml:path = "../lenso-capability-knowledge-base"\n./crates/lenso-knowledge-base-postgres-plugin/Cargo.toml:path = "../lenso-capability-knowledge-base"'
actual_path_dependencies="$(rg --no-heading --with-filename -o 'path\s*=\s*"[^"]+"' --glob 'Cargo.toml' . | sort)"

if [[ "$actual_path_dependencies" != "$expected_path_dependencies" ]]; then
  printf 'unexpected path dependency boundary\n' >&2
  diff -u \
    <(printf '%s\n' "$expected_path_dependencies") \
    <(printf '%s\n' "$actual_path_dependencies") || true
  exit 1
fi

# Immutable remote suite dependencies may not introduce a second Kernel,
# runtime, or protocol source.
cargo_bin="${LENSO_CARGO_BIN:-cargo}"
metadata="$($cargo_bin metadata --locked --format-version=1)"

single_source_packages=(
  lenso
  lenso-app-plan
  lenso-kernel
  lenso-native-adapter
  lenso-native-adapter-macros
  lenso-runtime-codec
  lenso-contract-authoring
  lenso-contract-authoring-macros
  lenso-contract-codegen
  lenso-contract-runtime
  lenso-plugin-authoring
)
for package in "${single_source_packages[@]}"; do
  package_count="$(jq --arg package "$package" '[.packages[] | select(.name == $package)] | length' <<<"$metadata")"
  if [[ "$package_count" != "1" ]]; then
    printf '%s resolved %s times; exactly one runtime/protocol source is required\n' \
      "$package" "$package_count" >&2
    exit 1
  fi
done

for source_family in \
  'lenso,lenso-native-adapter,lenso-native-adapter-macros,lenso-runtime-codec' \
  'lenso-app-plan,lenso-kernel' \
  'lenso-contract-authoring,lenso-contract-authoring-macros,lenso-contract-codegen,lenso-contract-runtime,lenso-plugin-authoring'; do
  source_count="$(
    jq --arg family "$source_family" '
      ($family | split(",")) as $names
      | [.packages[] | select(.name as $name | $names | index($name)) | .source]
      | unique
      | length
    ' <<<"$metadata"
  )"
  if [[ "$source_count" != "1" ]]; then
    printf 'runtime/protocol family resolved from %s sources: %s\n' \
      "$source_count" "$source_family" >&2
    exit 1
  fi
done

if rg -n \
  'sqlx|postgres|lenso-postgres-kit|lenso-capability-secrets|lenso-capability-access-control|lenso-capability-search' \
  crates/lenso-capability-knowledge-base/Cargo.toml \
  crates/lenso-capability-knowledge-base/src \
  --glob '!**/generated.rs'; then
  printf 'portable Knowledge Base Capability gained an implementation dependency\n' >&2
  exit 1
fi

if rg -n 'HashMap|Mutex<.*Vec|in.memory|memory fallback' crates --glob '*.rs'; then
  printf 'ambient in-memory durable state is not allowed\n' >&2
  exit 1
fi

if rg -n 'lenso-platform-|lenso-module-|HostBuilder|HostLinkedModule|ModuleManifest' \
  Cargo.toml crates README.md docs --glob '!**/generated.rs'; then
  printf 'legacy Lenso framework dependency or API found\n' >&2
  exit 1
fi

for capability in \
  'lenso.knowledge-base@1' \
  'lenso.secrets@1' \
  'lenso.organization-membership@1' \
  'lenso.access-control@1' \
  'lenso.search@1' \
  'lenso.search-index@1'; do
  if ! rg -q "$capability" README.md docs crates; then
    printf 'documented Capability is missing: %s\n' "$capability" >&2
    exit 1
  fi
done

if rg -n 'lenso-capability-(data-export-source|retention-participant)' Cargo.toml crates; then
  printf 'subject-scoped privacy Capability was added without a matching contract\n' >&2
  exit 1
fi

printf 'repository boundary is Knowledge-Base-owned and Search remains rebuildable\n'
