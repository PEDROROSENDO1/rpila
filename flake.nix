{
  description = "A stacking Wayland compositor built on Smithay";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" ] (
          system: f system nixpkgs.legacyPackages.${system}
        );
    in
    {
      packages = forAllSystems (
        _: pkgs: rec {
          pila = pkgs.callPackage ./package.nix {
            commit = self.shortRev or self.dirtyShortRev or null;
          };
          default = pila;
          # Pila and the standard tools its default configuration runs (the
          # same list as packaging/nfpm-desktop.yaml's): `pila` alone is the
          # compositor alone, whose defaults that aren't installed are skipped.
          # swaylock needs `security.pam.services.swaylock = { };` on NixOS.
          pila-desktop = pkgs.buildEnv {
            name = "pila-desktop-${pila.version}";
            paths = [
              pila
              pkgs.swaybg
              pkgs.kitty
              pkgs.waybar
              pkgs.fuzzel
              pkgs.mako
              pkgs.font-awesome
              pkgs.swayidle
              pkgs.swaylock
              pkgs.wireplumber
              pkgs.brightnessctl
              pkgs.playerctl
              pkgs.grim
              pkgs.slurp
              pkgs.wl-clipboard
            ];
            # For `services.displayManager.sessionPackages`, as `pila`.
            passthru.providedSessions = [ "pila" ];
            meta = pila.meta // {
              description = "Pila with the wallpaper, bar, launcher, notifications, lock and keys of its default configuration";
            };
          };
        }
      );

      devShells = forAllSystems (
        system: pkgs:
        let
          pila = self.packages.${system}.default;
        in
        {
          default = pkgs.mkShell {
            inputsFrom = [ pila ];
            packages = [
              pkgs.clippy
              pkgs.rustfmt
              pkgs.xwayland
            ];
            RUSTFLAGS = pila.rustFlags;
          };
        }
      );
    };
}
