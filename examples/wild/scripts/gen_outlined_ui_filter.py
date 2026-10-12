#!/usr/bin/env python3
"""Emit an ffmpeg filtergraph for an outlined-UI animation (v1.4 corpus clip).

Strokes, drawn by ffmpeg rather than VAC's own renderer:
- an outlined box (4 px cyan outline, no fill) slides right along the top;
- a red card with a 3 px white border slides left along the bottom;
- a static outlined square (2 px yellow) sits in the middle.
"""
W, H = 480, 320
FPS = 30
DURATION = 3.0
N_FRAMES = int(FPS * DURATION)

filters = [
    # The static square, every frame.
    "drawbox=x=210:y=130:w=60:h=60:color=0xf0c040:t=2",
]
for n in range(N_FRAMES):
    t = n / FPS
    ox = int(20 + 300 * t / DURATION)          # outlined box: 20 -> 320
    cx = int(300 - 260 * t / DURATION)         # card: 300 -> 40
    filters.append(f"drawbox=x={ox}:y=30:w=140:h=80:color=0x40c0ff:t=4:enable='eq(n,{n})'")
    filters.append(f"drawbox=x={cx}:y=220:w=150:h=70:color=0xff5050:t=fill:enable='eq(n,{n})'")
    filters.append(f"drawbox=x={cx}:y=220:w=150:h=70:color=0xffffff:t=3:enable='eq(n,{n})'")

print(",\n".join(filters))
