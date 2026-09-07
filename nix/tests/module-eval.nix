{
  lib,
  pkgs,
  nixpkgs,
  system,
  module,
  shaula,
  terraform,
}:
let
  evaluate =
    settings:
    nixpkgs.lib.nixosSystem {
      inherit system;
      modules = [
        module
        {
          boot.isContainer = true;
          system.stateVersion = "26.05";
          services.shaula = settings;
        }
      ];
    };
  base = {
    enable = true;
    bindingsKeyFile = "/run/secrets/bindings";
    oidc = {
      provider = "https://identity.example.com";
      clientId = "shaula-web";
      clientSecretFile = "/run/secrets/oidc";
      publicUrl = "https://shaula.example.com";
      apiAudience = "shaula-api";
    };
  };
  rejects =
    settings: message:
    lib.any (a: !a.assertion && lib.hasInfix message a.message)
      (evaluate (lib.recursiveUpdate base settings)).config.assertions;
  valid = (evaluate base).config;
in
assert !((evaluate { }).config.systemd.services ? shaula);
assert lib.all (a: a.assertion) valid.assertions;
assert valid.services.shaula.package.drvPath == shaula.drvPath;
assert valid.services.shaula.terraformPackage.drvPath == terraform.drvPath;
assert valid.systemd.services.shaula.serviceConfig.KillSignal == "SIGINT";
assert rejects { bindingsKeyFile = "/nix/store/not-a-secret"; } "absolute runtime paths";
assert rejects { oidc.clientSecretFile = "relative-secret"; } "absolute runtime paths";
assert rejects {
  settings.http.bindings_server_key = "must-not-go-into-the-store";
} "plaintext bindings key";
assert rejects { settings.storage.data_dir = "/tmp/escape"; } "module-owned storage";
assert rejects { settings.execution.engines = { }; } "module-owned storage";
pkgs.runCommand "shaula-nixos-module-check" { } ''
  touch "$out"
''
