{
  description = "chiral — use a reMarkable 2 as a drawing tablet on Wayland";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            rust-analyzer
            pkg-config
            gtk4
            gtk4-layer-shell
            glib
            wayland
          ];
        };
      });

      packages = forAllSystems (pkgs: rec {
        chiral = pkgs.rustPlatform.buildRustPackage {
          pname = "chiral";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook4 ];
          buildInputs = with pkgs; [ gtk4 gtk4-layer-shell glib ];
          meta = {
            description = "Use a reMarkable 2 as a drawing tablet on Wayland";
            mainProgram = "chiral";
            platforms = systems;
          };
        };
        default = chiral;
      });

      nixosModules.default = import ./nix/module.nix self;
    };
}
