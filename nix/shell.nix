{
  mkShell,
  common,
  file,
  openssh,
  qemu,
}:
mkShell ({
  nativeBuildInputs = [
    common.toolchain
    common.armv7Cc
    # Deploy/recon to the tablet over ssh/scp.
    openssh
    # Verify cross binaries (arch, static linkage).
    file
    # qemu-arm runs the armv7 binaries on the dev machine for quick checks.
    qemu
  ];
} // common.env)
