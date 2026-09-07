"""Test-only HTTPS OIDC issuer; never installed in the Shaula package/module."""

import base64
import hashlib
import hmac
import json
import secrets
import ssl
import sys
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import parse_qs, urlencode, urlsplit

import jwt
from cryptography.hazmat.primitives.asymmetric import rsa

ISSUER = "https://idp.test:8443"
CALLBACK = "https://shaula.test/auth/oidc/callback"
CLIENT_SECRET = "nixos-test-only-oidc-secret"
SCOPES = "fleet.read template.read auth.read auth.write"
KEY = rsa.generate_private_key(public_exponent=65537, key_size=2048)
CODES = {}


def b64(data):
    return base64.urlsafe_b64encode(data).decode().rstrip("=")


def token(
    subject="operator", scope=SCOPES, audience="shaula-api", typ="at+jwt", **extra
):
    now = int(time.time())
    claims = {
        "iss": ISSUER,
        "sub": subject,
        "aud": audience,
        "iat": now,
        "exp": now + 3600,
        "client_id": "test-automation",
        "jti": secrets.token_hex(16),
        "scope": scope,
        "name": "NixOS test operator",
    }
    claims.update(extra)
    return jwt.encode(
        claims, KEY, algorithm="RS256", headers={"typ": typ, "kid": "vm-key"}
    )


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        # OIDC callback queries, tokens and Basic credentials are not log data.
        pass

    def respond(self, status, body):
        payload = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        url = urlsplit(self.path)
        params = {key: values[0] for key, values in parse_qs(url.query).items()}
        if url.path == "/.well-known/openid-configuration":
            self.respond(
                200,
                {
                    "issuer": ISSUER,
                    "authorization_endpoint": ISSUER + "/authorize",
                    "token_endpoint": ISSUER + "/token",
                    "jwks_uri": ISSUER + "/jwks",
                    "response_types_supported": ["code"],
                    "subject_types_supported": ["public"],
                    "id_token_signing_alg_values_supported": ["RS256"],
                    "token_endpoint_auth_methods_supported": ["client_secret_basic"],
                    "code_challenge_methods_supported": ["S256"],
                    "scopes_supported": ["openid", "profile"],
                },
            )
        elif url.path == "/jwks":
            numbers = KEY.public_key().public_numbers()
            self.respond(
                200,
                {
                    "keys": [
                        {
                            "kty": "RSA",
                            "kid": "vm-key",
                            "alg": "RS256",
                            "use": "sig",
                            "n": b64(numbers.n.to_bytes(256, "big")),
                            "e": b64(numbers.e.to_bytes(3, "big")),
                        }
                    ]
                },
            )
        elif url.path == "/authorize":
            if (
                params.get("client_id") != "shaula-web"
                or params.get("redirect_uri") != CALLBACK
                or params.get("response_type") != "code"
                or params.get("code_challenge_method") != "S256"
                or not all(
                    params.get(key) for key in ["state", "nonce", "code_challenge"]
                )
            ):
                self.respond(400, {"error": "invalid_request"})
                return
            code = secrets.token_urlsafe(32)
            CODES[code] = params
            self.send_response(302)
            self.send_header(
                "Location",
                CALLBACK + "?" + urlencode({"state": params["state"], "code": code}),
            )
            self.end_headers()
        elif url.path == "/test/access-token":
            # This deliberately unauthenticated mint exists only in isolated VMs.
            options = {
                key: params[key]
                for key in ["subject", "scope", "audience", "typ"]
                if key in params
            }
            if params.get("expired") == "true":
                options["exp"] = int(time.time()) - 3600
            self.respond(200, {"access_token": token(**options)})
        else:
            self.respond(404, {"error": "not_found"})

    def do_POST(self):
        if self.path != "/token":
            self.respond(404, {"error": "not_found"})
            return
        expected = (
            "Basic "
            + base64.b64encode(("shaula-web:" + CLIENT_SECRET).encode()).decode()
        )
        if not hmac.compare_digest(self.headers.get("Authorization", ""), expected):
            self.respond(401, {"error": "invalid_client"})
            return
        length = int(self.headers.get("Content-Length", "0"))
        if not 0 < length <= 16384:
            self.respond(400, {"error": "invalid_request"})
            return
        params = {
            key: values[0]
            for key, values in parse_qs(self.rfile.read(length).decode()).items()
        }
        code = CODES.pop(params.get("code"), None)
        challenge = b64(
            hashlib.sha256(params.get("code_verifier", "").encode()).digest()
        )
        if (
            code is None
            or params.get("grant_type") != "authorization_code"
            or params.get("redirect_uri") != CALLBACK
            or not hmac.compare_digest(challenge, code["code_challenge"])
        ):
            self.respond(400, {"error": "invalid_grant"})
            return
        self.respond(
            200,
            {
                "access_token": token(),
                "token_type": "Bearer",
                "expires_in": 3600,
                "id_token": token(
                    audience="shaula-web", typ="JWT", nonce=code["nonce"]
                ),
            },
        )


if __name__ == "__main__":
    tls = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    tls.load_cert_chain(sys.argv[1], sys.argv[2])
    server = HTTPServer(("0.0.0.0", 8443), Handler)
    server.socket = tls.wrap_socket(server.socket, server_side=True)
    server.serve_forever()
