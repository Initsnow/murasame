{
  description = "Murasame - AI Desktop Companion & Development Environment";

  inputs = {
    # nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    nixpkgs.url = "git+https://mirrors.tuna.tsinghua.edu.cn/git/nixpkgs.git?ref=nixos-26.05&shallow=1";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    let
      murasameVersion = "0.2.1";

      systemOutputs = flake-utils.lib.eachDefaultSystem (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};

          # Slint official prebuilt binaries from GitHub Releases (v1.18.1)
          slintVersion = "1.18.1";
          slintPrebuiltDeps = with pkgs; [
            stdenv.cc.cc.lib
            udev
            libinput
            libgbm
            fontconfig
            freetype
            libxkbcommon
            wayland
            libglvnd
          ];

          slint-viewer-bin = pkgs.stdenv.mkDerivation {
            pname = "slint-viewer";
            version = slintVersion;
            src = pkgs.fetchurl {
              url = "https://github.com/slint-ui/slint/releases/download/v${slintVersion}/slint-viewer-linux.tar.gz";
              hash = "sha256-hrJg5mHNwk3K2vYyZrA08ygcvsVCr8JlRacqmG/kKhM=";
            };
            nativeBuildInputs = [ pkgs.autoPatchelfHook ];
            buildInputs = slintPrebuiltDeps;
            sourceRoot = "slint-viewer";
            installPhase = ''
              mkdir -p $out/bin
              cp slint-viewer $out/bin/
              chmod +x $out/bin/slint-viewer
            '';
          };

          slint-lsp-bin = pkgs.stdenv.mkDerivation {
            pname = "slint-lsp";
            version = slintVersion;
            src = pkgs.fetchurl {
              url = "https://github.com/slint-ui/slint/releases/download/v${slintVersion}/slint-lsp-linux.tar.gz";
              hash = "sha256-YnNdKxkIroozs13Nxx/QxxP0TvikqbV+jfoHMZkD7vo=";
            };
            nativeBuildInputs = [ pkgs.autoPatchelfHook ];
            buildInputs = slintPrebuiltDeps;
            sourceRoot = "slint-lsp";
            installPhase = ''
              mkdir -p $out/bin
              cp slint-lsp $out/bin/
              chmod +x $out/bin/slint-lsp
            '';
          };

          runtimeLibraries = with pkgs; [
            fontconfig
            freetype
            wayland
            libxkbcommon
            libglvnd
            libinput
            libgbm
            vulkan-loader
            alsa-lib
            openssl
            llvmPackages.libcxx
          ];

          # Murasame prebuilt binary packages from GitHub Releases
          murasame-appimage-src = pkgs.fetchurl {
            url = "https://github.com/Initsnow/murasame/releases/download/v${murasameVersion}/murasame-v${murasameVersion}-x86_64.AppImage";
            # Hash matches GitHub release asset once published
            hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
          };

          appimageContents = pkgs.appimageTools.extractType2 {
            pname = "murasame";
            version = murasameVersion;
            src = murasame-appimage-src;
          };

          murasame-appimage = pkgs.appimageTools.wrapType2 {
            pname = "murasame";
            version = murasameVersion;
            src = murasame-appimage-src;

            extraInstallCommands = ''
              mkdir -p $out/share/applications $out/share/icons/hicolor/192x192/apps
              cp -r ${appimageContents}/usr/share/applications/* $out/share/applications/ 2>/dev/null || true
              cp -r ${appimageContents}/usr/share/icons/* $out/share/icons/ 2>/dev/null || true
            '';

            extraPkgs =
              pkgs: with pkgs; [
                vulkan-loader
                wayland
                libxkbcommon
                libglvnd
                libgbm
                libinput
                alsa-lib
                fontconfig
                freetype
                openssl
                grim
              ];

            meta = {
              description = "A lightweight AI desktop companion powered by Live2D and Rust";
              homepage = "https://github.com/Initsnow/murasame";
              license = pkgs.lib.licenses.gpl3Plus;
              mainProgram = "murasame";
              platforms = [ "x86_64-linux" ];
            };
          };

          murasame-tar = pkgs.stdenv.mkDerivation rec {
            pname = "murasame";
            version = murasameVersion;

            src = pkgs.fetchurl {
              url = "https://github.com/Initsnow/murasame/releases/download/v${version}/murasame-v${version}-linux-x64.tar.gz";
              hash = "sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
            };

            nativeBuildInputs = with pkgs; [
              autoPatchelfHook
              makeWrapper
            ];

            buildInputs = runtimeLibraries ++ [ pkgs.stdenv.cc.cc.lib ];

            installPhase = ''
              mkdir -p $out/bin $out/share/murasame $out/share/applications $out/share/icons/hicolor/192x192/apps
              cp murasame $out/bin/
              [ -f voice-tune ] && cp voice-tune $out/bin/
              cp -r config.toml characters Murasame_live2d models assets $out/share/murasame/
              cp murasame.desktop $out/share/applications/
              cp app_icon.png $out/share/icons/hicolor/192x192/apps/murasame.png

              wrapProgram $out/bin/murasame \
                --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.grim ]} \
                --set-default MURASAME_RESOURCE_DIR $out/share/murasame
            '';

            meta = {
              description = "A lightweight AI desktop companion powered by Live2D and Rust";
              homepage = "https://github.com/Initsnow/murasame";
              license = pkgs.lib.licenses.gpl3Plus;
              mainProgram = "murasame";
              platforms = [ "x86_64-linux" ];
            };
          };
        in
        {
          packages =
            {
              slint-viewer = slint-viewer-bin;
              slint-lsp = slint-lsp-bin;
            }
            // pkgs.lib.optionalAttrs (system == "x86_64-linux") {
              murasame = murasame-appimage;
              murasame-tar = murasame-tar;
              default = murasame-appimage;
            };

          devShells.default = pkgs.mkShell {
            nativeBuildInputs = with pkgs; [
              # Rust toolchain
              rustc
              cargo
              rustfmt
              clippy
              rust-analyzer

              # C/C++ compiler & build tools
              clang
              pkg-config
              cmake

              # Slint CLI tools from GitHub Releases
              slint-viewer-bin
              slint-lsp-bin

              # Wayland screenshot utility
              grim
            ];

            buildInputs = runtimeLibraries;

            # Environment variables
            RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
            LD_LIBRARY_PATH = "${pkgs.lib.makeLibraryPath runtimeLibraries}";
            LIBRARY_PATH = "${pkgs.lib.makeLibraryPath runtimeLibraries}";

            # Slint debug / MCP support (preserves element IDs and source locations)
            SLINT_EMIT_DEBUG_INFO = "1";

            # Use clang as the default C/C++ compiler
            CC = "clang";
            CXX = "clang++";
          };
        }
      );
    in
    systemOutputs
    // {
      nixosModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.programs.murasame;
        in
        {
          options.programs.murasame = {
            enable = lib.mkEnableOption "Murasame AI desktop companion";
            package = lib.mkPackageOption self.packages.${pkgs.system} "murasame" {
              default = [ "default" ];
            };
          };

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ cfg.package ];
          };
        };

      overlays.default = final: prev: {
        murasame = self.packages.${prev.system}.default;
      };
    };
}
