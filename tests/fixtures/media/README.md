# Original metadata fixture

`metadata.mp4` is an original synthetic fixture generated for this repository. It
contains one second of solid blue 64×48 video (25 fps, MPEG-4 Visual), a 440 Hz
mono tone (AAC), title `VidLib fixture`, audio language `eng` (MediaInfo normalizes
it to `en`), and recording date `2024-02-03 04:05:06` with **no timezone**.
No personal or third-party media were used. The fixture and its contents are
released under CC0-1.0: https://creativecommons.org/publicdomain/zero/1.0/.

Generation (FFmpeg 7.1, a developer tool only):

```sh
ffmpeg -f lavfi -i color=c=blue:s=64x48:r=25:d=1 \
  -f lavfi -i sine=frequency=440:sample_rate=48000:duration=1 \
  -c:v mpeg4 -c:a aac -metadata title='VidLib fixture' \
  -metadata date='2024-02-03 04:05:06' -metadata:s:a:0 language=eng \
  -shortest metadata.mp4
```

The recording date deliberately exercises civil-time preservation. File-system
creation/modification times are not recording-date fallbacks. Integration tests
use the packaged 25.10 Windows x64 helper on Windows. Linux development tests
explicitly inject `/usr/bin/mediainfo` 25.04; that binary is not a product fallback
and those tests do not qualify the Windows executable or installer.

Vendoring: `python3 tools/vendor_media_info.py --check` verifies committed hashes,
notices, PE architecture and Tauri resource mapping completely offline.
`--fetch` is an explicit developer/build operation using pinned HTTPS URLs;
`--archive /path/to/MediaInfo_CLI_25.10_Windows_x64.zip` accepts the exact pinned
archive. The application never invokes this script or downloads helpers.
