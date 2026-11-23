{
  description = "SSH for Gemini: pure HTTP POST/GET <-> local SSH bridge daemon (Rust)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    {
      nixosModules.default = import ./nix/module.nix;
      nixosModules.ssh-for-gemini = self.nixosModules.default;

      overlays.default = final: prev: {
        ssh-for-gemini =
          self.packages.${final.system}.ssh-for-gemini
            or final.callPackage ./package.nix {
              version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
            };
      };
    }
    // flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
      in
      {
        packages = {
          default = pkgs.callPackage ./package.nix {
            doCheck = false;
            inherit version;
          };
          ssh-for-gemini = self.packages.${system}.default;
        };

        checks.default = pkgs.callPackage ./package.nix {
          doCheck = true;
          inherit version;
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo
            rustc
            rust-analyzer
            rustfmt
            clippy
          ];
          RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
        };
      });
}
