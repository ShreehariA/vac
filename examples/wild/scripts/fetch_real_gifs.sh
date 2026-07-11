#!/usr/bin/env bash
# Re-download the five Wikimedia-Commons GIFs used for the v0.4.1
# "real-world calling card" matrix.  All five are CC-licensed; see
# `examples/wild/README.md` for per-file attribution and licences.

set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p inputs

UA="vac-decompiler/0.4.1 (educational tests)"
for spec in \
  "real_pendulum.gif|AnimatedPendulum.gif" \
  "real_bouncing_ball.gif|Bouncing_ball.gif" \
  "real_newtons_cradle.gif|Newtons_cradle_animation_book.gif" \
  "real_double_pendulum.gif|Demonstrating_Chaos_with_a_Double_Pendulum.gif" \
  "real_loader.gif|Loading_2_transparent.gif"
do
  local_name="${spec%%|*}"
  remote="${spec##*|}"
  url="https://commons.wikimedia.org/wiki/Special:FilePath/${remote}"
  printf '  %-34s <- %s\n' "$local_name" "$remote"
  curl -fsSL -A "$UA" --max-time 60 -o "inputs/${local_name}" "$url"
done

echo
echo "Downloaded to examples/wild/inputs/:"
ls -lh inputs/real_*.gif
