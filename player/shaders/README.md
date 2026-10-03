# Upscaling shaders

The [Anime4K](https://github.com/bloc97/Anime4K) v4.0.1 GLSL shaders mpv runs
to upscale anime. They are vendored here unchanged and embedded in the binary,
which writes them to the cache directory on first use, so upscaling needs no
installation step. Anime4K is MIT-licensed (`LICENSE-Anime4K`), compatible with
this project's GPLv3.

## Presets

A preset is a mode and a quality. `player/src/shaders.rs` builds the chains:

- Anime4K's modes A, B, C, A+A, B+B and C+A follow the recipes in Anime4K's
  instructions for mpv. The quality picks the network sizes: Fast M then S,
  High L then M, Max VL then M (Anime4K's own high-end choice), Ultra a VL
  restore, a UL upscale, then L.
- Every Anime4K chain doubles twice, with the auto-downscale passes between the
  two doublings, so 720p reaches 4K through two network passes. Each pass
  checks the sizes involved and skips itself when it is not needed.

The tests in `shaders.rs` check the order — highlights clamped first, every
restore feeding a later upscale, the downscale between the two doublings — and
that every file a chain names is embedded and none is a UL restore.

## Left out

- `Restore_CNN_UL` and `Restore_CNN_Soft_UL` need more varying variables than
  OpenGL allows (31) and fail to link; mpv renders through OpenGL here.
- [ArtCNN](https://github.com/Artoriuz/ArtCNN) builds only under Vulkan, which
  libmpv's render API does not offer.

## Using other copies

`PlayerConfig::shader_dir` points the player at another directory; the probe
takes `--shader-dir` or `ANIRUST_SHADER_DIR`. `UpscalePreset::missing_from`
reports which files a directory lacks, and the probe then plays without
upscaling.
