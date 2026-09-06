{
  description = "tigertail development environment and packages";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/release-26.05";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    rust-overlay,
  }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        # Cross package set for the daemon's static musl build. Only `nix
        # build` pays for its from-source cross gcc; the devShell links with
        # the cached glibc cross gcc instead (see nix/common.nix).
        muslPkgs = import nixpkgs {
          localSystem = system;
          # isStatic makes the toolchain default to static linking, so the
          # daemon runs on the tablet without any nix-store interpreter.
          crossSystem = {
            config = "armv7l-unknown-linux-musleabihf";
            isStatic = true;
          };
          overlays = [ (import rust-overlay) ];
        };
        common = pkgs.callPackage ./nix/common.nix { };
      in
      {
        packages = {
          default = self.packages.${system}.tigertaild;
          tigertaild = muslPkgs.callPackage ./nix/package.nix { };
        };

        devShells.default = pkgs.callPackage ./nix/shell.nix { inherit common; };
      });
}
