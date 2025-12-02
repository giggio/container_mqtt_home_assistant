{
  description = "Container Status MQTT Home Assistant";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = "github:nix-community/naersk";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, flake-utils, fenix, naersk }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ fenix.overlays.default ];
        };
        lib = pkgs.lib;

        buildForTarget = import ./cross-build.nix {
          inherit nixpkgs; inherit lib; inherit system; inherit fenix; inherit naersk;
        };
      in
      {
        packages = {
          x86_64-gnu = buildForTarget { target = "x86_64-unknown-linux-gnu"; }; # this will link to nix libraries and wont work in any other Linux system
          x86_64 = buildForTarget { target = "x86_64-unknown-linux-musl"; };
          x86_64_test = buildForTarget { target = "x86_64-unknown-linux-musl"; mode = "test"; };
          x86_64_clippy = buildForTarget { target = "x86_64-unknown-linux-musl"; mode = "clippy"; };
          aarch64 = buildForTarget { target = "aarch64-unknown-linux-musl"; };
          default = pkgs.runCommand "cmha-all-outputs" # builds outputs to be used by conteiner images only, statically linked
            {
              buildInputs = [
                self.packages.${system}.x86_64_test
                self.packages.${system}.x86_64_clippy
              ];
            }
            ''
              mkdir -p $out/bin
              ln -s ${self.packages.${system}.x86_64}/bin/cmha $out/bin/cmha_x86_64
              ln -s ${self.packages.${system}.aarch64}/bin/cmha $out/bin/cmha_aarch64
            '';
        };
        devShells.default = import ./shell.nix { inherit pkgs; };
        formatter = pkgs.nixpkgs-fmt;
      }
    );
}
