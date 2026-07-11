#!/usr/bin/env bash
# Fetch a few CC-licensed AI-generated images from Wikimedia Commons to
# stress-test VAC's existing blob detector on AI content.  All four
# images are tagged {{PD-algorithm}} on Commons (US copyright doesn't
# protect purely algorithmic output) and are kept here only as
# read-only test fixtures — they are not redistributed.

set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p inputs

UA="vac-decompiler/0.4.1 (educational tests)"

# Map: <local filename> | <Commons filename>
#
# Picked so the four span the structural→photographic spectrum the same
# way the original capability matrix did, but with AI provenance:
#   1) stylised flat / illustration AI — should round-trip cleanly
#   2) watercolour / painterly AI — medium decomposability
#   3) AI-enhanced or generated portrait — photorealistic, predicted failure
#   4) AI scene / landscape — photorealistic, predicted failure
for spec in \
  "ai_flat_illustration.png|Midjourney_artificial_intelligence_art.png" \
  "ai_watercolor.png|Vincent_van_Gogh_in_watercolour.png" \
  "ai_photorealistic_portrait.jpg|Mona_Lisa_2024.jpg" \
  "ai_dream_machine.png|A_machine_that_reads_dreams_and_translates_them_into_images_-_by_Midjourney.png"
do
  local_name="${spec%%|*}"
  remote="${spec##*|}"
  url="https://commons.wikimedia.org/wiki/Special:FilePath/${remote}"
  printf '  %-38s <- %s\n' "$local_name" "$remote"
  if curl -fsSL -A "$UA" --max-time 90 -o "inputs/${local_name}" "$url"; then
    printf '    %s\n' "$(du -h "inputs/${local_name}" | cut -f1)"
  else
    printf '    FAILED\n'
  fi
done

echo
echo "Downloaded:"
ls -lh inputs/ai_*.* 2>/dev/null || echo "  none"
