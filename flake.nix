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

        # Linux libraries gpui links against (the README's apt list) plus the
        # Vulkan loader, which gpui dlopens at runtime rather than linking.
        runtimeLibs = with pkgs; [
          wayland
          libxkbcommon
          vulkan-loader
          fontconfig
          freetype
          xorg.libX11
          xorg.libxcb
          xorg.xcbutil
        ];

        zorite = pkgs.rustPlatform.buildRustPackage {
          pname = "zorite";
          # Releases are tagged (vX.Y.Z); Cargo.toml stays at 0.1.0 by design,
          # so the package version is maintained here.
          version = "0.6.0";

          src = self;

          cargoLock = {
            lockFile = ./Cargo.lock;
            # Every git dependency in Cargo.lock (gpui + zed siblings, wgpu's
            # naga, gpui-component, mermaid-rs-renderer, heic-decoder + its
            # rav1d fork, …) is fetched with Nix's builtin git fetcher — pure
            # without per-source hashes because the lock pins exact revs. An
            # eventual nixpkgs submission would need explicit outputHashes
            # instead (builtin fetchGit isn't allowed there).
            allowBuiltinFetchGit = true;
          };

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = runtimeLibs;

          # The workspace tests are headless (CI runs them on bare runners).
          # Doc-tests need the same libs; both inherit buildInputs.
          checkFlags = [ ];

          postFixup = ''
            # gpui dlopens Vulkan and the Wayland/X11 client libs at runtime.
            patchelf --add-rpath ${pkgs.lib.makeLibraryPath runtimeLibs} \
              $out/bin/zorite
          '';

          postInstall = ''
            install -Dm644 resources/icons/icon.png \
              $out/share/icons/hicolor/512x512/apps/zorite.png
            mkdir -p $out/share/applications
            cat > $out/share/applications/zorite.desktop <<EOF
            [Desktop Entry]
            Name=Zorite
            Comment=Local-first Markdown daily journal
            Exec=zorite
            Icon=zorite
            Type=Application
            Categories=Office;
            EOF
          '';

          meta = with pkgs.lib; {
            description = "Local-first, Markdown daily-journal desktop app";
            homepage = "https://github.com/packetThrower/zorite";
            license = licenses.gpl3Plus;
            mainProgram = "zorite";
            platforms = [ "x86_64-linux" "aarch64-linux" ];
          };
        };
      in
      {
        packages.default = zorite;
        packages.zorite = zorite;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ zorite ];
          packages = with pkgs; [ rustc cargo clippy rustfmt ];
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath runtimeLibs;
        };
      });
}
