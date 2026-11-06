{
  description = "Slint + Rust development environment";

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
    flake-utils.lib.eachDefaultSystem (
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
      in
      {
        packages = {
          slint-viewer = slint-viewer-bin;
          slint-lsp = slint-lsp-bin;
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
}
