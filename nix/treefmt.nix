_: {
  perSystem = { pkgs, toolchain, ... }: {
    treefmt = {
      projectRootFile = "flake.nix";
      programs = {
        nixfmt.enable = true;
        deadnix.enable = true;
        statix.enable = true;
        actionlint.enable = true;
        rustfmt = {
          enable = true;
          package = toolchain;
          edition = "2021";
        };
        ruff-format.enable = true;
        ruff-check.enable = true;
        shellcheck.enable = true;
        shfmt.enable = true;
      };
      settings = {
        excludes = [
          "references/**"
          "target/**"
          "**/target/**"
          ".agents/**"
          "web/node_modules/**"
          "web/dist/**"
          "web/test-results/**"
          "web/playwright-report/**"
          "flake.lock"
          "web/package-lock.json"
        ];
        formatter = {
          # Nix checks run from source archives without a .git directory.
          actionlint.options = [
            "-config-file"
            ".github/actionlint.yaml"
          ];
          # Auto-fix linters run first; whitespace formatters run last.
          nixfmt.priority = 10;
          ruff-format.priority = 10;
          oxfmt = {
            command = "${pkgs.oxfmt}/bin/oxfmt";
            options = [
              "--config"
              "web/.oxfmtrc.json"
            ];
            includes = [
              "web/*.ts"
              "web/*.tsx"
              "web/*.json"
              "web/*.css"
            ];
          };
          oxlint = {
            command = "${pkgs.oxlint}/bin/oxlint";
            options = [ "--deny-warnings" ];
            includes = [
              "web/*.ts"
              "web/*.tsx"
            ];
          };
        };
      };
    };
  };
}
