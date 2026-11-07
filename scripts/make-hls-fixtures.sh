#!/usr/bin/env bash
# Regenerates the small HLS fixtures used by crates/core/tests/hls_download.rs.
# The generated files are committed; this script documents exactly how they were made.
# Requires ffmpeg. The picture is a 64x48 test pattern and the sound a sine tone: 6 seconds total.
set -euo pipefail
cd "$(dirname "$0")/.."
out=crates/core/tests/fixtures/hls
rm -rf "$out"
mkdir -p "$out"/{ts,fmp4,aes,range,rangemp4}
src=(-f lavfi -i "testsrc=size=64x48:rate=10:duration=6" -f lavfi -i "sine=frequency=440:duration=6")
enc=(-c:v libx264 -preset ultrafast -g 10 -keyint_min 10 -sc_threshold 0 -pix_fmt yuv420p -c:a aac -b:a 32k)

# 1. Muxed MPEG-TS, three 2 s segments.
ffmpeg -v error -y "${src[@]}" "${enc[@]}" -f hls -hls_time 2 -hls_playlist_type vod \
  -hls_segment_filename "$out/ts/seg%d.ts" "$out/ts/media.m3u8"

# 2. Fragmented MP4 with a separate audio rendition (own init segments).
ffmpeg -v error -y "${src[@]}" -map 0:v -map 1:a "${enc[@]}" -f hls -hls_time 2 -hls_playlist_type vod \
  -hls_segment_type fmp4 -var_stream_map "v:0,agroup:aud a:0,agroup:aud,default:yes,language:eng,name:English" \
  -master_pl_name master.m3u8 -hls_segment_filename "$out/fmp4/s%v_%d.m4s" "$out/fmp4/p%v.m3u8"

# 3. AES-128 encrypted MPEG-TS. The key is a fixed test value, not a secret.
printf '\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f' > "$out/aes/key.bin"
printf 'key.bin\n%s\n' "$out/aes/key.bin" > "$out/aes/keyinfo"
ffmpeg -v error -y "${src[@]}" "${enc[@]}" -f hls -hls_time 2 -hls_playlist_type vod \
  -hls_key_info_file "$out/aes/keyinfo" -hls_segment_filename "$out/aes/seg%d.ts" "$out/aes/media.m3u8"
rm "$out/aes/keyinfo"

# 4. One MPEG-TS file addressed with EXT-X-BYTERANGE.
ffmpeg -v error -y "${src[@]}" "${enc[@]}" -f hls -hls_time 2 -hls_playlist_type vod -hls_flags single_file \
  -hls_segment_filename "$out/range/media.ts" "$out/range/media.m3u8"

# 5. One fragmented MP4 file: an init byte range plus media byte ranges.
ffmpeg -v error -y "${src[@]}" "${enc[@]}" -f hls -hls_time 2 -hls_playlist_type vod -hls_flags single_file \
  -hls_segment_type fmp4 -hls_fmp4_init_filename init.mp4 -hls_segment_filename "$out/rangemp4/media.mp4" "$out/rangemp4/media.m3u8"

# 3b. AES-128 without an explicit IV: each segment's IV is its media sequence number (here 5, 6, 7).
mkdir -p "$out/aesseq"
cp "$out/aes/key.bin" "$out/aesseq/key.bin"
for n in 0 1 2; do
  iv=$(printf '%032x' $((n + 5)))
  openssl enc -aes-128-cbc -K 000102030405060708090a0b0c0d0e0f -iv "$iv" -in "$out/ts/seg$n.ts" -out "$out/aesseq/seg$n.ts"
done
printf '#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n#EXT-X-MEDIA-SEQUENCE:5\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-KEY:METHOD=AES-128,URI="key.bin"\n#EXTINF:2.000000,\nseg0.ts\n#EXTINF:2.000000,\nseg1.ts\n#EXTINF:2.000000,\nseg2.ts\n#EXT-X-ENDLIST\n' > "$out/aesseq/media.m3u8"

# 6. Subtitles (WebVTT, hand written) used by the master playlist test.
mkdir -p "$out/subs"
printf 'WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n00:00:00.500 --> 00:00:01.500\nHello offline\n\n00:00:03.000 --> 00:00:04.000\nSecond cue\n' > "$out/subs/en0.vtt"
printf 'WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n\n00:00:00.500 --> 00:00:01.500\nHallo offline\n' > "$out/subs/de0.vtt"
for lang in en de; do
  printf '#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:6\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:6.000000,\n%s0.vtt\n#EXT-X-ENDLIST\n' "$lang" > "$out/subs/$lang.m3u8"
done
printf '1\n00:00:00,500 --> 00:00:01,500\nProvider subtitle\n' > "$out/subs/provider.srt"
echo "Fixtures written to $out"; du -sh "$out"
