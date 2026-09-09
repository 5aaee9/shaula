self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.shaula;
  json = pkgs.formats.json { };
  dataDir = "/var/lib/${cfg.stateDirectory}";
  publicConfig = json.generate "shaula-bootstrap-public.json" (
    lib.recursiveUpdate {
      version = 1;
      storage.data_dir = dataDir;
      template_source_dirs = cfg.templateSourceDirectories;
      http.listen = "127.0.0.1:8080";
      execution.engines.terraform.executable = lib.getExe cfg.terraformPackage;
    } cfg.settings
  );
  runtimeSecret = path: lib.hasPrefix "/" path && !(lib.hasPrefix "/nix/store/" path);
  start = pkgs.writeShellApplication {
    name = "shaula-start";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.jq
      pkgs.docker-client
      pkgs.kubectl
    ];
    text = ''
      umask 077
      # Values never enter the Nix store or command-line arguments.
      export SHAULA_OIDC_CLIENT_SECRET
      SHAULA_OIDC_CLIENT_SECRET="$(cat "$CREDENTIALS_DIRECTORY/oidc-client-secret")"
      jq --rawfile key "$CREDENTIALS_DIRECTORY/bindings-key" \
        '.http.bindings_server_key = ($key | rtrimstr("\n") | rtrimstr("\r"))' \
        ${publicConfig} > "$RUNTIME_DIRECTORY/bootstrap.json"
      exec ${lib.getExe cfg.package} serve --config "$RUNTIME_DIRECTORY/bootstrap.json"
    '';
  };
in
{
  options.services.shaula = {
    enable = lib.mkEnableOption "the Shaula Scale Set controller";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.shaula;
      defaultText = lib.literalExpression "inputs.shaula.packages.\${pkgs.stdenv.hostPlatform.system}.shaula";
      description = "Shaula executable, including its embedded operator UI.";
    };
    terraformPackage = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.terraform;
      defaultText = lib.literalExpression "inputs.shaula.packages.\${pkgs.stdenv.hostPlatform.system}.terraform";
      description = ''
        Exact Terraform CLI, defaulting to the verified 1.9.8 release binary
        under BUSL-1.1. Changing it changes attestation authority and requires
        renewed compatibility acceptance. OpenTofu is not a supported substitute.
      '';
    };
    templateSourceDirectories = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "${cfg.package}/share/shaula/templates" ];
      defaultText = lib.literalExpression ''[ "''${config.services.shaula.package}/share/shaula/templates" ]'';
      description = ''
        Trusted read-only directories synchronized to the default template catalog
        at startup. Package changes replace source entries, while published Profile
        revisions stay immutable. Missing configured directories fail startup.
        Use [] to clear the default catalog without removing published templates.
      '';
    };
    stateDirectory = lib.mkOption {
      type = lib.types.strMatching "[a-zA-Z0-9][a-zA-Z0-9._-]*";
      default = "shaula";
      description = "Private persistent directory name beneath /var/lib, managed by systemd.";
    };
    bindingsKeyFile = lib.mkOption {
      type = lib.types.str;
      example = "/run/secrets/shaula-bindings-key";
      description = ''
        Absolute runtime path to a persistent, deployment-unique key (at least
        32 characters). Use a string, not a Nix path or builtins.readFile.
        Provision it with a secret manager; do not regenerate it on restart.
      '';
    };
    settings = lib.mkOption {
      inherit (json) type;
      default = { };
      example = lib.literalExpression ''
        {
          http.authorization = [{
            issuer = "https://identity.example.com";
            subject = "operator-subject";
            scopes = [ "fleet.read" "template.read" ];
          }];
          execution.operation_timeout_secs = 1800;
        }
      '';
      description = ''
        Non-secret daemon bootstrap settings. The listener must stay loopback.
        storage.data_dir, execution.engines and http.bindings_server_key are
        owned by this module. OIDC settings belong in the oidc options.
      '';
    };
    oidc = {
      provider = lib.mkOption {
        type = lib.types.str;
        example = "https://identity.example.com";
        description = "Exact HTTPS issuer.";
      };
      clientId = lib.mkOption {
        type = lib.types.str;
        description = "Confidential browser client ID.";
      };
      publicUrl = lib.mkOption {
        type = lib.types.str;
        example = "https://shaula.example.com";
        description = "Fixed external HTTPS origin; configure TLS ingress separately.";
      };
      apiAudience = lib.mkOption {
        type = lib.types.str;
        description = "API audience, distinct from the browser client ID.";
      };
      clientSecretFile = lib.mkOption {
        type = lib.types.str;
        example = "/run/secrets/shaula-oidc-client-secret";
        description = "Absolute runtime secret-file path, never a Nix-store path. Loaded with systemd credentials.";
      };
      caCertificate = lib.mkOption {
        type = lib.types.nullOr lib.types.path;
        default = null;
        description = "Optional public PEM CA certificate for a private HTTPS Provider; TLS verification remains enabled.";
      };
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = runtimeSecret cfg.bindingsKeyFile && runtimeSecret cfg.oidc.clientSecretFile;
        message = "Shaula secret files must be absolute runtime paths outside /nix/store; pass strings, not Nix paths.";
      }
      {
        assertion = !(cfg.settings ? http.bindings_server_key);
        message = "Use services.shaula.bindingsKeyFile, never a plaintext bindings key in settings.";
      }
      {
        assertion = !(cfg.settings ? storage.data_dir) && !(cfg.settings ? execution.engines);
        message = "Use services.shaula.stateDirectory and terraformPackage for module-owned storage and engine paths.";
      }
      {
        assertion = !(cfg.settings ? template_source_dirs);
        message = "Use services.shaula.templateSourceDirectories for module-owned template sources.";
      }
    ];
    systemd.services.shaula = {
      description = "Shaula GitHub Actions Scale Set controller";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      environment = {
        SHAULA_OIDC_PROVIDER = cfg.oidc.provider;
        SHAULA_OIDC_CLIENT_ID = cfg.oidc.clientId;
        SHAULA_OIDC_PUBLIC_URL = cfg.oidc.publicUrl;
        SHAULA_OIDC_API_AUDIENCE = cfg.oidc.apiAudience;
      }
      // lib.optionalAttrs (cfg.oidc.caCertificate != null) {
        SHAULA_OIDC_CA_CERT = toString cfg.oidc.caCertificate;
      };
      serviceConfig = {
        ExecStart = lib.getExe start;
        DynamicUser = true;
        StateDirectory = cfg.stateDirectory;
        StateDirectoryMode = "0700";
        RuntimeDirectory = "shaula";
        RuntimeDirectoryMode = "0700";
        WorkingDirectory = dataDir;
        UMask = "0077";
        LoadCredential = [
          "bindings-key:${cfg.bindingsKeyFile}"
          "oidc-client-secret:${cfg.oidc.clientSecretFile}"
        ];
        Restart = "on-failure";
        RestartSec = "5s";
        # The current composition root handles SIGINT, not SIGTERM.
        KillSignal = "SIGINT";
        KillMode = "control-group";
        TimeoutStopSec = "90s";
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
        ];
      };
    };
    # No firewall opening or anonymous health-probe bypass. TLS ingress is a
    # separate deployment concern and must proxy only the management listener.
  };
}
