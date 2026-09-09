"""NixOS VM driver: real release package, service module and external client."""

start_all()
machine.wait_for_unit("test-oidc.service")
machine.wait_for_unit("shaula.service")
machine.wait_for_unit("nginx.service")
client.wait_for_unit("multi-user.target")


def ready():
    machine.wait_until_succeeds(
        "test $(curl --silent --output /dev/null --write-out '%{http_code}' http://127.0.0.1:8080/livez) = 401",
        timeout=60,
    )


with subtest("module hardening and loopback-only management listener"):
    ready()
    machine.succeed("shaula version | grep 'shaula 0.1.0'")
    machine.succeed("test $(stat -Lc %a /var/lib/shaula) = 700")
    machine.succeed("test $(stat -c %a /run/shaula/bootstrap.json) = 600")
    machine.fail("runuser -u nobody -- cat /run/shaula/bootstrap.json")
    machine.fail("runuser -u nobody -- cat /var/lib/shaula/shaula.db")
    machine.succeed("ss -ltn | grep '127.0.0.1:8080'")
    client.fail("curl --max-time 3 http://machine:8080/livez")
    machine.succeed("test $(systemctl show shaula -p DynamicUser --value) = yes")
    key_digest = machine.succeed("sha256sum /var/lib/shaula-test-secrets/bindings-key")

with subtest("TLS-verified OIDC, embedded assets and authenticated persistence"):
    client.succeed("python /etc/shaula-http-checks.py initial")
    machine.succeed(
        "sqlite3 /var/lib/shaula/shaula.db 'PRAGMA integrity_check' | grep '^ok$'"
    )
    machine.fail(
        "journalctl -u shaula --no-pager | grep -F github_app_nixos_fixture_never_valid"
    )
    machine.fail(
        "journalctl -u shaula --no-pager | grep -F nixos-test-only-oidc-secret"
    )

with subtest("graceful SIGINT restart retains SQLite but invalidates browser sessions"):
    machine.succeed("systemctl restart shaula")
    machine.wait_for_unit("shaula.service")
    ready()
    machine.succeed("journalctl -u shaula --no-pager | grep 'shaula stopped cleanly'")
    client.succeed("python /etc/shaula-http-checks.py after-restart")

with subtest("machine reboot preserves the data directory and bindings key"):
    boot_id = machine.succeed("cat /proc/sys/kernel/random/boot_id")
    machine.reboot()
    # The driver's default QEMU exits on reboot; restart with the existing disk.
    machine.wait_for_shutdown()
    machine.start()
    machine.wait_for_unit("shaula.service")
    machine.wait_for_unit("nginx.service")
    ready()
    assert boot_id != machine.succeed("cat /proc/sys/kernel/random/boot_id")
    assert key_digest == machine.succeed(
        "sha256sum /var/lib/shaula-test-secrets/bindings-key"
    )
    client.succeed("python /etc/shaula-http-checks.py after-restart")

with subtest("missing systemd credential fails closed without a listener"):
    machine.succeed("systemctl stop shaula")
    machine.succeed(
        "mv /var/lib/shaula-test-secrets/oidc-client-secret /var/lib/shaula-test-secrets/saved-secret"
    )
    machine.fail("systemctl start shaula")
    machine.wait_until_succeeds("systemctl is-failed shaula")
    machine.fail("curl --max-time 2 http://127.0.0.1:8080/livez")
    machine.succeed(
        "mv /var/lib/shaula-test-secrets/saved-secret /var/lib/shaula-test-secrets/oidc-client-secret"
    )

with subtest("unavailable OIDC issuer prevents startup, without losing durable data"):
    machine.succeed("systemctl stop test-oidc")
    machine.execute("systemctl start shaula")
    machine.wait_until_succeeds("systemctl is-failed shaula")
    machine.fail("curl --max-time 2 http://127.0.0.1:8080/livez")
    machine.succeed("systemctl start test-oidc")
    machine.succeed("systemctl reset-failed shaula; systemctl start shaula")
    machine.wait_for_unit("shaula.service")
    ready()
    client.succeed("python /etc/shaula-http-checks.py after-restart")
