#!/usr/bin/env python3
"""Emit an ffmpeg filtergraph for the slide-in-card UI animation.

Phase 1 (0..1s): card x slides from -180 to 80 (linear, ease-in placeholder).
Phase 2 (1..2s): card holds at x=80.
Phase 3 (2..3s): card x slides from 80 to 480 (off-screen right).
"""
W, H = 480, 320
FPS = 30
DURATION = 3.0
N_FRAMES = int(FPS * DURATION)
BOX_W, BOX_H = 180, 60
Y = 120
COLOR = "0xff5050"

filters = []
for n in range(N_FRAMES):
    t = n / FPS
    if t < 1.0:
        x = int(-180 + 260 * t)
    elif t < 2.0:
        x = 80
    else:
        x = int(80 + 400 * (t - 2.0))
    filters.append(
        f"drawbox=x={x}:y={Y}:w={BOX_W}:h={BOX_H}:color={COLOR}:t=fill:enable='eq(n,{n})'"
    )

print(",\n".join(filters))
