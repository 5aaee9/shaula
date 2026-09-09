"""HTTP acceptance against the packaged daemon behind NixOS HTTPS ingress."""

import json
import os
import re
import sys
from pathlib import Path

import requests

BASE = "https://shaula.test"
ISSUER = "https://idp.test:8443"
PROFILE = "/api/v1/github-auth-profiles/vm-test"
APP_ID = "4863460"
PRIVATE_KEY = "github_app_nixos_fixture_never_valid"
STATE = Path("/tmp/shaula-client-state.json")
CA = "/etc/ssl/certs/ca-certificates.crt"


def session():
    client = requests.Session()
    client.trust_env = False
    client.verify = CA
    return client


def access_token(**params):
    response = session().get(ISSUER + "/test/access-token", params=params, timeout=10)
    response.raise_for_status()
    return response.json()["access_token"]


def expect(client, method, path, status, **kwargs):
    response = client.request(
        method, BASE + path, timeout=15, allow_redirects=False, **kwargs
    )
    assert response.status_code == status, (
        f"{method} {path}: expected {status}, got {response.status_code}"
    )
    assert "no-store" in response.headers.get("Cache-Control", ""), path
    assert PRIVATE_KEY not in response.text, "credential leaked in HTTP response"
    return response


def api(**params):
    client = session()
    client.headers["Authorization"] = "Bearer " + access_token(**params)
    return client


def initial():
    anonymous = session()
    for path in [
        "/api/v1/fleets",
        "/livez",
        "/readyz",
        "/assets/missing.js",
        "/unknown",
    ]:
        expect(anonymous, "GET", path, 401)
    expect(anonymous, "GET", "/", 302)
    expect(
        anonymous,
        "GET",
        "/api/v1/fleets",
        401,
        headers={"X-Actor": "operator", "X-Backend-Token": "obsolete"},
    )

    operator = api()
    expect(operator, "GET", "/readyz", 200)
    expect(operator, "GET", "/api/v1/fleets", 200)
    for params in [{"audience": "wrong"}, {"typ": "JWT"}, {"expired": "true"}]:
        expect(api(**params), "GET", "/api/v1/fleets", 401)
    expect(api(subject="unmapped"), "GET", "/api/v1/fleets", 403)
    expect(api(scope="auth.read"), "GET", "/api/v1/fleets", 403)
    expect(api(subject="reader"), "PUT", PROFILE, 403, json={})

    body = {
        "kind": "github_app",
        "schema_version": 2,
        "app_id": APP_ID,
        "private_key": PRIVATE_KEY,
        "target_policy": [{"kind": "organization", "owner": "example-org"}],
    }
    expect(operator, "PUT", PROFILE, 428, json=body)
    headers = {"If-None-Match": "*", "Idempotency-Key": "nixos-profile-create"}
    accepted = expect(operator, "PUT", PROFILE, 202, json=body, headers=headers)
    replay = expect(operator, "PUT", PROFILE, 202, json=body, headers=headers)
    assert accepted.json() == replay.json(), (
        "idempotent replay changed the accepted result"
    )
    original = expect(operator, "GET", PROFILE, 200).json()

    browser = session()
    logged_in = browser.get(BASE + "/", timeout=15)
    assert logged_in.status_code == 200
    assert any("/authorize?" in response.url for response in logged_in.history)
    assert any("/auth/oidc/callback?" in response.url for response in logged_in.history)
    assets = re.findall(r'(?:src|href)="(/assets/[^" ]+)"', logged_in.text)
    assert assets, "release binary did not embed Vite assets"
    for asset in assets:
        expect(browser, "GET", asset, 200)
    identity = expect(browser, "GET", "/api/v1/session", 200)
    csrf = identity.headers["X-CSRF-Token"]
    expect(browser, "PUT", PROFILE, 403, json={})
    expect(
        browser,
        "PUT",
        PROFILE,
        422,
        json={},
        headers={
            "Origin": BASE,
            "X-CSRF-Token": csrf,
            "If-None-Match": "*",
        },
    )
    expect(
        browser,
        "POST",
        "/auth/oidc/logout",
        204,
        headers={"Origin": BASE, "X-CSRF-Token": csrf},
    )
    expect(browser, "GET", "/api/v1/session", 401)

    # Keep a second browser session to prove restart invalidates it, while
    # persisted Profile identity and revisions survive in SQLite.
    browser.get(BASE + "/", timeout=15).raise_for_status()
    expect(browser, "GET", "/api/v1/session", 200)
    os.umask(0o077)
    STATE.write_text(
        json.dumps({"profile": original, "cookies": browser.cookies.get_dict()})
    )
    print(
        "HTTPS, OIDC/PKCE, JWT denial, scopes, embedded UI, CSRF and durable writes passed"
    )


def after_restart():
    saved = json.loads(STATE.read_text())
    current = expect(api(), "GET", PROFILE, 200).json()
    # Validation may asynchronously move status; immutable resource identity
    # and desired revision must remain exactly the same.
    for key in ["key", "incarnation", "desiredRevision"]:
        assert key in current and current[key] == saved["profile"][key], key
    old_browser = session()
    old_browser.cookies.update(saved["cookies"])
    expect(old_browser, "GET", "/api/v1/session", 401)
    expect(api(), "GET", "/readyz", 200)
    print(
        "persisted Profile identity and revision survived; old browser session rejected"
    )


if __name__ == "__main__":
    {"initial": initial, "after-restart": after_restart}[sys.argv[1]]()
