#!/usr/bin/env bash
# regenerate .claude/skills/*/SKILL.md from .opencode/agents/*.md - opencode is the source of truth.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)

for src in "$root"/.opencode/agents/*.md; do
  name=$(basename "$src" .md)
  out="$root/.claude/skills/$name"
  mkdir -p "$out"
  {
    printf -- '---\nname: %s\n' "$name"
    grep -m1 '^description:' "$src"
    printf -- '---\n'
    # body = everything after the closing --- of the opencode frontmatter
    awk 'body { print } /^---$/ { if (++n == 2) body = 1 }' "$src"
  } > "$out/SKILL.md"
  echo "$out/SKILL.md"
done
