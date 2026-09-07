{ self, inputs, ... }:
{
  perSystem =
    {
      config,
      lib,
      pkgs,
      system,
      ...
    }:
    {
      checks = {
        nixos-module = import ./tests/module-eval.nix {
          inherit lib pkgs system;
          inherit (inputs) nixpkgs;
          module = self.nixosModules.shaula;
          shaula = config.packages.shaula;
          terraform = config.packages.terraform;
        };
        nixos-e2e = import ./tests/service.nix {
          inherit pkgs;
          terraform = config.packages.terraform;
          module = self.nixosModules.shaula;
          shaula = config.packages.shaula;
        };
      };
    };
}
