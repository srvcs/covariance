{
  description = "srvcs-covariance: statistics: population covariance";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ rust-overlay.overlays.default ];
        };
        version = "0.1.0";
        rustToolchain = pkgs.rust-bin.stable."1.96.0".default.override {
          extensions = [ "clippy" "rustfmt" ];
        };
        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };
      in {
        packages = {
          default = rustPlatform.buildRustPackage {
            pname = "srvcs-covariance";
            inherit version;
            src = ./.;
            cargoHash = "sha256-bFFjGg/+Kkzzk4/DjKY/i7l+E8DZurv8H33Cs1h7dYI=";
          };
        } // pkgs.lib.optionalAttrs pkgs.stdenv.isLinux {
          container = pkgs.dockerTools.buildLayeredImage {
            name = "srvcs-covariance";
            tag = "latest";
            config = {
              Entrypoint = [ "${self.packages.${system}.default}/bin/srvcs-covariance" ];
              ExposedPorts = { "8080/tcp" = { }; };
              User = "65534:65534";
              Labels = {
                "org.opencontainers.image.title" = "srvcs-covariance";
                "org.opencontainers.image.description" = "Population-covariance orchestrator: composes srvcs-sum, srvcs-floatdivide, srvcs-floatsubtract, srvcs-floatmultiply and srvcs-floatadd to compute the population covariance mean((a - meanA) * (b - meanB)) of two equal-length lists of numbers.";
                "org.opencontainers.image.version" = version;
                "org.opencontainers.image.revision" = self.rev or "dev";
                "org.opencontainers.image.source" = "https://github.com/srvcs/covariance";
                "org.opencontainers.image.licenses" = "Apache-2.0";
              };
            };
          };
        };

        devShells.default = pkgs.mkShell {
          packages = [ rustToolchain pkgs.syft ];
        };
      });
}
