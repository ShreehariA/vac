#!/usr/bin/env python3
"""Emit an ffmpeg filtergraph that animates three ping-pong rectangles.

Each shape gets one `drawbox` filter per frame, timeline-gated with
`enable='eq(n,N)'`, so position is baked at filter graph build time
instead of relying on the missing per-frame eval mode in ffmpeg 8.x.
"""
import sys

W, H = 640, 360
FPS = 30
DURATION = 3.0
N_FRAMES = int(FPS * DURATION)

# Shapes: (color, y, phase)
# phase=0 -> goes left->right->left
# phase=1 -> goes right->left->right
shapes = [
    ("red",      80,  0),
    ("yellow",  160,  1),
    ("0x40c040", 240, 0),
]

X_LEFT, X_RIGHT = 60, 540
BOX_W, BOX_H = 44, 44
HALF = DURATION / 2  # apex at 1.5s

filters = []
for color, y, phase in shapes:
    for n in range(N_FRAMES):
        t = n / FPS
        # linear ping-pong; phase=0 starts at LEFT, phase=1 starts at RIGHT
        if t < HALF:
            f = t / HALF
        else:
            f = 1.0 - (t - HALF) / HALF
        if phase == 0:
            x = int(X_LEFT + (X_RIGHT - X_LEFT) * f)
        else:
            x = int(X_RIGHT - (X_RIGHT - X_LEFT) * f)
        filters.append(
            f"drawbox=x={x}:y={y}:w={BOX_W}:h={BOX_H}:color={color}:t=fill:enable='eq(n,{n})'"
        )

print(",\n".join(filters))
