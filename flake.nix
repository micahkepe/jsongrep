{
  description = "JSONPath-inspired query language for JSON, YAML, TOML, and other serialization formats";

  inputs = {
    nixpkgs.url = "https://flakehub.com/f/NixOS/nixpkgs/0";

    crane.url = "github:ipetkov/crane";

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
            pkgs.rust-analyzer
            pkgs.cargo-nextest
          ];
        };
      }
    );

    packages = forEachSupportedSystem ({
      pkgs,
      system,
    }: let
      craneLib = inputs.crane.mkLib pkgs;

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.intersection (lib.fileset.gitTracked ./.) (
          lib.fileset.unions [
            (craneLib.fileset.commonCargoSources ./.)
            (lib.fileset.fileFilter (file: file.hasExt "pest") ./src)
            ./benches
            ./tests
          ]
        );
      };

      commonArgs = {
        inherit src;
        strictDeps = true;
        cargoExtraArgs = "--locked --features all-formats";
        buildInputs = lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
          pkgs.libiconv
        ];
      };

      cargoArtifacts = craneLib.buildDepsOnly commonArgs;
    in {
      default = craneLib.buildPackage (commonArgs
        // {
          inherit cargoArtifacts;
          meta.mainProgram = "jg";
        });
    });
  };
}
