#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/worktree-slice.sh create <issue-num> <slug> [prefix]
  scripts/worktree-slice.sh list
  scripts/worktree-slice.sh remove <issue-num-or-path>
EOF
}

cmd="${1:-}"
if [[ -z "$cmd" ]]; then
  usage >&2
  exit 1
fi

repo_root="$(git rev-parse --show-toplevel)"
worktree_root="${repo_root}/.worktrees"

case "$cmd" in
  create)
    issue_num="${2:-}"
    slug="${3:-}"
    prefix="${4:-feature}"
    if [[ -z "$issue_num" || -z "$slug" ]]; then
      usage >&2
      exit 1
    fi
    issue_num="${issue_num#issue-}"
    wt_dir="${worktree_root}/issue-${issue_num}-${slug}"
    branch_name="${prefix}/issue-${issue_num}-${slug}"

    mkdir -p "$worktree_root"

    base_ref="HEAD"
    if git show-ref --verify --quiet refs/heads/development; then
      base_ref="development"
    elif git show-ref --verify --quiet refs/remotes/origin/development; then
      base_ref="origin/development"
    fi

    if git show-ref --verify --quiet "refs/heads/${branch_name}"; then
      git worktree add "$wt_dir" "$branch_name"
    else
      git worktree add -b "$branch_name" "$wt_dir" "$base_ref"
    fi
    echo "Created worktree: ${wt_dir} (branch: ${branch_name}, base: ${base_ref})"
    ;;
  list)
    git worktree list
    ;;
  remove)
    target="${2:-}"
    if [[ -z "$target" ]]; then
      usage >&2
      exit 1
    fi
    if [[ -d "$target" ]]; then
      wt_path="$target"
    elif [[ -d "${worktree_root}/${target}" ]]; then
      wt_path="${worktree_root}/${target}"
    else
      num="${target#issue-}"
      matches=()
      if [[ -d "$worktree_root" ]]; then
        while IFS= read -r line; do
          [[ -n "$line" ]] && matches+=("$line")
        done < <(find "$worktree_root" -maxdepth 1 -mindepth 1 -type d -name "issue-${num}-*" | sort)
      fi
      if [[ "${#matches[@]}" -eq 1 ]]; then
        wt_path="${matches[0]}"
      elif [[ "${#matches[@]}" -gt 1 ]]; then
        echo "ERROR: Multiple worktrees match '${target}': ${matches[*]}" >&2
        exit 1
      else
        echo "ERROR: Worktree not found for '${target}'" >&2
        exit 1
      fi
    fi
    git worktree remove "$wt_path"
    echo "Removed worktree: ${wt_path}"
    ;;
  *)
    usage >&2
    exit 1
    ;;
esac
