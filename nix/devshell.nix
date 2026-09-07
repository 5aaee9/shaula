_: {
  perSystem =
    {
      config,
      pkgs,
      toolchain,
      ...
    }:
    {
      devShells.default = pkgs.mkShell {
        packages = [
          toolchain
          pkgs.cargo-nextest
          pkgs.nodejs
          pkgs.pkg-config
          config.packages.terraform
          pkgs.jq
          pkgs.sqlite
          pkgs.openssl
          config.treefmt.build.wrapper
        ];
        RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
        shellHook = ''
          echo 'Shaula: run npm ci --prefix web once before cargo build/test.'
        '';
      };
    };
}
