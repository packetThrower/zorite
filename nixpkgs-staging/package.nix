{
  lib,
  rustPlatform,
  fetchFromGitHub,
  pkg-config,
  perl,
  wayland,
  libxkbcommon,
  vulkan-loader,
  fontconfig,
  freetype,
  xorg,
  nix-update-script,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "zorite";
  version = "0.6.0";

  src = fetchFromGitHub {
    owner = "packetThrower";
    repo = "zorite";
    tag = "v${finalAttrs.version}";
    hash = "sha256-QQ4NZylXAYu1yRLbJWgeilJ/iBDA8DkdIe4Yjo/rjKs=";
  };

  # Covers the crates.io and git dependencies (gpui and friends) in one hash.
  cargoHash = "sha256-TiwLCz/EpYSX8Xh9nhqz212AHAL4MZeNY9eOf6fj+mM=";

  nativeBuildInputs = [
    pkg-config
    # rusqlite's bundled-sqlcipher-vendored-openssl builds OpenSSL from
    # source, and OpenSSL's build system is perl.
    perl
  ];

  buildInputs = [
    wayland
    libxkbcommon
    fontconfig
    freetype
    xorg.libX11
    xorg.libxcb
    xorg.xcbutil
  ];

  postFixup = ''
    # gpui dlopens Vulkan and the Wayland/X11 client libraries at runtime
    # rather than linking them.
    patchelf --add-rpath ${
      lib.makeLibraryPath [
        wayland
        libxkbcommon
        vulkan-loader
      ]
    } $out/bin/zorite
  '';

  postInstall = ''
    install -Dm644 resources/icons/icon.png \
      $out/share/icons/hicolor/512x512/apps/zorite.png
    mkdir -p $out/share/applications
    cat > $out/share/applications/zorite.desktop <<INI
    [Desktop Entry]
    Name=Zorite
    Comment=Local-first Markdown daily journal
    Exec=zorite
    Icon=zorite
    Type=Application
    Categories=Office;
    INI
  '';

  passthru.updateScript = nix-update-script { };

  meta = {
    description = "Local-first, Markdown daily-journal desktop app with wiki-links, whiteboards, and PDF annotation";
    homepage = "https://github.com/packetThrower/zorite";
    changelog = "https://github.com/packetThrower/zorite/blob/v${finalAttrs.version}/CHANGELOG.md";
    license = lib.licenses.gpl3Plus;
    maintainers = with lib.maintainers; [ packetThrower ];
    mainProgram = "zorite";
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
    ];
  };
})
