#!/usr/bin/env bash
# Regenerates the ClearKey (CENC, AES-CTR) fixture used by crates/core/tests/clearkey_playback.rs.
# The key and key ID below are public test values made up for this fixture; nothing real is protected.
# Picture: 64x48 test pattern. Sound: a 440 Hz sine, which the test listens for to prove decryption.
set -euo pipefail
cd "$(dirname "$0")/.."
out=crates/core/tests/fixtures/drm
mkdir -p "$out"
KEY=00112233445566778899aabbccddeeff
KID=a1b2c3d4e5f60718293a4b5c6d7e8f90
ffmpeg -v error -y -f lavfi -i "testsrc=size=64x48:rate=10:duration=4" -f lavfi -i "sine=frequency=440:duration=4" \
  -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac -b:a 32k \
  -encryption_scheme cenc-aes-ctr -encryption_key "$KEY" -encryption_kid "$KID" -movflags +faststart "$out/clearkey.mp4"
echo "Wrote $out/clearkey.mp4 (key $KEY, key ID $KID)"
