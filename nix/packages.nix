_: {
  perSystem =
    {
      config,
      pkgs,
      toolchain,
      ...
    }:
    {
      packages = {
        shaula = pkgs.callPackage ./packages/shaula.nix {
          inherit toolchain;
          terraform = config.packages.terraform;
        };
        terraform = pkgs.callPackage ./packages/terraform.nix { };
        default = config.packages.shaula;
      };
      checks.rust = config.packages.shaula;
    };
}
