# Synthetic MPEG fixture

`video_overlay.mpg` contains 32×24 red MPEG-1 video at 25 fps and a short
mono MP2 tone. No game assets are included. The original fixture used a
384 kbit/s mono MP2 stream; Symphonia rejects that invalid Layer II bitrate
and channel combination. Its video packets were retained and its audio was
re-encoded to 128 kbit/s for the built-in decoder tests:

```sh
ffmpeg -i original-video_overlay.mpg -c:v copy -c:a mp2 -b:a 128k -f mpeg video_overlay.mpg
```

This command documents one-time fixture preparation only. Builds, tests and
movie playback do not launch or require FFmpeg. The integration tests run
playback in a child process with an empty PATH to verify that requirement.
