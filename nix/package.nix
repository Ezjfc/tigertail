# Called from the armv7 musl cross package set (see flake.nix), so
# buildRustPackage targets armv7-unknown-linux-musleabihf and the daemon comes
# out statically linked.
{
  lib,
  makeRustPlatform,
  pkgsBuildHost,
}:
let
  toolchain = pkgsBuildHost.rust-bin.stable.latest.default.override {
    targets = [ "armv7-unknown-linux-musleabihf" ];
  };
  rustPlatform = makeRustPlatform {
    cargo = toolchain;
    rustc = toolchain;
  };
in
rustPlatform.buildRustPackage {
  pname = "tigertaild";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  src = lib.cleanSourceWith {
    src = ../.;
    filter = path: _type:
      let base = baseNameOf path;
      in !(builtins.elem base [ "target" ".direnv" "result" ]);
  };

  cargoLock = {
    lockFile = ../Cargo.lock;
    # The rm-pad transform library is a git dependency; its hash must be
    # bumped whenever the pinned rev changes.
    outputHashes = {
      "rm-pad-0.1.0" = "sha256-sUxZzxP/vbfnxqIoT4p7f9wD/CmqlpaX9gkZtmENmUM=";
    };
  };

  buildAndTestSubdir = "crates/tigertaild";

  postInstall = ''
    install -Dm644 ../data/tigertaild.service "$out/lib/systemd/system/tigertaild.service"
    install -Dm644 ../tigertail.toml.example "$out/share/tigertail/tigertail.toml.example"
  '';

  # armv7 test binaries can't run on the build machine; qemu-based checks
  # happen in the devShell instead.
  doCheck = false;

  meta = {
    description = "Native reMarkable daemon exposing the tablet as a USB HID pen";
    mainProgram = "tigertaild";
    platforms = [ "armv7l-linux" ];
  };
}
