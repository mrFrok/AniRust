{
  description = "AniRust, an Anixart desktop client with an mpv player";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;
    in {
      packages = nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" ] (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          windowLibraries = with pkgs; [
            libGL libxkbcommon wayland libx11 libxcursor libxi libxrandr libxcb
          ];
          anirust = pkgs.rustPlatform.buildRustPackage {
            pname = "anirust";
            inherit version;
            src = self;
            cargoLock.lockFile = ./Cargo.lock;
            cargoBuildFlags = [ "-p" "anirust-gui" "-p" "anirust-cli" ];
            nativeBuildInputs = with pkgs; [ pkg-config cmake makeWrapper ];
            buildInputs = with pkgs; [ mpv-unwrapped fontconfig libxkbcommon ];

            postInstall = ''
              install -Dm644 packaging/linux/io.github.mrfrok.AniRust.desktop \
                "$out/share/applications/io.github.mrfrok.AniRust.desktop"
              install -Dm644 packaging/icons/anirust.svg \
                "$out/share/icons/hicolor/scalable/apps/io.github.mrfrok.AniRust.svg"
              wrapProgram "$out/bin/anirust" \
                --run 'export ANIRUST_LAUNCHER="$0"' \
                --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath windowLibraries} \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.ffmpeg ]}
              wrapProgram "$out/bin/anirust-cli" \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.ffmpeg ]}
            '';

            meta = with pkgs.lib; {
              description = "Desktop client for Anixart with an mpv player";
              homepage = "https://github.com/mrFrok/AniRust";
              license = licenses.gpl3Plus;
              mainProgram = "anirust";
              platforms = platforms.linux;
            };
          };
        in {
          inherit anirust;
          default = anirust;
        });
    };
}
