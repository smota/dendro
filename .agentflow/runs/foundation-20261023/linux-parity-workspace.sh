#!/usr/bin/env bash
set -eu
umask 077
scratch=/home/sam/code/dendro-foundation-parity-20261023
mkdir -p "$scratch"
rsync -a --exclude .git --exclude target --exclude node_modules --exclude .claude --exclude .codex --exclude .grok --exclude .gemini --exclude .agentflow/runs ./ "$scratch/"
cd "$scratch"
export CARGO_TARGET_DIR=/home/sam/code/dendro/target
export CARGO_BUILD_JOBS=4
cargo check --workspace --locked > /home/sam/dendro-foundation-linux-parity-20261023/workspace-check.log 2>&1
cat /home/sam/dendro-foundation-linux-parity-20261023/workspace-check.log
