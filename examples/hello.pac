canvas 320x200
background #1a1a2e

# Three shapes drawn back-to-front.  The backdrop uses a v0.2
# linear-gradient fill; the curved blob is a cubic-Bezier path.

let backdrop = polygon(20 20, 300 20, 300 180, 20 180)
backdrop.fill = gradient(linear, #2c2c54 at 20,20, #4a4e89 at 300,180)

let diamond = polygon(160 50, 240 100, 160 150, 80 100)
diamond.fill = #e94560

let blob = path(M 90,90 C 100,70 140,70 150,90 C 160,110 140,130 120,130 C 100,130 80,110 90,90)
blob.fill = #ffd166
