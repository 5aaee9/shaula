{ self, inputs, ... }:
{
  imports = [
    inputs.treefmt-nix.flakeModule
    ./packages.nix
    ./devshell.nix
    ./treefmt.nix
    ./tests.nix
  ];

  flake.nixosModules = {
    shaula = import ./modules/shaula.nix self;
    default = self.nixosModules.shaula;
  };

  systems = [
    "x86_64-linux"
    "aarch64-linux"
  ];

  perSystem = { system, ... }: {
    _module.args = {
      pkgs = import inputs.nixpkgs {
        inherit system;
        # Terraform >=1.9 is the current engine contract, not OpenTofu.
        # This exception is deliberately limited to its BUSL-1.1 package.
        config.allowUnfreePredicate = package: inputs.nixpkgs.lib.getName package == "terraform";
      };
      toolchain = inputs.fenix.packages.${system}.stable.withComponents [
        "cargo"
        "clippy"
        "rustc"
        "rustfmt"
        "rust-src"
      ];
    };
  };
}
