# Distribution packages

A published stable GitHub Release starts the distribution workflow. For a
release that already exists, run **Distribution packages** with its tag. The
workflow uploads `distribution-v<version>` with the AUR source and binary
recipes, the Homebrew tap files and the validated WinGet manifests.

## Publishing

- **AUR** publishes `anirust` and `anirust-bin` from the latest stable release.
  Register a dedicated SSH public key in the maintainer's AUR account, save its
  private key as the repository secret `AUR_SSH_PRIVATE_KEY`, and set the
  repository variable `PUBLISH_AUR` to `true`. [AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines)
- **Homebrew** publishes a Linux formula and an Apple silicon cask to the
  [AniRust tap](https://github.com/mrFrok/homebrew-anirust). The tap's write
  deploy key is stored in `HOMEBREW_TAP_DEPLOY_KEY`. Set `PUBLISH_HOMEBREW` to
  `true` to update the tap after each stable release.
- **Nix** takes its version from the Cargo workspace and uses the committed
  `flake.lock`. With flakes enabled, run
  `nix profile install github:mrFrok/AniRust#anirust` on x86_64 or aarch64 Linux.
  Tags that include the flake can also be used as the profile input ref.
- **Flatpak** is built by the release workflow and shipped in GitHub Releases.
- **WinGet** uses the released x64 Inno Setup executable, its SHA-256 and the
  installer product code. To submit manually, run `winget validate --manifest
  <dir>` on Windows and open a pull request with the generated version directory
  in `microsoft/winget-pkgs`. To submit from Actions, set `PUBLISH_WINGET` to
  `true` and save a classic GitHub token with `public_repo` scope as
  `WINGET_CREATE_GITHUB_TOKEN`. Microsoft runs its own validation before the
  package becomes searchable as `mrFrok.AniRust`.
  [Microsoft submission guide](https://learn.microsoft.com/en-us/windows/package-manager/package/repository) · [WingetCreate token setup](https://github.com/microsoft/winget-create/blob/main/doc/token.md)
