#!/usr/bin/env bash
set -eu
umask 077
scratch=/home/sam/dendro-foundation-linux-parity-20261023
mkdir -p "$scratch/src" "$scratch/tests" "$scratch/policy"
cp -r crates/dendro-platform-policy/src "$scratch/policy/"
cp crates/dendro-platform-policy/Cargo.toml "$scratch/policy/Cargo.toml"
cp -r crates/dendro-core/src/platform_v1 "$scratch/src/"
printf 'pub mod platform_v1;\n' > "$scratch/src/lib.rs"
cat > "$scratch/Cargo.toml" <<'EOF'
[package]
name="t24a-platform-harness"
version="0.1.0"
edition="2021"
[dependencies]
dendro-platform-policy={path="policy"}
serde={version="1",features=["derive"]}
serde_json="1"
sha2="0.10"
rand="0.8"
fd-lock="4"
rustix={version="1.1.5",default-features=false,features=["fs","std"]}
[workspace.package]
version="0.1.0"
edition="2021"
license="MIT"
authors=[]
[lints.rust]
unexpected_cfgs={level="warn",check-cfg=["cfg(dendro_platform_v080_harness)"]}
EOF
cp crates/dendro-core/tests/platform_v080.rs "$scratch/tests/"
cd "$scratch"
RUSTFLAGS='--cfg dendro_platform_v080_harness' cargo test --lib --no-default-features > primitive-test.log 2>&1
cat primitive-test.log
