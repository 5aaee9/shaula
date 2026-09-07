{
  pkgs,
  shaula,
  terraform,
  module,
}:
let
  python = pkgs.python3.withPackages (p: [
    p.cryptography
    p.pyjwt
    p.requests
  ]);
  # Disposable test PKI only: never use these Nix-store keys in a deployment.
  pki = pkgs.runCommand "shaula-vm-test-pki" { nativeBuildInputs = [ pkgs.openssl ]; } ''
    mkdir -p "$out"
    openssl req -x509 -newkey rsa:2048 -noenc -days 3650 \
      -subj /CN=shaula-vm-test-ca \
      -addext basicConstraints=critical,CA:TRUE \
      -addext keyUsage=critical,keyCertSign,cRLSign \
      -keyout ca-key.pem -out "$out/ca.pem"
    openssl req -new -newkey rsa:2048 -noenc -subj /CN=shaula.test \
      -keyout "$out/key.pem" -out server.csr
    printf '%s\n' 'basicConstraints=critical,CA:FALSE' \
      'keyUsage=critical,digitalSignature,keyEncipherment' \
      'extendedKeyUsage=serverAuth' 'subjectAltName=DNS:idp.test,DNS:shaula.test' > extensions
    openssl x509 -req -days 3650 -in server.csr -CA "$out/ca.pem" \
      -CAkey ca-key.pem -set_serial 2 -extfile extensions -out "$out/cert.pem"
  '';
  shared = { nodes, ... }: {
    networking.hosts.${nodes.machine.networking.primaryIPAddress} = [
      "idp.test"
      "shaula.test"
    ];
    security.pki.certificateFiles = [ "${pki}/ca.pem" ];
    environment.systemPackages = [
      pkgs.curl
      python
    ];
  };
in
pkgs.testers.runNixOSTest {
  name = "shaula-service-e2e";
  nodes = {
    machine = { lib, ... }: {
      imports = [
        module
        shared
      ];
      virtualisation.memorySize = 1536;
      virtualisation.cores = 2;
      environment.systemPackages = [
        pkgs.sqlite
        pkgs.jq
        shaula
      ];
      networking.firewall.allowedTCPPorts = [
        443
        8443
      ];
      services.shaula = {
        enable = true;
        package = shaula;
        terraformPackage = terraform;
        bindingsKeyFile = "/var/lib/shaula-test-secrets/bindings-key";
        oidc = {
          provider = "https://idp.test:8443";
          clientId = "shaula-web";
          clientSecretFile = "/var/lib/shaula-test-secrets/oidc-client-secret";
          publicUrl = "https://shaula.test";
          apiAudience = "shaula-api";
          caCertificate = "${pki}/ca.pem";
        };
        settings.http.authorization = [
          {
            issuer = "https://idp.test:8443";
            subject = "operator";
            scopes = [
              "fleet.read"
              "template.read"
              "auth.read"
              "auth.write"
            ];
          }
          {
            issuer = "https://idp.test:8443";
            subject = "reader";
            scopes = [ "auth.read" ];
          }
        ];
      };
      systemd.services.shaula = {
        requires = [ "shaula-test-secrets.service" ];
        after = [
          "shaula-test-secrets.service"
          "test-oidc.service"
        ];
        # Make startup failures observable without automatic retries in tests.
        serviceConfig.Restart = lib.mkForce "no";
      };
      systemd.services.shaula-test-secrets = {
        wantedBy = [ "multi-user.target" ];
        before = [ "shaula.service" ];
        serviceConfig = {
          Type = "oneshot";
          RemainAfterExit = true;
        };
        script = ''
          umask 077
          mkdir -p /var/lib/shaula-test-secrets
          if ! test -e /var/lib/shaula-test-secrets/bindings-key; then
            ${pkgs.openssl}/bin/openssl rand -hex 32 > /var/lib/shaula-test-secrets/bindings-key
          fi
          printf '%s' nixos-test-only-oidc-secret > /var/lib/shaula-test-secrets/oidc-client-secret
        '';
      };
      systemd.services.test-oidc = {
        wantedBy = [ "multi-user.target" ];
        wants = [ "network-online.target" ];
        after = [ "network-online.target" ];
        before = [ "shaula.service" ];
        serviceConfig.ExecStart = "${python}/bin/python ${./oidc-provider.py} ${pki}/cert.pem ${pki}/key.pem";
        postStart = ''
          ${pkgs.curl}/bin/curl --silent --show-error --fail --retry 20 \
            --retry-all-errors --retry-delay 1 --retry-max-time 30 --max-time 2 \
            --cacert ${pki}/ca.pem \
            https://idp.test:8443/.well-known/openid-configuration > /dev/null
        '';
      };
      services.nginx = {
        enable = true;
        recommendedProxySettings = true;
        virtualHosts."shaula.test" = {
          forceSSL = true;
          sslCertificate = "${pki}/cert.pem";
          sslCertificateKey = "${pki}/key.pem";
          extraConfig = "access_log off;";
          locations."/" = {
            proxyPass = "http://127.0.0.1:8080";
            extraConfig = "proxy_no_cache 1; proxy_cache_bypass 1;";
          };
        };
      };
    };
    client = {
      imports = [ shared ];
      virtualisation.memorySize = 768;
      environment.etc."shaula-http-checks.py".source = ./http-checks.py;
    };
  };
  testScript = builtins.readFile ./driver.py;
}
