{
  description = "JSONPath-inspired query language for JSON, YAML, TOML, and other serialization formats";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0";

    alejandra.url = "github:kamadorueda/alejandra/4.0.0";
    alejandra.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = {self, ...} @ inputs: let
    inherit (inputs.nixpkgs) lib;

    supportedSystems = [
      "x86_64-linux"
      "aarch64-linux"
      "aarch64-darwin"
    ];

    forEachSupportedSystem = f:
      lib.genAttrs supportedSystems (
        system:
          f {
            inherit system;
            pkgs = import inputs.nixpkgs {
              inherit system;
            };
          }
      );
  in {
    formatter =
      forEachSupportedSystem ({system, ...}:
        inputs.alejandra.packages.${system}.default);

    devShells = forEachSupportedSystem (
      {
        pkgs,
        system,
      }: {
        default = pkgs.mkShellNoCC {
          inputsFrom = [self.packages.${system}.default];
          packages = [
            self.formatter.${system}
            pkgs.just
            pkgs.clippy
            pkgs.cargo-nextest
          ];
        };
      }
    );

    packages = forEachSupportedSystem ({pkgs, ...}: {
      default = let
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      in
        pkgs.rustPlatform.buildRustPackage {
          pname = cargoToml.package.name;
          version = cargoToml.package.version;
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.intersection (lib.fileset.gitTracked ./.) (
              lib.fileset.unions [
                ./src
                ./Cargo.toml
                ./Cargo.lock
                ./benches
                ./tests
              ]
            );
          };
          cargoHash = "sha256-cP0nStfLr5Lq9ZIctYBnomdUP1fNf5/g7lSMBo0wOWA";
          buildFeatures = ["all-formats"];
        };
    });
  };
}
