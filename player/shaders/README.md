# Anime4K shaders

This directory is where the [Anime4K](https://github.com/bloc97/Anime4K) GLSL
shaders go. They are **not vendored** here; fetch them from upstream.

Anime4K is MIT-licensed, which is compatible with this project's GPLv3. mpv
loads the shaders natively through `glsl-shaders`, so there is nothing to
reimplement — a preset is an ordered list of files, and
`player/src/shaders.rs` holds those lists.

## Installing

Download `Anime4K_v4.0.zip` from the upstream releases page and extract the
`.glsl` files here, or point `PlayerConfig::shader_dir` at wherever you keep
them. The probe accepts `--shader-dir`, and also reads `ANIRUST_SHADER_DIR`.

## Files the presets need

| Preset | Shaders |
| --- | --- |
| Off | none |
| Fast | `Clamp_Highlights`, `Restore_CNN_S`, `Upscale_CNN_x2_S` |
| Balanced | `Clamp_Highlights`, `Restore_CNN_M`, `Upscale_CNN_x2_M`, `AutoDownscalePre_x2` |
| Quality | `Clamp_Highlights`, `Restore_CNN_L`, `Upscale_CNN_x2_L`, `AutoDownscalePre_x2`, `Restore_CNN_S` |

All names are prefixed `Anime4K_` and suffixed `.glsl`.

`UpscalePreset::missing_from` reports which of these a directory lacks, and the
player falls back to no upscaling rather than failing when an install is
incomplete.

## Order

`Clamp_Highlights` runs first so later passes see untouched highlights, and
restoration runs before upscaling. `player/src/shaders.rs` has tests asserting
both, so a careless edit to a chain fails the build rather than quietly
degrading the picture.
