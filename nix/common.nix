{
  rust-bin,
  writeShellScriptBin,
  pkgsCross,
}:
let
  # Rust toolchain with both on-device targets: the daemon is static musl
  # (immune to the tablet's old glibc), the Qt GUI is dynamic gnueabihf
  # (links the tablet's own Qt 5.15).
  toolchain = rust-bin.stable.latest.default.override {
    extensions = [ "rust-src" "rust-analyzer" ];
    targets = [
      "armv7-unknown-linux-musleabihf"
      "armv7-unknown-linux-gnueabihf"
    ];
  };

  # Link driver for both armv7 targets. Same trick as rm-pad's nix/common.nix:
  # the glibc cross gcc stays in the binary cache (a musl cross gcc would build
  # from source). Rust's musl targets are self-contained — rustc supplies its
  # own crt objects and libc.a — so the glibc-targeted driver only runs ld.
  cc = pkgsCross.armv7l-hf-multiplatform.stdenv.cc;
  armv7Cc = writeShellScriptBin "${cc.targetPrefix}gcc" ''
    exec ${cc}/bin/${cc.targetPrefix}gcc "$@"
  '';
  armv7Linker = "${armv7Cc}/bin/${cc.targetPrefix}gcc";
in
{
  inherit toolchain armv7Cc;

  env = {
    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_LINKER = armv7Linker;
    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER = armv7Linker;
    # The workspace builds for the tablet by default; the GUI overrides with
    # an explicit --target armv7-unknown-linux-gnueabihf.
    CARGO_BUILD_TARGET = "armv7-unknown-linux-musleabihf";
    # `cargo test`/`cargo run` execute the armv7 binaries via qemu user
    # emulation (static binaries, so no sysroot needed).
    CARGO_TARGET_ARMV7_UNKNOWN_LINUX_MUSLEABIHF_RUNNER = "qemu-arm";
  };
}
