{
  description = "Zorite — a local-first, Markdown daily-journal desktop app (Rust + GPUI)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system:
      let
        pkgs = import nixpkgs { inherit system; };

        # The same derivation nixpkgs ships (nix/package.nix), built from this
        # checkout instead of the release tag. Git dependencies come from the
        # lock via Nix's builtin fetcher, so there is no cargoHash to maintain.
        zorite = (pkgs.callPackage ./nix/package.nix { }).overrideAttrs (old: {
          src = self;
          cargoDeps = pkgs.rustPlatform.importCargoLock {
            lockFile = ./Cargo.lock;
            allowBuiltinFetchGit = true;
          };
          # nixpkgs master's cargoSetupHook forces system libraries (2026-10-12);
          # nixos-unstable doesn't have it yet. Remove once it does.
          env = (old.env or { }) // {
            OPENSSL_NO_VENDOR = "1";
            LIBSQLITE3_SYS_USE_PKG_CONFIG = "1";
          };
          meta = old.meta // {
            changelog = "https://github.com/packetThrower/zorite/blob/main/CHANGELOG.md";
          };
        });
      in
      {
        packages.default = zorite;
        packages.zorite = zorite;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ zorite ];
          packages = with pkgs; [ rustc cargo clippy rustfmt ];
          # What the package puts on the binary's rpath, for `cargo run`.
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (with pkgs; [
            libGL
            wayland
            libxkbcommon
            vulkan-loader
          ]);
        };
      });
}
